//! Channel rack groups, playlist track groups and solo, time selections and consolidating clips to audio
//! (project.md sections 10.2 and 10.3).

#![allow(clippy::unwrap_used, clippy::panic, reason = "tests")]

mod common;

use bodo_engine::{Call, Session};
use bodo_kit_core::Tick;
use bodo_model::playlist::ClipSource;
use common::{run, session};
use proptest::prelude::*;
use serde_json::{Value, json};

/// The M2 beat: Drums on track 1 (Intro, Verse, Fill, Outro) and a Fill on track 2 at bar 6.
fn beat() -> Session {
    let mut session = session();
    let script: Value = serde_json::from_str(include_str!("fixtures/beat.json")).unwrap();
    let calls: Vec<Call> = serde_json::from_value(script["calls"].clone()).unwrap();
    for call in calls {
        session.invoke(&call).unwrap();
    }
    session
}

fn group_of(session: &Session, channel: &str) -> Option<String> {
    session.project().channels.values().find(|c| c.name == channel).unwrap().group.clone()
}

fn track_names(session: &Session) -> Vec<String> {
    let project = session.project();
    project.track_order.iter().map(|t| project.tracks[t].name.clone()).collect()
}

/// `(start, length)` of every clip on track `index` (from 0).
fn clips_on(session: &Session, index: usize) -> Vec<(i64, i64)> {
    let project = session.project();
    project.tracks[&project.track_order[index]].clips.iter().map(|c| (c.start.0, c.len.0)).collect()
}

#[test]
fn channels_join_rename_and_leave_groups() {
    let mut s = beat();
    run(&mut s, "channel.set_group", json!({ "channels": ["Kick", "Snare"], "group": "Drums" }));
    assert_eq!((group_of(&s, "Kick"), group_of(&s, "Clap")), (Some(String::from("Drums")), None));
    run(&mut s, "channel.rename_group", json!({ "group": "Drums", "name": "Beat" }));
    assert_eq!(group_of(&s, "Snare").as_deref(), Some("Beat"));
    run(&mut s, "channel.set_group", json!({ "channels": ["Clap"], "group": "Hits" }));
    let clash = s.execute("channel.rename_group", json!({ "group": "Hits", "name": "Beat" })).unwrap_err();
    assert!(clash.message.contains("already"), "{clash}");
    run(&mut s, "channel.set_group", json!({ "channels": ["Kick"], "group": null }));
    assert_eq!(group_of(&s, "Kick"), None);
    let duplicate = run(&mut s, "channel.duplicate", json!({ "channel": "Snare" }));
    assert_eq!(group_of(&s, duplicate["name"].as_str().unwrap()).as_deref(), Some("Beat"), "a copy stays in its group");
}

#[test]
fn a_group_swing_drag_is_one_undo_step() {
    let mut s = beat();
    let before = s.undo_labels().count();
    for swing in [0.1, 0.3, 0.5] {
        s.invoke(&Call::new("channel.set_swing", json!({ "channels": ["Kick", "HiHat"], "swing": swing })).in_gesture(4)).unwrap();
    }
    let project = s.project();
    assert!(project.channels.values().filter(|c| c.name == "Kick" || c.name == "HiHat").all(|c| c.swing == 0.5));
    assert!(project.channels.values().filter(|c| c.name == "Clap").all(|c| c.swing == 0.0));
    assert_eq!(s.undo_labels().count(), before + 1);
}

#[test]
fn track_groups_gather_their_tracks_and_mute_or_solo_together() {
    let mut s = beat();
    run(&mut s, "playlist.group_tracks", json!({ "tracks": [1, 3], "name": "Drums" }));
    assert_eq!(track_names(&s)[..4], ["Drums", "Track 3", "Track 2", "Track 4"], "the group's tracks sit together");
    run(&mut s, "playlist.group_tracks", json!({ "tracks": [5], "name": "Drums" }));
    assert_eq!(track_names(&s)[..4], ["Drums", "Track 3", "Track 5", "Track 2"], "a joining track goes after the group's last");
    run(&mut s, "playlist.set_group_props", json!({ "group": "Drums", "solo": true }));
    let soloed: Vec<String> = s.project().tracks.values().filter(|t| t.solo).map(|t| t.name.clone()).collect();
    assert_eq!(soloed.len(), 3, "{soloed:?}");
    run(&mut s, "playlist.rename_track_group", json!({ "group": "Drums", "name": "Beat" }));
    assert!(s.project().tracks.values().filter(|t| t.solo).all(|t| t.group.as_deref() == Some("Beat")));
    run(&mut s, "playlist.ungroup_tracks", json!({ "tracks": ["Track 3"] }));
    assert_eq!(s.project().tracks.values().filter(|t| t.group.is_some()).count(), 2);
    run(&mut s, "playlist.ungroup_tracks", json!({ "group": "Beat" }));
    assert!(s.project().tracks.values().all(|t| t.group.is_none()));
    let missing = s.execute("playlist.set_group_props", json!({ "group": "Beat", "muted": true })).unwrap_err();
    assert!(missing.message.contains("Beat"), "{missing}");
}

#[test]
fn a_time_selection_copies_and_cuts_the_parts_of_every_track() {
    let mut s = beat();
    // Bars 2 to 7: the second half of the Intro, the whole Verse on track 1, the Fill on track 2.
    let range = json!({ "start": "2:1:0", "end": "7:1:0" });
    let copied = run(&mut s, "edit.copy", range.clone());
    assert_eq!(copied["copied"], 3);
    run(&mut s, "edit.paste", json!({ "at": "17:1:0" }));
    let pasted: Vec<(i64, i64)> = clips_on(&s, 0).into_iter().filter(|(start, _)| *start >= 61_440).collect();
    assert_eq!(pasted, [(61_440, 3840), (65_280, 15_360)], "the Intro's half, then the Verse, keeping their gap");
    assert_eq!(clips_on(&s, 1).last(), Some(&(76_800, 3840)), "track 2's Fill lands on track 2, 15 bars after where it was");
    let intro_half = s.project().tracks[&s.project().track_order[0]].clips.iter().find(|c| c.start == Tick(61_440)).unwrap().clone();
    assert_eq!(intro_half.offset, bodo_model::playlist::ClipOffset::Ticks(Tick(3840)), "the trimmed part plays from where it was cut");
    let mut cut = beat();
    run(&mut cut, "edit.cut", json!({ "start": "2:1:0", "end": "5:1:0" }));
    assert_eq!(clips_on(&cut, 0)[..2], [(0, 3840), (15_360, 7680)], "the middle is gone and nothing moved");
    let reversed = cut.execute("edit.copy", json!({ "start": "5:1:0", "end": "2:1:0" })).unwrap_err();
    assert!(reversed.message.contains("after it starts"), "{reversed}");
}

#[test]
fn consolidating_renders_clips_into_one_audio_clip_in_one_step() {
    let dir = common::temp_dir("consolidate");
    let mut s = beat();
    run(&mut s, "project.save_as", json!({ "path": dir.join("Song.bodo").display().to_string() }));
    let before = s.undo_labels().count();
    let intro = s.project().tracks[&s.project().track_order[0]].clips[0].id;
    let verse = s.project().tracks[&s.project().track_order[0]].clips[1].id;
    let result = run(&mut s, "clip.consolidate", json!({ "clips": [intro.to_string(), verse.to_string()], "name": "Intro and verse" }));
    let file = std::path::PathBuf::from(result["file"].as_str().unwrap());
    assert!(file.starts_with(bodo_io::project_folder(&dir.join("Song.bodo")).join("Rendered")), "{}", file.display());
    let audio = bodo_kit_audio::decode_file(&file).unwrap();
    assert_eq!(audio.channels[0].len(), 720_000, "six bars at 96 BPM: 24 beats of 0.625 s at 48 kHz");
    assert!(audio.channels[0].iter().any(|s| s.abs() > 0.01), "the drums are in it");
    assert_eq!(result["replaced"], 2);
    let first = clips_on(&s, 0);
    assert_eq!(first[0], (0, 23_040), "one clip where the two were");
    let source = s.project().tracks[&s.project().track_order[0]].clips[0].source;
    assert!(matches!(source, ClipSource::Audio(_)));
    assert_eq!(s.undo_labels().count(), before + 1);
    s.undo();
    assert_eq!(clips_on(&s, 0)[..2], [(0, 7680), (7680, 15_360)]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn note_colors_are_set_and_checked() {
    let mut s = beat();
    run(&mut s, "note.add_many", json!({ "channel": "808", "notes": [{ "key": 48, "start": 0 }] }));
    run(&mut s, "note.set_props", json!({ "color": 5 }));
    let pattern = &s.project().patterns[&s.editor().pattern.unwrap()];
    assert!(pattern.notes.values().flat_map(|seq| seq.notes().iter()).any(|n| n.key == 48 && n.color == 5));
    assert!(s.execute("note.set_props", json!({ "color": 16 })).is_err());
}

/// One random edit of groups, solo and time selections.
#[derive(Clone, Debug)]
enum Op {
    ChannelGroup { channel: usize, group: Option<u8> },
    TrackGroup { tracks: Vec<usize>, group: u8 },
    Ungroup(u8),
    GroupSolo { group: u8, on: bool },
    TrackSolo { track: usize, on: bool },
    Copy { start: u32, bars: u32 },
    Cut { start: u32, bars: u32 },
    Paste { bar: u32 },
    Undo,
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (1..=5usize, proptest::option::of(0..3u8)).prop_map(|(channel, group)| Op::ChannelGroup { channel, group }),
        (prop::collection::vec(1..=8usize, 1..4), 0..3u8).prop_map(|(tracks, group)| Op::TrackGroup { tracks, group }),
        (0..3u8).prop_map(Op::Ungroup),
        (0..3u8, any::<bool>()).prop_map(|(group, on)| Op::GroupSolo { group, on }),
        (1..=8usize, any::<bool>()).prop_map(|(track, on)| Op::TrackSolo { track, on }),
        (1..=8u32, 1..=4u32).prop_map(|(start, bars)| Op::Copy { start, bars }),
        (1..=8u32, 1..=4u32).prop_map(|(start, bars)| Op::Cut { start, bars }),
        (1..=12u32).prop_map(|bar| Op::Paste { bar }),
        Just(Op::Undo),
    ]
}

fn apply(s: &mut Session, op: &Op, depth: usize) {
    let result = match op {
        Op::ChannelGroup { channel, group } => {
            s.execute("channel.set_group", json!({ "channels": [channel], "group": group.map(|g| format!("Group {g}")) }))
        }
        Op::TrackGroup { tracks, group } => s.execute("playlist.group_tracks", json!({ "tracks": tracks, "name": format!("Group {group}") })),
        Op::Ungroup(group) => s.execute("playlist.ungroup_tracks", json!({ "group": format!("Group {group}") })),
        Op::GroupSolo { group, on } => s.execute("playlist.set_group_props", json!({ "group": format!("Group {group}"), "solo": on })),
        Op::TrackSolo { track, on } => s.execute("playlist.set_track_props", json!({ "track": track, "solo": on })),
        Op::Copy { start, bars } => s.execute("edit.copy", json!({ "start": format!("{start}:1:0"), "end": format!("{}:1:0", start + bars) })),
        Op::Cut { start, bars } => s.execute("edit.cut", json!({ "start": format!("{start}:1:0"), "end": format!("{}:1:0", start + bars) })),
        Op::Paste { bar } => s.execute("edit.paste", json!({ "at": format!("{bar}:1:0") })),
        Op::Undo => {
            if s.undo_labels().count() > depth {
                s.undo();
            }
            return;
        }
    };
    if let Err(error) = result {
        assert!(!error.message.contains("would break the project"), "{op:?} broke an invariant: {error}");
    }
}

/// Whether every track group's tracks sit next to each other.
fn groups_are_together(s: &Session) -> bool {
    let project = s.project();
    let groups: Vec<Option<String>> = project.track_order.iter().map(|t| project.tracks[t].group.clone()).collect();
    let mut seen: Vec<String> = Vec::new();
    for (i, group) in groups.iter().enumerate() {
        let Some(name) = group else { continue };
        if i > 0 && groups[i - 1].as_ref() == Some(name) {
            continue;
        }
        if seen.contains(name) {
            return false;
        }
        seen.push(name.clone());
    }
    true
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, failure_persistence: None, ..ProptestConfig::default() })]

    #[test]
    fn random_group_and_time_edits_keep_the_project_valid(ops in prop::collection::vec(op(), 1..24)) {
        let mut s = beat();
        let start = s.project().clone();
        let depth = s.undo_labels().count();
        for op in &ops {
            apply(&mut s, op, depth);
            prop_assert!(s.project().validate().is_ok(), "{op:?}: {:?}", s.project().validate());
        }
        // A group's tracks always sit together: taking a track out of a run leaves the run whole.
        prop_assert!(groups_are_together(&s), "groups split: {:?}", s.project().track_order);
        // Undoing every op's step gives back the beat as it was built.
        while s.undo_labels().count() > depth && s.undo().is_some() {}
        prop_assert_eq!(&**s.project(), &*start);
    }
}
