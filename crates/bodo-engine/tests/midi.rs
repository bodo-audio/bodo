//! MIDI file commands: exporting the beat writes every note where it plays, importing makes a pattern and channels,
//! and a broken file changes nothing.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use bodo_engine::{Call, ErrorCode, Session, inspect};
use bodo_kit_midi::smf::{EventKind, Format, MetaEvent, Timing, Track, TrackEvent};
use bodo_kit_midi::{MidiMessage, Smf, U4, U7};
use common::{run, session, temp_dir};
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

#[test]
fn the_beat_exports_every_note_where_it_plays() {
    let dir = temp_dir("midi-export");
    let mut session = beat();
    let path = dir.join("Beat");
    let result = run(&mut session, "project.export_midi", json!({ "path": path.display().to_string() }));
    // Intro 6 notes for 2 bars, Verse 17 for 4 bars, Fill 10 twice (two tracks), Outro 2.
    assert_eq!(result["notes"], json!(6 * 2 + 17 * 4 + 10 * 2 + 2));
    assert_eq!(result["tracks"], json!(5), "one track per channel");
    let file = dir.join("Beat.mid");
    assert_eq!(result["path"], json!(file.display().to_string()), ".mid is added");
    let imported = bodo_io::read_midi_file(&file).unwrap();
    assert_eq!(imported.tempo[0].bpm, 96.0);
    let kick = imported.parts.iter().find(|p| p.name == "Kick").unwrap();
    let starts: Vec<i64> = kick.notes.iter().map(|n| n.start.0).take(4).collect();
    assert_eq!(starts, [0, 1920, 3840, 5760], "the Intro's kick on beats 1 and 3 of bars 1 and 2");
    assert_eq!(kick.notes.iter().filter(|n| n.start.0 == 7680).count(), 1, "the Verse starts at bar 3");
    // Exporting writes a file and leaves the project alone.
    assert!(!session.undo_labels().any(|l| l.contains("MIDI")));

    let verse = dir.join("Verse.mid");
    let result = run(&mut session, "pattern.export_midi", json!({ "pattern": "Verse", "path": verse.display().to_string() }));
    assert_eq!(result["notes"], json!(17));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn an_exported_pattern_imports_as_the_same_steps() {
    let dir = temp_dir("midi-round-trip");
    let mut session = beat();
    let path = dir.join("Verse.mid");
    run(&mut session, "pattern.export_midi", json!({ "pattern": "Verse", "path": path.display().to_string() }));

    let mut fresh = common::session();
    let result = run(&mut fresh, "project.import_midi", json!({ "path": path.display().to_string() }));
    assert_eq!(result["name"], json!("Verse"));
    assert_eq!(result["notes"], json!(17));
    assert_eq!(result["tempo"], json!(96.0));
    let names: Vec<&str> = result["channels"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Kick 2", "Clap 2", "HiHat 2", "808"], "named after the parts, kept unique");
    let summary = inspect::project(&fresh).text;
    for line in [
        "Kick 2       x...x...x...x...",
        "Clap 2       ....x.......x...",
        "HiHat 2      x.x.x.x.x.x.x.x.",
        "808          x.....x...x.....",
        "Verse at 1:1:0",
    ] {
        assert!(summary.contains(line), "missing `{line}` in:\n{summary}");
    }
    assert!(summary.contains("96 BPM"), "{summary}");
    // One undo step takes it all back.
    assert_eq!(fresh.undo_labels().last(), Some("Import Verse"));
    fresh.undo();
    assert_eq!(fresh.project().patterns.len(), 1);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn imports_place_the_pattern_where_asked() {
    let dir = temp_dir("midi-place");
    let mut session = beat();
    let path = dir.join("Fill.mid");
    run(&mut session, "pattern.export_midi", json!({ "pattern": "Fill", "path": path.display().to_string() }));
    let result = run(&mut session, "project.import_midi", json!({ "path": path.display().to_string(), "tempo": false, "track": 3, "at": "5:1:0" }));
    assert_eq!(result["name"], json!("Fill 2"));
    assert_eq!(result["clip"]["start"], json!("5:1:0"));
    assert_eq!(result["tempo"], Value::Null);
    let summary = inspect::project(&session).text;
    assert!(summary.contains("3 Track 3: Fill 2 at 5:1:0 (1 bar)"), "{summary}");
    // Without a track, the clip goes on the first empty track at bar 1.
    let result = run(&mut session, "project.import_midi", json!({ "path": path.display().to_string() }));
    assert_eq!(result["clip"]["start"], json!("1:1:0"));
    assert!(inspect::project(&session).text.contains("4 Track 4: Fill 3 at 1:1:0"));
    // Or nowhere.
    let result = run(&mut session, "project.import_midi", json!({ "path": path.display().to_string(), "place": false }));
    assert_eq!(result["clip"], Value::Null);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn files_in_another_division_import_exactly() {
    let dir = temp_dir("midi-division");
    let note = |tick: u64, key: u8, on: bool| TrackEvent {
        tick,
        kind: EventKind::Channel {
            channel: U4::new(9).unwrap(),
            message: if on {
                MidiMessage::NoteOn { key: U7::new(key).unwrap(), velocity: U7::new(100).unwrap() }
            } else {
                MidiMessage::NoteOff { key: U7::new(key).unwrap(), velocity: U7::MIN }
            },
        },
    };
    let track = Track {
        events: vec![
            TrackEvent { tick: 0, kind: EventKind::Meta(MetaEvent::TrackName(b"Drums".to_vec())) },
            note(0, 36, true),
            note(24, 36, false),
            note(48, 38, true),
            note(72, 38, false),
        ],
        end: 96,
    };
    let bytes = Smf { format: Format::SingleTrack, timing: Timing::Metrical { ticks_per_quarter: 96 }, tracks: vec![track] }.to_bytes().unwrap();
    let path = dir.join("drums.mid");
    std::fs::write(&path, bytes).unwrap();
    let mut session = session();
    run(&mut session, "project.import_midi", json!({ "path": path.display().to_string() }));
    let pattern = inspect::pattern(&session, None).unwrap().text;
    assert!(pattern.contains("C2 at 1:1:0 for 1 step"), "{pattern}");
    assert!(pattern.contains("D2 at 1:1:480 for 1 step"), "{pattern}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn broken_and_empty_files_change_nothing() {
    let dir = temp_dir("midi-broken");
    let mut session = beat();
    let before = session.project().clone();
    let broken = dir.join("broken.mid");
    std::fs::write(&broken, b"MThd not really").unwrap();
    let error = session.execute("project.import_midi", json!({ "path": broken.display().to_string() })).unwrap_err();
    assert_eq!(error.code, ErrorCode::Io);
    assert!(error.message.contains("broken.mid"), "{error}");
    let missing = session.execute("project.import_midi", json!({ "path": dir.join("nope.mid").display().to_string() })).unwrap_err();
    assert_eq!(missing.code, ErrorCode::Io);
    let empty = dir.join("empty.mid");
    let bytes =
        Smf { format: Format::SingleTrack, timing: Timing::Metrical { ticks_per_quarter: 480 }, tracks: vec![Track::default()] }.to_bytes().unwrap();
    std::fs::write(&empty, bytes).unwrap();
    let error = session.execute("project.import_midi", json!({ "path": empty.display().to_string() })).unwrap_err();
    assert_eq!(error.code, ErrorCode::Rejected);
    assert!(std::sync::Arc::ptr_eq(session.project(), &before), "nothing changed");

    let mut fresh = common::session();
    let error = fresh.execute("project.export_midi", json!({ "path": dir.join("x.mid").display().to_string() })).unwrap_err();
    assert_eq!(error.code, ErrorCode::Rejected, "an empty playlist has nothing to export");
    std::fs::remove_dir_all(dir).unwrap();
}
