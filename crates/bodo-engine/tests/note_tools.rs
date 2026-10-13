//! What each piano roll tool does to notes (project.md section 10.4).

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use bodo_engine::{Call, ErrorCode, Session};
use bodo_model::Note;
use common::{run, session};
use serde_json::{Value, json};

/// Adds notes `(key, start, length)` to the Kick channel of the current pattern and returns their ids.
fn add(session: &mut Session, notes: &[(u8, i64, i64)]) -> Vec<u64> {
    let notes: Vec<Value> = notes.iter().map(|(key, start, len)| json!({ "key": key, "start": start, "length": len })).collect();
    let result = run(session, "note.add_many", json!({ "channel": "Kick", "notes": notes }));
    result["notes"].as_array().unwrap().iter().map(|n| n.as_u64().unwrap()).collect()
}

/// The Kick channel's notes in the current pattern, sorted.
fn kick(session: &Session) -> Vec<Note> {
    let project = session.project();
    let pattern = project.patterns.get(&session.editor().pattern.unwrap()).unwrap();
    let kick = project.channels.values().find(|c| c.name == "Kick").unwrap().id;
    pattern.notes.get(&kick).map(|seq| seq.notes().iter().cloned().collect()).unwrap_or_default()
}

fn starts(notes: &[Note]) -> Vec<i64> {
    notes.iter().map(|n| n.start.0).collect()
}

fn lengths(notes: &[Note]) -> Vec<i64> {
    notes.iter().map(|n| n.len.0).collect()
}

fn keys(notes: &[Note]) -> Vec<u8> {
    notes.iter().map(|n| n.key).collect()
}

#[test]
fn quantize_moves_by_strength_and_remembers_the_grid() {
    let mut s = session();
    let ids = add(&mut s, &[(60, 300, 100), (62, 1000, 300)]);
    run(&mut s, "note.quantize", json!({ "notes": ids, "grid": 240, "strength": 0.5 }));
    let notes = kick(&s);
    assert_eq!(starts(&notes), [270, 980], "half the distance to 240 and 960");
    assert_eq!(notes.iter().map(|n| n.quantized_start.unwrap().0).collect::<Vec<_>>(), [240, 960]);
    run(&mut s, "note.quantize", json!({ "grid": 240, "ends": true }));
    let notes = kick(&s);
    assert_eq!(starts(&notes), [240, 960]);
    assert_eq!(lengths(&notes), [240, 240], "ends land on the grid too");
}

#[test]
fn transpose_refuses_to_leave_the_key_range() {
    let mut s = session();
    let ids = add(&mut s, &[(60, 0, 240), (125, 240, 240)]);
    run(&mut s, "note.transpose", json!({ "notes": [ids[0]], "semitones": -12 }));
    assert_eq!(keys(&kick(&s)), [48, 125]);
    let before = s.project().clone();
    let error = s.execute("note.transpose", json!({ "notes": ids, "semitones": 5 })).unwrap_err();
    assert_eq!(error.code, ErrorCode::Rejected);
    assert_eq!(**s.project(), *before, "nothing moved");
}

#[test]
fn legato_fills_the_gaps() {
    let mut s = session();
    add(&mut s, &[(60, 0, 100), (64, 0, 50), (62, 480, 100), (65, 960, 120)]);
    run(&mut s, "edit.select_all", json!({ "what": "notes" }));
    run(&mut s, "note.legato", json!({}));
    let notes = kick(&s);
    assert_eq!(starts(&notes), [0, 0, 480, 960]);
    assert_eq!(lengths(&notes), [480, 480, 480, 120], "the chord reaches the next note; the last keeps its length");
}

#[test]
fn chop_cuts_pieces_and_selects_them() {
    let mut s = session();
    let ids = add(&mut s, &[(60, 0, 1000)]);
    let result = run(&mut s, "note.chop", json!({ "notes": ids, "length": "1 step" }));
    let notes = kick(&s);
    assert_eq!(starts(&notes), [0, 240, 480, 720, 960]);
    assert_eq!(lengths(&notes), [240, 240, 240, 240, 40]);
    assert_eq!(result["notes"].as_array().unwrap().len(), 5);
    assert_eq!(s.editor().notes.len(), 5);
    s.project().validate().unwrap();
}

#[test]
fn glue_joins_touching_notes_of_a_key() {
    let mut s = session();
    add(&mut s, &[(60, 0, 240), (60, 240, 240), (60, 600, 100), (62, 100, 100)]);
    run(&mut s, "edit.select_all", json!({ "what": "notes" }));
    run(&mut s, "note.glue", json!({}));
    let notes = kick(&s);
    assert_eq!(starts(&notes), [0, 100, 600]);
    assert_eq!(lengths(&notes), [480, 100, 100]);
}

#[test]
fn strum_rolls_chords_and_keeps_their_ends() {
    let mut s = session();
    let ids = add(&mut s, &[(60, 0, 480), (64, 0, 480), (67, 0, 480)]);
    run(&mut s, "note.strum", json!({ "notes": ids, "time": 20 }));
    let notes = kick(&s);
    assert_eq!(starts(&notes), [0, 20, 40]);
    assert_eq!(keys(&notes), [60, 64, 67]);
    assert!(notes.iter().all(|n| n.end().0 == 480));
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "note.strum", json!({ "notes": ids, "time": 20, "direction": "down" }));
    let notes = kick(&s);
    assert_eq!((starts(&notes), keys(&notes)), (vec![0, 20, 40], vec![67, 64, 60]), "down starts from the top");
}

#[test]
fn arpeggios_play_chords_in_order() {
    let mut s = session();
    let ids = add(&mut s, &[(60, 0, 960), (64, 0, 960), (67, 0, 960)]);
    run(&mut s, "note.arpeggiate", json!({ "notes": ids, "rate": "1 step" }));
    let notes = kick(&s);
    assert_eq!(starts(&notes), [0, 240, 480, 720]);
    assert_eq!(keys(&notes), [60, 64, 67, 60]);
    assert_eq!(lengths(&notes), [240; 4]);
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "note.arpeggiate", json!({ "notes": ids, "order": "down", "rate": "1 step", "gate": 0.5 }));
    let notes = kick(&s);
    assert_eq!(keys(&notes), [67, 64, 60, 67]);
    assert_eq!(lengths(&notes), [120; 4]);
}

#[test]
fn flip_reverses_time_or_mirrors_keys() {
    let mut s = session();
    let ids = add(&mut s, &[(60, 0, 240), (67, 480, 240)]);
    run(&mut s, "note.flip", json!({ "notes": ids, "axis": "time" }));
    let notes = kick(&s);
    assert_eq!((starts(&notes), keys(&notes)), (vec![0, 480], vec![67, 60]));
    run(&mut s, "note.flip", json!({ "notes": ids, "axis": "keys" }));
    assert_eq!(keys(&kick(&s)), [60, 67]);
}

#[test]
fn humanize_repeats_per_seed_and_unhumanize_restores_exactly() {
    let build = |seed: u64| {
        let mut s = session();
        let ids = add(&mut s, &[(60, 0, 240), (62, 240, 240), (64, 480, 240), (65, 720, 240)]);
        run(&mut s, "note.humanize", json!({ "notes": ids, "timingMs": 20, "velocity": 0.2, "seed": seed }));
        s
    };
    let (a, b, c) = (build(5), build(5), build(6));
    assert_eq!(kick(&a), kick(&b), "the same seed gives the same performance");
    assert_ne!(kick(&a), kick(&c));
    let notes = kick(&a);
    assert!(notes.iter().any(|n| n.start.0 % 240 != 0), "starts drifted: {:?}", starts(&notes));
    assert!(notes.iter().all(|n| (n.start.0 - n.quantized_start.unwrap().0).abs() < 200), "a few dozen ms at most");
    let mut a = a;
    run(&mut a, "note.unhumanize", json!({}));
    let back = kick(&a);
    assert_eq!(starts(&back), [0, 240, 480, 720]);
    assert!(back.iter().all(|n| n.quantized_start.is_none()));
}

#[test]
fn limit_to_scale_snaps_keys() {
    let mut s = session();
    let ids = add(&mut s, &[(61, 0, 240), (66, 240, 240), (64, 480, 240)]);
    run(&mut s, "note.limit_to_scale", json!({ "notes": ids, "root": "C", "scale": "major" }));
    assert_eq!(keys(&kick(&s)), [60, 65, 64]);
    let error = s.execute("note.limit_to_scale", json!({ "notes": ids, "root": "C", "scale": "bebop" })).unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidParams);
}

#[test]
fn stamp_adds_a_chord_and_selects_it() {
    let mut s = session();
    let result = run(&mut s, "note.stamp", json!({ "channel": "Kick", "chord": "Am7", "octave": 3, "start": "1:2:0" }));
    assert_eq!(result["keys"], json!([57, 60, 64, 67]));
    let notes = kick(&s);
    assert_eq!(keys(&notes), [57, 60, 64, 67]);
    assert!(notes.iter().all(|n| n.start.0 == 960 && n.len.0 == 960));
    assert_eq!(s.editor().notes.len(), 4);
    assert_eq!(s.execute("note.stamp", json!({ "channel": "Kick", "chord": "Q7", "start": 0 })).unwrap_err().code, ErrorCode::InvalidParams);
}

#[test]
fn expression_curves_set_and_clear() {
    let mut s = session();
    let ids = add(&mut s, &[(60, 0, 480)]);
    run(&mut s, "note.set_expression", json!({ "note": ids[0], "pitch": [{ "at": 240, "value": 1.0 }, { "at": 0, "value": 0.0 }] }));
    let expr = kick(&s)[0].expr.clone().unwrap();
    assert_eq!(expr.pitch.iter().map(|p| p.at.0).collect::<Vec<_>>(), [0, 240], "points sorted by time");
    run(&mut s, "note.set_expression", json!({ "note": ids[0], "pitch": [] }));
    assert!(kick(&s)[0].expr.is_none(), "an empty curve clears it");
}

#[test]
fn a_lane_brush_across_notes_is_one_undo_step() {
    let mut s = session();
    let ids = add(&mut s, &[(60, 0, 240), (62, 240, 240), (64, 480, 240)]);
    let steps = s.undo_labels().count();
    for (id, velocity) in ids.iter().zip([0.2, 0.4, 0.6]) {
        s.invoke(&Call::new("note.set_props", json!({ "notes": [id], "velocity": velocity, "releaseVelocity": 0.5, "modX": 0.3 })).in_gesture(3))
            .unwrap();
    }
    assert_eq!(s.undo_labels().count(), steps + 1);
    let notes = kick(&s);
    assert_eq!(notes.iter().map(|n| (f32::from(n.velocity) / 65535.0 * 10.0).round()).collect::<Vec<_>>(), [2.0, 4.0, 6.0]);
    assert!(notes.iter().all(|n| (f32::from(n.release_velocity) / 65535.0 - 0.5).abs() < 0.01 && (n.mod_x - 0.3).abs() < 1e-6));
    run(&mut s, "note.set_props", json!({ "notes": [ids[0]], "slide": true, "porta": true, "finePitchCents": -25.0, "modY": 1.0 }));
    let first = &kick(&s)[0];
    assert!(first.slide && first.porta && first.fine_pitch_cents == -25.0 && first.mod_y == 1.0);
    assert_eq!(s.execute("note.set_props", json!({ "notes": [ids[0]], "modX": 2.0 })).unwrap_err().code, ErrorCode::InvalidParams);
}

#[test]
fn slice_cuts_only_the_notes_that_cross_the_line() {
    let mut s = session();
    let ids = add(&mut s, &[(60, 0, 480), (62, 480, 240), (64, 0, 120)]);
    let result = run(&mut s, "note.slice", json!({ "notes": ids, "at": 240 }));
    assert_eq!(result["notes"].as_array().unwrap().len(), 4, "the halves of the first note and the two untouched ones");
    let notes = kick(&s);
    let c4: Vec<(i64, i64)> = notes.iter().filter(|n| n.key == 60).map(|n| (n.start.0, n.len.0)).collect();
    assert_eq!(c4, [(0, 240), (240, 240)]);
    assert_eq!(notes.iter().filter(|n| n.key != 60).count(), 2, "notes that don't cross stay whole");
}

#[test]
fn a_painted_or_erased_stroke_is_one_undo_step() {
    let mut s = session();
    let before = s.undo_labels().count();
    for start in [0, 240, 480] {
        let call = Call::new("note.add_many", json!({ "channel": "Kick", "notes": [{ "key": 60, "start": start, "length": 240 }] }));
        s.invoke(&call.in_gesture(7)).unwrap();
    }
    assert_eq!(kick(&s).len(), 3);
    assert_eq!(s.undo_labels().count(), before + 1);
    let ids: Vec<u32> = kick(&s).iter().map(|n| n.id.0).collect();
    for id in ids {
        s.invoke(&Call::new("note.remove", json!({ "notes": [id] })).in_gesture(8)).unwrap();
    }
    assert!(kick(&s).is_empty());
    assert_eq!(s.undo_labels().count(), before + 2);
}
