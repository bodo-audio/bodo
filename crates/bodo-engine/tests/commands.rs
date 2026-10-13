//! Every command against a fixture project: its documented example runs, keeps the project valid, undoes exactly, and
//! its result is pinned by a snapshot. Plus random edit sequences that must keep every invariant and undo cleanly.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use bodo_engine::{Call, Session};
use common::{run, session};
use proptest::prelude::*;
use serde_json::{Value, json};

/// The beat from the exit criterion, plus a route, so every example has something to act on.
fn fixture() -> Session {
    let mut session = session();
    let script: Value = serde_json::from_str(include_str!("fixtures/beat.json")).unwrap();
    let calls: Vec<Call> = serde_json::from_value(script["calls"].clone()).unwrap();
    for call in calls {
        session.invoke(&call).unwrap();
    }
    run(&mut session, "mixer.add_route", json!({ "from": "insert:1", "to": "insert:5", "level": 0.5 }));
    session
}

/// Commands that read or write files outside the test's control; the beat, MIDI, recording and library tests cover
/// them.
const FILE_COMMANDS: [&str; 17] = [
    "project.open",
    "project.save",
    "project.save_as",
    "project.save_template",
    "project.import_midi",
    "pattern.export_midi",
    "project.export_midi",
    "recording.add_takes",
    "clip.consolidate",
    "library.list",
    "library.tag",
    "preset.save_channel",
    "preset.load_channel",
    "preset.save_mixer",
    "preset.load_mixer",
    "preset.save_score",
    "preset.load_score",
];

/// Commands whose examples act on an audio clip channel named Crash, which the fixture gets first for them.
const NEED_AUDIO: [&str; 2] = ["clip.place_audio", "clip.set_stretch"];

#[test]
fn every_example_runs_validly_and_undoes_exactly() {
    let ids: Vec<&'static str> = fixture().registry().iter().map(|c| c.id).collect();
    for id in ids.into_iter().filter(|id| !FILE_COMMANDS.contains(id)) {
        let mut session = fixture();
        if NEED_AUDIO.contains(&id) {
            run(&mut session, "channel.add_audio", json!({ "sample": "factory:Packs/Bodo Drums/Crash.wav", "name": "Crash" }));
        }
        // Automation edits act on the clip and the pattern lane the creating examples make.
        for setup in ["automation.create_for", "automation.set_lane"] {
            if id.starts_with("automation.") && id != setup {
                let example = session.registry().get(setup).unwrap().example.clone().unwrap();
                session.execute(setup, example).unwrap();
            }
        }
        // Group edits act on the groups the grouping examples make.
        for (setup, users) in [
            ("channel.set_group", &["channel.rename_group"][..]),
            ("playlist.group_tracks", &["playlist.rename_track_group", "playlist.set_group_props", "playlist.ungroup_tracks"][..]),
        ] {
            if users.contains(&id) {
                let example = session.registry().get(setup).unwrap().example.clone().unwrap();
                session.execute(setup, example).unwrap();
            }
        }
        let spec = session.registry().get(id).unwrap();
        let example = spec.example.clone().unwrap_or_else(|| panic!("{id} has no example"));
        let undoable = spec.effects.undoable;
        let acts_on_session = spec.is_session_command();
        let before = session.project().clone();
        let outcome = session.execute(id, example.clone());
        session.project().validate().unwrap_or_else(|e| panic!("{id} broke the project: {e}"));
        let snapshot = match &outcome {
            Ok(output) => json!({ "params": example, "result": output.result, "undo": output.undo }),
            Err(error) => json!({ "params": example, "error": error }),
        };
        insta::assert_json_snapshot!(id.replace('.', "__"), snapshot);
        if let Ok(output) = outcome
            && output.changed
            && !acts_on_session
        {
            assert!(undoable, "{id} changed the project without being undoable");
            session.undo().unwrap();
            assert_eq!(**session.project(), *before, "undoing {id} restores the project exactly");
        }
    }
}

#[test]
fn a_gesture_is_one_undo_step() {
    let mut session = fixture();
    let steps = session.undo_labels().count();
    for volume in [0.9, 0.7, 0.5] {
        session.invoke(&Call::new("channel.set_volume", json!({ "channel": "Kick", "volume": volume })).in_gesture(7)).unwrap();
    }
    assert_eq!(session.undo_labels().count(), steps + 1);
    assert_eq!(session.undo_labels().last(), Some("Set Kick volume"));
    session.invoke(&Call::new("channel.set_volume", json!({ "channel": "Kick", "volume": 0.3 })).in_gesture(8)).unwrap();
    assert_eq!(session.undo_labels().count(), steps + 2, "a new gesture is a new step");
    session.undo();
    session.undo();
    assert_eq!(session.project().channels.values().find(|c| c.name == "Kick").map(|c| c.volume), Some(1.0));
}

#[test]
fn a_batch_is_all_or_nothing() {
    let mut session = fixture();
    let before = session.project().clone();
    let calls = [
        Call::new("pattern.new", json!({ "name": "Bridge" })),
        Call::new("pattern.set_steps", json!({ "channel": "Kick", "steps": "x..." })),
        Call::new("channel.set_volume", json!({ "channel": "Nobody", "volume": 0.5 })),
    ];
    let error = session.batch(&calls, None).unwrap_err();
    assert_eq!((error.index, error.error.code), (2, bodo_engine::ErrorCode::NotFound));
    assert!(std::sync::Arc::ptr_eq(session.project(), &before));
    let output = session.batch(&calls[..2], Some("Bridge")).unwrap();
    assert_eq!(output.undo.as_deref(), Some("Bridge"));
    session.undo();
    assert_eq!(**session.project(), *before);
    let refused = session.batch(&[Call::new("edit.undo", json!({}))], None).unwrap_err();
    assert_eq!(refused.error.code, bodo_engine::ErrorCode::NotInBatch);
}

#[test]
fn routes_that_close_a_loop_are_refused_with_the_path() {
    let mut session = fixture();
    run(&mut session, "mixer.add_route", json!({ "from": "insert:5", "to": "insert:6" }));
    let error = session.execute("mixer.add_route", json!({ "from": "insert:6", "to": "insert:1" })).unwrap_err();
    assert_eq!(error.code, bodo_engine::ErrorCode::Rejected);
    assert!(error.message.contains("insert 1") && error.message.contains("insert 6"), "{error}");
}

/// One random edit.
#[derive(Clone, Debug)]
enum Op {
    Toggle { channel: usize, step: u32 },
    Steps { channel: usize, pattern: String },
    NewPattern,
    SelectPattern(i32),
    DeletePattern,
    Place { track: usize, bar: u32, bars: u32 },
    MoveSelectedClip { bars: i64, track: usize },
    AddNotes { key: u8, step: u32 },
    MoveSelectedNotes { ticks: i64, keys: i32 },
    Volume { channel: usize, volume: f32 },
    AddChannel,
    RemoveChannel(usize),
    Route { from: usize, to: usize },
    Undo,
    Redo,
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (0..6usize, 1..=32u32).prop_map(|(channel, step)| Op::Toggle { channel, step }),
        (0..6usize, "[x.]{1,16}").prop_map(|(channel, pattern)| Op::Steps { channel, pattern }),
        Just(Op::NewPattern),
        (-2..=2i32).prop_map(Op::SelectPattern),
        Just(Op::DeletePattern),
        (1..=17usize, 1..=16u32, 1..=4u32).prop_map(|(track, bar, bars)| Op::Place { track, bar, bars }),
        (-3..=3i64, 1..=4usize).prop_map(|(bars, track)| Op::MoveSelectedClip { bars, track }),
        (24..=96u8, 1..=16u32).prop_map(|(key, step)| Op::AddNotes { key, step }),
        (-480..=480i64, -5..=5i32).prop_map(|(ticks, keys)| Op::MoveSelectedNotes { ticks, keys }),
        (0..6usize, 0.0..=2.0f32).prop_map(|(channel, volume)| Op::Volume { channel, volume }),
        Just(Op::AddChannel),
        (0..6usize).prop_map(Op::RemoveChannel),
        (1..=17usize, 1..=17usize).prop_map(|(from, to)| Op::Route { from, to }),
        Just(Op::Undo),
        Just(Op::Redo),
    ]
}

fn apply(session: &mut Session, op: &Op) {
    let call = |id: &str, params: Value| Call::new(id, params);
    let result = match op {
        Op::Toggle { channel, step } => session.invoke(&call("pattern.toggle_step", json!({ "channel": channel + 1, "step": step }))),
        Op::Steps { channel, pattern } => session.invoke(&call("pattern.set_steps", json!({ "channel": channel + 1, "steps": pattern }))),
        Op::NewPattern => session.invoke(&call("pattern.new", json!({}))),
        Op::SelectPattern(offset) => session.invoke(&call("ui.select_pattern", json!({ "offset": offset }))),
        Op::DeletePattern => session.invoke(&call("pattern.delete", json!({}))),
        Op::Place { track, bar, bars } => {
            session.invoke(&call("clip.place", json!({ "track": track, "start": bar.to_string(), "length": format!("{bars} bars") })))
        }
        Op::MoveSelectedClip { bars, track } => session.invoke(&call("clip.move", json!({ "by": bars * 3840, "track": track }))),
        Op::AddNotes { key, step } => session.invoke(&call("note.add_many", json!({ "notes": [{ "key": key, "start": (step - 1) * 240 }] }))),
        Op::MoveSelectedNotes { ticks, keys } => session.invoke(&call("note.move", json!({ "ticks": ticks, "keys": keys }))),
        Op::Volume { channel, volume } => session.invoke(&call("channel.set_volume", json!({ "channel": channel + 1, "volume": volume }))),
        Op::AddChannel => session.invoke(&call("channel.add", json!({ "sample": "factory:Packs/Bodo Drums/Rim.wav" }))),
        Op::RemoveChannel(channel) => session.invoke(&call("channel.remove", json!({ "channels": [channel + 1] }))),
        Op::Route { from, to } => session.invoke(&call("mixer.add_route", json!({ "from": format!("insert:{from}"), "to": format!("insert:{to}") }))),
        Op::Undo => {
            session.undo();
            return;
        }
        Op::Redo => {
            session.redo();
            return;
        }
    };
    // Refusals are fine (a missing channel, nothing selected, a loop); breaking an invariant never is.
    if let Err(error) = result {
        assert!(!error.message.contains("would break the project"), "{op:?} broke an invariant: {error}");
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, failure_persistence: None, ..ProptestConfig::default() })]

    #[test]
    fn random_edits_keep_the_project_valid_and_undo_cleanly(ops in prop::collection::vec(op(), 1..40)) {
        let mut session = session();
        let start = session.project().clone();
        for op in &ops {
            apply(&mut session, op);
            prop_assert!(session.project().validate().is_ok(), "{op:?}: {:?}", session.project().validate());
        }
        // Redo whatever the ops left undone, so the history runs from the start to this tip.
        while session.redo().is_some() {}
        let tip = session.project().clone();
        while session.undo().is_some() {}
        prop_assert_eq!(&**session.project(), &*start);
        while session.redo().is_some() {}
        prop_assert_eq!(&**session.project(), &*tip);
    }
}
