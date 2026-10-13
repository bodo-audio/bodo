//! The playlist's editing commands: slice, slip, make unique, time insert and delete, split by channel, markers,
//! copy and paste, and toggled selection.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use bodo_engine::{Call, Session};
use bodo_kit_core::Tick;
use bodo_model::playlist::{ClipOffset, ClipSource};
use bodo_model::{Clip, TrackId};
use common::{run, session};
use serde_json::{Value, json};

fn beat() -> Session {
    let mut session = session();
    let script: Value = serde_json::from_str(include_str!("fixtures/beat.json")).unwrap();
    let calls: Vec<Call> = serde_json::from_value(script["calls"].clone()).unwrap();
    for call in calls {
        session.invoke(&call).unwrap();
    }
    session
}

fn track(session: &Session, n: usize) -> (TrackId, Vec<Clip>) {
    let id = session.project().track_order[n - 1];
    (id, session.project().tracks[&id].clips.iter().cloned().collect())
}

fn starts(clips: &[Clip]) -> Vec<i64> {
    clips.iter().map(|c| c.start.0).collect()
}

#[test]
fn slicing_cuts_crossing_clips_and_both_halves_play_the_same_pattern() {
    let mut session = beat();
    let before = session.project().clone();
    // The Verse clip runs from bar 3 for 4 bars; cut it at bar 5.
    run(&mut session, "clip.slice", json!({ "at": "5:1:0", "tracks": [1] }));
    let (_, clips) = track(&session, 1);
    let verse: Vec<&Clip> = clips.iter().filter(|c| c.start.0 >= 7680 && c.start.0 < 23_040).collect();
    assert_eq!(verse.len(), 2);
    assert_eq!((verse[0].start.0, verse[0].len.0), (7680, 7680));
    assert_eq!((verse[1].start.0, verse[1].len.0, verse[1].offset), (15_360, 7680, ClipOffset::Ticks(Tick(7680))));
    assert_eq!(verse[0].source, verse[1].source, "pattern clips are not copied");
    session.undo();
    assert_eq!(**session.project(), *before);
    let error = session.execute("clip.slice", json!({ "at": "100:1:0", "tracks": [1] })).unwrap_err();
    assert!(error.message.contains("no clip crosses"), "{error}");
}

#[test]
fn slipping_moves_content_not_the_clip() {
    let mut session = beat();
    let (_, clips) = track(&session, 1);
    let intro = clips[0].id;
    run(&mut session, "clip.slip", json!({ "clips": [intro], "by": 240 }));
    let (_, after) = track(&session, 1);
    let clip = after.iter().find(|c| c.id == intro).unwrap();
    assert_eq!((clip.start, clip.len, clip.offset), (clips[0].start, clips[0].len, ClipOffset::Ticks(Tick(240))));
    // Pattern content wraps around the pattern's length (one bar).
    run(&mut session, "clip.slip", json!({ "clips": [intro], "by": -480 }));
    let (_, after) = track(&session, 1);
    assert_eq!(after.iter().find(|c| c.id == intro).unwrap().offset, ClipOffset::Ticks(Tick(3840 - 240)));
}

#[test]
fn making_a_clip_unique_copies_its_pattern_for_that_clip_only() {
    let mut session = beat();
    // Track 2 holds a second Fill clip; make it unique.
    let (_, clips) = track(&session, 2);
    let fill = clips[0].clone();
    let result = run(&mut session, "clip.make_unique", json!({ "clip": fill.id }));
    let (_, after) = track(&session, 2);
    let ClipSource::Pattern(copy) = after[0].source else { panic!("a pattern clip") };
    let ClipSource::Pattern(original) = fill.source else { panic!("a pattern clip") };
    assert_ne!(copy, original);
    assert_eq!(result["name"], "Fill 2");
    let project = session.project();
    assert_eq!(project.patterns[&copy].notes, project.patterns[&original].notes);
    let (_, first) = track(&session, 1);
    assert!(first.iter().any(|c| c.source == ClipSource::Pattern(original)), "the other Fill clip still plays the original");
}

#[test]
fn inserting_and_deleting_time_moves_the_song() {
    let mut session = beat();
    run(&mut session, "project.add_marker", json!({ "at": "8:1:0", "name": "End" }));
    let (_, before) = track(&session, 1);
    assert_eq!(starts(&before), [0, 7680, 23_040, 26_880]);
    // One bar at bar 2 cuts the two-bar Intro and pushes everything after it.
    run(&mut session, "playlist.insert_time", json!({ "at": "2:1:0", "length": "1 bar" }));
    let (_, inserted) = track(&session, 1);
    assert_eq!(starts(&inserted), [0, 7680, 11_520, 26_880, 30_720]);
    assert_eq!(session.project().markers[0].at, Tick(30_720));
    session.undo();
    // Deleting bar 2 drops the Intro's second bar and pulls the rest in.
    run(&mut session, "playlist.delete_time", json!({ "at": "2:1:0", "length": "1 bar" }));
    let (_, deleted) = track(&session, 1);
    assert_eq!(starts(&deleted), [0, 3840, 19_200, 23_040]);
    assert_eq!(deleted[0].len, Tick(3840));
    assert_eq!(session.project().markers[0].at, Tick(23_040));
    assert!(session.project().validate().is_ok());
}

#[test]
fn splitting_a_pattern_by_channel_stacks_one_clip_per_channel() {
    let mut session = beat();
    let result = run(&mut session, "pattern.split_by_channel", json!({ "pattern": "Verse" }));
    let parts = result["patterns"].as_array().unwrap().len();
    assert_eq!(parts, 4, "Kick, Clap, HiHat and 808");
    let project = session.project().clone();
    let names: Vec<&str> = project.patterns.values().map(|p| p.name.as_str()).collect();
    assert!(names.contains(&"Verse - Kick") && names.contains(&"Verse - 808"), "{names:?}");
    for row in 1..=4 {
        let (_, clips) = track(&session, row);
        assert!(clips.iter().any(|c| c.start == Tick(7680)), "track {row} has a part at bar 3");
    }
}

#[test]
fn markers_stay_in_time_order_and_answer_to_names() {
    let mut session = beat();
    run(&mut session, "project.add_marker", json!({ "at": "5:1:0", "name": "Drop" }));
    run(&mut session, "project.add_marker", json!({ "at": "1:1:0", "name": "Start" }));
    let names: Vec<String> = session.project().markers.iter().map(|m| m.name.clone()).collect();
    assert_eq!(names, ["Start", "Drop"]);
    run(&mut session, "project.rename_marker", json!({ "marker": "drop", "name": "Chorus" }));
    run(&mut session, "project.remove_marker", json!({ "marker": 1 }));
    let names: Vec<String> = session.project().markers.iter().map(|m| m.name.clone()).collect();
    assert_eq!(names, ["Chorus"]);
}

#[test]
fn clips_and_notes_copy_and_paste() {
    let mut session = beat();
    let (_, clips) = track(&session, 1);
    run(&mut session, "edit.select", json!({ "clips": [clips[2].id, clips[3].id] }));
    run(&mut session, "edit.copy", json!({}));
    let pasted = run(&mut session, "edit.paste", json!({}));
    assert_eq!(pasted["pasted"], 2);
    let (_, after) = track(&session, 1);
    // Fill and Outro (bars 7 and 8) land right after themselves, at bars 9 and 10.
    assert_eq!(starts(&after), [0, 7680, 23_040, 26_880, 30_720, 34_560]);
    let selected: Vec<_> = session.editor().clips.iter().copied().collect();
    assert_eq!(selected.len(), 2, "the pasted clips are selected");

    run(&mut session, "ui.select_pattern", json!({ "pattern": "Verse" }));
    run(&mut session, "edit.select_all", json!({ "what": "notes" }));
    run(&mut session, "edit.cut", json!({ "what": "notes" }));
    let verse = session.project().patterns.values().find(|p| p.name == "Verse").unwrap().clone();
    assert!(verse.notes.is_empty(), "cut removes the notes");
    run(&mut session, "edit.paste", json!({ "at": 0 }));
    let verse = session.project().patterns.values().find(|p| p.name == "Verse").unwrap().clone();
    assert_eq!(verse.notes.values().map(|s| s.len()).sum::<usize>(), 17, "and paste puts them back");
}

#[test]
fn shift_selection_toggles_and_never_enters_the_history() {
    let mut session = beat();
    let (_, clips) = track(&session, 1);
    let steps = session.undo_labels().count();
    run(&mut session, "edit.select", json!({ "clips": [clips[0].id, clips[1].id] }));
    run(&mut session, "edit.select", json!({ "clips": [clips[1].id, clips[2].id], "toggle": true }));
    let selected: Vec<_> = session.editor().clips.iter().copied().collect();
    let mut expected = vec![clips[0].id, clips[2].id];
    expected.sort();
    assert_eq!(selected, expected);
    assert_eq!(session.undo_labels().count(), steps);
}
