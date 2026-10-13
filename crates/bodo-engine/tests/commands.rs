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

/// Commands that read or write files outside the test's control; the beat, MIDI, recording, library and render tests
/// cover them.
const FILE_COMMANDS: [&str; 19] = [
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
    "render.export_mix",
    "render.export_stems",
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
        // Edits of a layer or a MIDI Out channel act on the one the adding example makes.
        for (edit, setup) in [("channel.set_layer", "channel.add_layer"), ("channel.set_midi_out", "channel.add_midi_out")] {
            if id == edit {
                let example = session.registry().get(setup).unwrap().example.clone().unwrap();
                session.execute(setup, example).unwrap();
            }
        }
        // Slot edits act on the effect fx.load's example loads; pasting needs a copied chain.
        let needs_effect = (id.starts_with("fx.") || id.starts_with("plugin.")) && !["fx.load", "plugin.list", "plugin.describe"].contains(&id);
        let setups: &[&str] = if id == "fx.paste_chain" {
            &["fx.load", "fx.copy_chain"]
        } else if needs_effect {
            &["fx.load"]
        } else {
            &[]
        };
        for setup in setups {
            let example = session.registry().get(setup).unwrap().example.clone().unwrap();
            session.execute(setup, example).unwrap();
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
    // The loop is in the error's JSON too, as references an agent can act on, from the new route's source around.
    let json = serde_json::to_value(&error).unwrap();
    assert_eq!(json["cycle"], json!(["insert:6", "insert:1", "insert:5"]), "{json}");
    // Sidechains count: a ducker on insert 1 listening to insert 6 would close the same loop.
    run(&mut session, "fx.load", json!({ "insert": 1, "slot": 1, "effect": "Ducker" }));
    let error = session.execute("fx.set_sidechain", json!({ "insert": 1, "slot": 1, "from": "insert:6" })).unwrap_err();
    assert_eq!(serde_json::to_value(&error).unwrap()["cycle"], json!(["insert:6", "insert:1", "insert:5"]));
}

/// One random edit.
#[derive(Clone, Debug)]
enum Op {
    Toggle {
        channel: usize,
        step: u32,
    },
    Steps {
        channel: usize,
        pattern: String,
    },
    NewPattern,
    SelectPattern(i32),
    DeletePattern,
    Place {
        track: usize,
        bar: u32,
        bars: u32,
    },
    MoveSelectedClip {
        bars: i64,
        track: usize,
    },
    AddNotes {
        key: u8,
        step: u32,
    },
    MoveSelectedNotes {
        ticks: i64,
        keys: i32,
    },
    Volume {
        channel: usize,
        volume: f32,
    },
    AddChannel,
    RemoveChannel(usize),
    Route {
        from: usize,
        to: usize,
    },
    Sampler {
        channel: usize,
        start: f32,
        end: f32,
        cutoff: f32,
        arp: bool,
    },
    AddLayer {
        children: Vec<usize>,
    },
    SetLayer {
        layer: usize,
        children: Vec<usize>,
    },
    AddMidiOut {
        midi_channel: u8,
    },
    /// A mixer or FX command with its params.
    Mixer(&'static str, Value),
    Undo,
    Redo,
}

/// Random mixer and FX edits: effects, slots, sidechains, solo, routing, latency, EQ, group moves and removals.
fn mixer_op() -> impl Strategy<Value = Op> {
    let insert = 0..=8usize;
    let slot = 1..=10usize;
    let effect = prop::sample::select(vec!["bodo.utility", "bodo.filter", "bodo.delay", "bodo.ducker", "bodo.latency-test"]);
    prop_oneof![
        (insert.clone(), slot.clone(), effect).prop_map(|(i, s, e)| Op::Mixer("fx.load", json!({ "insert": i, "slot": s, "effect": e }))),
        (insert.clone(), slot.clone()).prop_map(|(i, s)| Op::Mixer("fx.remove", json!({ "insert": i, "slot": s }))),
        (insert.clone(), slot.clone(), slot.clone()).prop_map(|(i, a, b)| Op::Mixer("fx.move", json!({ "insert": i, "from": a, "to": b }))),
        (insert.clone(), slot.clone(), slot.clone()).prop_map(|(i, a, b)| Op::Mixer("fx.swap", json!({ "insert": i, "a": a, "b": b }))),
        (insert.clone(), slot.clone(), insert.clone())
            .prop_map(|(i, s, f)| Op::Mixer("fx.set_sidechain", json!({ "insert": i, "slot": s, "from": format!("insert:{f}") }))),
        (insert.clone(), slot.clone(), 0..5u32, -100.0..20000.0f32)
            .prop_map(|(i, s, p, v)| Op::Mixer("plugin.set_param", json!({ "insert": i, "slot": s, "param": p, "value": v }))),
        (insert.clone(), slot.clone(), 0.0..=1.0f32).prop_map(|(i, s, m)| Op::Mixer("fx.set_mix", json!({ "insert": i, "slot": s, "mix": m }))),
        (insert.clone(), slot.clone()).prop_map(|(i, s)| Op::Mixer("fx.set_enabled", json!({ "insert": i, "slot": s }))),
        insert.clone().prop_map(|i| Op::Mixer("fx.copy_chain", json!({ "insert": i }))),
        (insert.clone(), insert.clone()).prop_map(|(a, b)| Op::Mixer("fx.paste_chain", json!({ "inserts": [a, b] }))),
        (insert.clone(), any::<bool>()).prop_map(|(i, x)| Op::Mixer("mixer.solo", json!({ "insert": i, "exclusive": x }))),
        (insert.clone(), insert.clone()).prop_map(|(a, b)| Op::Mixer("mixer.route_exclusive", json!({ "from": format!("insert:{a}"), "to": b }))),
        (insert.clone(), insert.clone()).prop_map(|(a, b)| Op::Mixer("mixer.remove_route", json!({ "from": format!("insert:{a}"), "to": b }))),
        (insert.clone(), insert.clone())
            .prop_map(|(a, b)| Op::Mixer("mixer.add_route", json!({ "from": format!("insert:{a}"), "to": b, "sidechain": true }))),
        insert.clone().prop_map(|i| Op::Mixer("mixer.set_to_master", json!({ "insert": i }))),
        (insert.clone(), -2000..=2000i64).prop_map(|(i, f)| Op::Mixer("mixer.set_latency_offset", json!({ "insert": i, "frames": f }))),
        (insert.clone(), -30.0..30.0f32).prop_map(|(i, g)| Op::Mixer("mixer.set_eq", json!({ "insert": i, "band": "mid", "gainDb": g }))),
        (insert.clone(), insert.clone(), -12.0..12.0f32).prop_map(|(a, b, d)| Op::Mixer("mixer.adjust", json!({ "inserts": [a, b], "volumeDb": d }))),
        (1..=8usize, 1..=8usize).prop_map(|(i, to)| Op::Mixer("mixer.reorder_insert", json!({ "insert": i, "to": to }))),
        (1..=8usize).prop_map(|i| Op::Mixer("mixer.remove_insert", json!({ "insert": i }))),
        Just(Op::Mixer("mixer.add_insert", json!({}))),
    ]
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
        (0..8usize, 0.0..1.0f32, 0.0..1.0f32, 20.0..20_000.0f32, any::<bool>()).prop_map(|(channel, start, end, cutoff, arp)| Op::Sampler {
            channel,
            start,
            end,
            cutoff,
            arp
        }),
        prop::collection::vec(0..8usize, 0..4).prop_map(|children| Op::AddLayer { children }),
        (0..8usize, prop::collection::vec(0..8usize, 0..4)).prop_map(|(layer, children)| Op::SetLayer { layer, children }),
        (0..=17u8).prop_map(|midi_channel| Op::AddMidiOut { midi_channel }),
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
        Op::Sampler { channel, start, end, cutoff, arp } => session.invoke(&call(
            "sampler.set",
            json!({ "channel": channel + 1, "start": start, "end": end, "filter": { "kind": "lowPass", "cutoff": cutoff }, "arp": { "enabled": arp } }),
        )),
        Op::AddLayer { children } => {
            let children: Vec<usize> = children.iter().map(|c| c + 1).collect();
            session.invoke(&call("channel.add_layer", json!({ "children": children })))
        }
        Op::SetLayer { layer, children } => {
            let children: Vec<usize> = children.iter().map(|c| c + 1).collect();
            session.invoke(&call("channel.set_layer", json!({ "channel": layer + 1, "children": children })))
        }
        Op::AddMidiOut { midi_channel } => session.invoke(&call("channel.add_midi_out", json!({ "port": "Synth", "midiChannel": midi_channel }))),
        Op::Mixer(id, params) => session.invoke(&call(id, params.clone())),
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

    #[test]
    fn random_mixer_edits_keep_the_project_valid_and_compilable(ops in prop::collection::vec(prop_oneof![3 => mixer_op(), 1 => op().prop_filter("the fixture's own steps stay", |o| !matches!(o, Op::Undo | Op::Redo))], 1..40)) {
        let mut session = fixture();
        let start = session.project().clone();
        let base = session.undo_labels().count();
        for op in &ops {
            apply(&mut session, op);
            prop_assert!(session.project().validate().is_ok(), "{op:?}: {:?}", session.project().validate());
            // Whatever the edits did, the mixer still compiles: no loops, and delay compensation has an answer.
            prop_assert!(bodo_mix::mix_latency(session.project(), false).is_ok(), "{op:?} left a mixer that doesn't compile");
        }
        while session.redo().is_some() {}
        let tip = session.project().clone();
        // Undo back to the fixture, not through the steps that built it.
        while session.undo_labels().count() > base && session.undo().is_some() {}
        prop_assert_eq!(&**session.project(), &*start);
        while session.redo().is_some() {}
        prop_assert_eq!(&**session.project(), &*tip);
    }
}
