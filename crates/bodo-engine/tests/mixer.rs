//! The mixer through its commands: effects in slots and their parameters, sidechains, chains copied between inserts,
//! solo, exclusive routes, group edits, track latency, and renders of the mix and of stems.

#![allow(clippy::unwrap_used, clippy::panic, reason = "test helpers outside #[test] functions")]

mod common;

use bodo_engine::{Event, Session, inspect};
use bodo_model::mixer::{RouteKind, RouteSource};
use bodo_model::{InsertId, MixerInsert};
use common::{run, session, temp_dir};
use serde_json::json;

/// Insert `n` (0 is the master).
fn insert(session: &Session, n: usize) -> MixerInsert {
    let mixer = &session.project().mixer;
    if n == 0 { (*mixer.master).clone() } else { (*mixer.inserts[&mixer.order[n - 1]]).clone() }
}

fn id(session: &Session, n: usize) -> InsertId {
    insert(session, n).id
}

#[test]
fn effects_load_with_parameters_by_name_and_clamp() {
    let mut session = session();
    let loaded = run(&mut session, "fx.load", json!({ "insert": 1, "effect": "Delay", "params": { "Feedback": 60, "mix": 120 } }));
    assert_eq!(loaded["slot"]["slot"], 1, "the first empty slot");
    assert_eq!(loaded["slot"]["effect"], "bodo.delay");
    let slot = &insert(&session, 1).slots[0];
    assert_eq!(slot.param(1), Some(60.0));
    assert_eq!(slot.param(2), Some(100.0), "Mix is clamped to 100 %");

    // The second load lands in the next empty slot; parameters go by name or id, in their own units.
    run(&mut session, "fx.load", json!({ "insert": 1, "effect": "bodo.filter" }));
    let set = run(&mut session, "plugin.set_param", json!({ "insert": 1, "slot": 2, "param": "cutoff", "value": 50000 }));
    assert_eq!(set["value"], 20000.0, "cutoff clamps to 20 kHz");
    run(&mut session, "plugin.set_param", json!({ "insert": 1, "slot": 2, "param": 0, "value": 1.4 }));
    assert_eq!(insert(&session, 1).slots[1].param(0), Some(1.0), "choices round to whole indexes");
    let params = run(&mut session, "plugin.get_params", json!({ "insert": 1, "slot": 2 }));
    assert_eq!(params["slot"]["params"][0]["text"], "High pass");

    let unknown = session.execute("plugin.set_param", json!({ "insert": 1, "slot": 2, "param": "Wobble", "value": 1 })).unwrap_err();
    assert!(!unknown.candidates.is_empty(), "an unknown parameter lists the real ones");
    assert!(session.execute("fx.load", json!({ "insert": 1, "effect": "Reverb 9000" })).is_err());
    assert!(session.execute("plugin.set_param", json!({ "insert": 1, "slot": 5, "param": 0, "value": 1 })).is_err(), "an empty slot");

    // Every effect instance gets its own id.
    let slots = insert(&session, 1).slots;
    assert_ne!(slots[0].plugin.as_ref().unwrap().instance, slots[1].plugin.as_ref().unwrap().instance);
    let listed = run(&mut session, "plugin.list", json!({}));
    let uids: Vec<&str> = listed["plugins"].as_array().unwrap().iter().map(|p| p["uid"].as_str().unwrap()).collect();
    assert_eq!(uids, ["bodo.utility", "bodo.filter", "bodo.delay", "bodo.ducker"], "the latency test effect stays hidden");
}

#[test]
fn slots_move_swap_and_switch_and_gestures_merge() {
    let mut session = session();
    for effect in ["Utility", "Filter", "Delay"] {
        run(&mut session, "fx.load", json!({ "insert": 2, "effect": effect }));
    }
    let uid = |session: &Session, slot: usize| insert(session, 2).slots[slot].plugin.as_ref().map(|p| p.uid.clone());
    run(&mut session, "fx.move", json!({ "insert": 2, "from": 1, "to": 3 }));
    assert_eq!(
        [uid(&session, 0), uid(&session, 1), uid(&session, 2)],
        [Some("bodo.filter".into()), Some("bodo.delay".into()), Some("bodo.utility".into())]
    );
    run(&mut session, "fx.swap", json!({ "insert": 2, "a": 1, "b": 3 }));
    assert_eq!(uid(&session, 0).as_deref(), Some("bodo.utility"));
    run(&mut session, "fx.set_enabled", json!({ "insert": 2, "slot": 1 }));
    assert!(!insert(&session, 2).slots[0].enabled, "toggles off");
    run(&mut session, "fx.remove", json!({ "insert": 2, "slot": 1 }));
    assert!(insert(&session, 2).slots[0].plugin.is_none());

    // Dragging a knob is one undo step.
    let steps = session.undo_labels().count();
    for value in [0.2, 0.4, 0.6] {
        session.invoke(&bodo_engine::Call::new("fx.set_mix", json!({ "insert": 2, "slot": 2, "mix": value })).in_gesture(3)).unwrap();
    }
    for value in [100.0, 2000.0, 9000.0] {
        session
            .invoke(&bodo_engine::Call::new("plugin.set_param", json!({ "insert": 2, "slot": 2, "param": 1, "value": value })).in_gesture(4))
            .unwrap();
    }
    assert_eq!(session.undo_labels().count(), steps + 2);
}

#[test]
fn sidechains_add_their_route_and_refuse_audio_routes_and_loops() {
    let mut session = session();
    run(&mut session, "fx.load", json!({ "insert": 2, "effect": "Ducker" }));
    run(&mut session, "fx.set_sidechain", json!({ "insert": 2, "slot": 1, "from": "insert:1" }));
    let key = bodo_model::mixer::RouteKey { from: RouteSource::Insert(id(&session, 1)), to: id(&session, 2) };
    assert_eq!(session.project().mixer.routes.get(&key).map(|r| r.kind), Some(RouteKind::Sidechain));
    assert_eq!(insert(&session, 2).slots[0].sidechain, Some(RouteSource::Insert(id(&session, 1))));

    // Effects without a sidechain input refuse one; audio routes must be turned into sidechains first.
    run(&mut session, "fx.load", json!({ "insert": 2, "effect": "Delay" }));
    assert!(session.execute("fx.set_sidechain", json!({ "insert": 2, "slot": 2, "from": "insert:1" })).is_err());
    run(&mut session, "mixer.add_route", json!({ "from": "insert:3", "to": "insert:4" }));
    run(&mut session, "fx.load", json!({ "insert": 4, "effect": "Ducker" }));
    let refused = session.execute("fx.set_sidechain", json!({ "insert": 4, "slot": 1, "from": "insert:3" })).unwrap_err();
    assert!(refused.hint.as_deref().is_some_and(|h| h.contains("sidechain: true")), "{refused}");
    run(&mut session, "mixer.set_route", json!({ "from": "insert:3", "to": "insert:4", "sidechain": true }));
    run(&mut session, "fx.set_sidechain", json!({ "insert": 4, "slot": 1, "from": "insert:3" }));
    // Turning the route back into audio unhooks the slot.
    run(&mut session, "mixer.set_route", json!({ "from": "insert:3", "to": "insert:4", "sidechain": false }));
    assert_eq!(insert(&session, 4).slots[0].sidechain, None);
    assert!(session.execute("fx.set_sidechain", json!({ "insert": 4, "slot": 1, "from": "insert:0" })).is_err(), "not from the master");
    session.project().validate().unwrap();
}

#[test]
fn chains_paste_as_new_instances_into_the_selected_inserts() {
    let mut session = session();
    run(&mut session, "fx.load", json!({ "insert": 1, "slot": 3, "effect": "Delay", "params": { "Feedback": 70 } }));
    run(&mut session, "fx.load", json!({ "insert": 1, "slot": 4, "effect": "Ducker" }));
    run(&mut session, "fx.set_sidechain", json!({ "insert": 1, "slot": 4, "from": "insert:5" }));
    run(&mut session, "fx.copy_chain", json!({ "insert": 1 }));
    run(&mut session, "edit.select", json!({ "inserts": [2, 3] }));
    let pasted = run(&mut session, "fx.paste_chain", json!({}));
    assert_eq!(pasted["inserts"], json!([2, 3]));
    for n in [2, 3] {
        let slots = insert(&session, n).slots;
        assert_eq!(slots[2].param(1), Some(70.0));
        assert_ne!(slots[2].plugin.as_ref().unwrap().instance, insert(&session, 1).slots[2].plugin.as_ref().unwrap().instance);
        assert_eq!(slots[3].sidechain, None, "insert {n} has no route from insert 5");
    }
    session.project().validate().unwrap();
}

#[test]
fn group_selection_moves_inserts_together() {
    let mut session = session();
    run(&mut session, "mixer.set_volume", json!({ "insert": 2, "db": 5.0 }));
    let selected = run(&mut session, "edit.select", json!({ "inserts": [1, 2, 3] }));
    assert_eq!(selected["inserts"].as_array().unwrap().len(), 3);
    assert_eq!(selected["insert"], json!(id(&session, 3)), "the last picked is the selected insert");
    run(&mut session, "edit.select", json!({ "inserts": [3], "toggle": true }));
    run(&mut session, "mixer.adjust", json!({ "volumeDb": 3.0, "pan": -0.25 }));
    assert_eq!(insert(&session, 1).volume_db, 3.0);
    assert_eq!(insert(&session, 2).volume_db, 6.0, "faders stop at +6 dB");
    assert_eq!(insert(&session, 2).pan, -0.25);
    assert_eq!(insert(&session, 3).volume_db, 0.0, "toggled out of the group");
    // Picking one insert alone ends the group.
    run(&mut session, "edit.select", json!({ "insert": 4 }));
    run(&mut session, "mixer.adjust", json!({ "volumeDb": -1.0 }));
    assert_eq!((insert(&session, 1).volume_db, insert(&session, 4).volume_db), (3.0, -1.0));
}

#[test]
fn solo_route_exclusive_and_latency() {
    let mut session = session();
    run(&mut session, "mixer.solo", json!({ "insert": 1 }));
    run(&mut session, "mixer.solo", json!({ "insert": 2 }));
    let soloed = run(&mut session, "mixer.solo", json!({ "insert": 3, "exclusive": true }));
    assert_eq!(soloed["soloed"], json!([3]));

    run(&mut session, "mixer.add_route", json!({ "from": "insert:1", "to": "insert:5" }));
    run(&mut session, "mixer.add_route", json!({ "from": "insert:1", "to": "insert:6" }));
    run(&mut session, "mixer.route_exclusive", json!({ "from": "insert:1", "to": "insert:7" }));
    let from = RouteSource::Insert(id(&session, 1));
    let targets: Vec<InsertId> = session.project().mixer.routes.keys().filter(|k| k.from == from).map(|k| k.to).collect();
    assert_eq!(targets, [id(&session, 7)]);
    assert!(!insert(&session, 1).to_master, "routing exclusively elsewhere leaves the master");
    run(&mut session, "mixer.route_exclusive", json!({ "from": "insert:1", "to": 0 }));
    assert!(insert(&session, 1).to_master && session.project().mixer.routes.keys().all(|k| k.from != from));

    let latency = run(&mut session, "mixer.set_latency_offset", json!({ "insert": 2, "ms": 10.0 }));
    assert_eq!(latency["frames"], 480);
    assert!(session.execute("mixer.set_latency_offset", json!({ "insert": 2, "frames": 96_000 })).is_err());
    run(&mut session, "fx.load", json!({ "insert": 3, "effect": "bodo.latency-test", "params": { "Latency": 256 } }));
    let mixer = inspect::mixer(&session);
    assert_eq!(mixer.data["latency"], 480, "the master waits for the latest path");
    assert!(mixer.text.contains("Latency Test"), "{}", mixer.text);
    run(&mut session, "mixer.set_pdc_bypass", json!({ "insert": 2, "mode": "always" }));
    assert_eq!(inspect::mixer(&session).data["latency"], 256);
}

#[test]
fn loop_record_reaches_the_frontend_and_pan_law_the_project() {
    let mut session = session();
    let _ = session.take_events();
    assert_eq!(run(&mut session, "transport.loop_record", json!({}))["loopRecord"], true);
    assert!(session.take_events().contains(&Event::LoopRecord(true)));
    run(&mut session, "project.set_pan_law", json!({ "law": "linear6dB" }));
    assert_eq!(session.project().settings.pan_law, bodo_kit_dsp::pan::PanLaw::Linear6dB);
}

#[test]
fn renders_write_the_mix_and_one_stem_per_insert() {
    let mut session = session();
    let script: serde_json::Value = serde_json::from_str(include_str!("fixtures/beat.json")).unwrap();
    for call in serde_json::from_value::<Vec<bodo_engine::Call>>(script["calls"].clone()).unwrap() {
        session.invoke(&call).unwrap();
    }
    let dir = temp_dir("stems");
    let mix = run(&mut session, "render.export_mix", json!({ "path": dir.join("song").display().to_string(), "pattern": "Verse", "format": "flac" }));
    let mix_path = std::path::PathBuf::from(mix["path"].as_str().unwrap());
    assert_eq!(mix_path.extension().and_then(|e| e.to_str()), Some("flac"));
    let decoded = bodo_kit_audio::decode_file(&mix_path).unwrap();
    assert!(decoded.frames() > 48_000, "a bar at 96 BPM and a second of tail");

    let stems =
        run(&mut session, "render.export_stems", json!({ "folder": dir.join("stems").display().to_string(), "pattern": "Verse", "mix": true }));
    let files = stems["stems"].as_array().unwrap();
    assert!(files.len() >= 4, "{stems}");
    for file in files {
        let audio = bodo_kit_audio::decode_file(std::path::Path::new(file["path"].as_str().unwrap())).unwrap();
        assert_eq!(audio.frames(), decoded.frames(), "every stem is as long as the mix");
    }
    assert!(dir.join("stems/Mix.wav").exists());
    let empty = session.execute("render.export_stems", json!({ "folder": "x", "end": "1:1:0" })).unwrap_err();
    assert_eq!(empty.code, bodo_engine::ErrorCode::Rejected);
    let _ = std::fs::remove_dir_all(dir);
}
