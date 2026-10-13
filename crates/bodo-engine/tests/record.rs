//! Recording commands: arming, inputs, the record toggle, and takes landing on the playlist as samples, audio clip
//! channels and clips.

#![allow(clippy::unwrap_used, clippy::panic, reason = "test helpers outside #[test] functions")]

mod common;

use std::path::Path;

use bodo_engine::Event;
use bodo_kit_audio::{WavFormat, WavWriter};
use bodo_kit_core::{Frames, SampleRate, Tick};
use bodo_model::channel::ChannelKind;
use bodo_model::mixer::{HwPort, RecordFormat, RecordTap};
use bodo_model::playlist::{ClipOffset, ClipSource};
use common::{run, session, temp_dir};
use serde_json::json;

/// Writes a take of `frames` frames of stereo silence.
fn take(path: &Path, frames: usize) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let silence = vec![0.0f32; frames];
    let mut writer = WavWriter::create(path, SampleRate::HZ_48000, 2, WavFormat::Float32).unwrap();
    writer.write(&[&silence, &silence], frames).unwrap();
    writer.finish().unwrap();
}

#[test]
fn inserts_arm_take_inputs_and_choose_their_tap() {
    let mut session = session();
    run(&mut session, "mixer.arm", json!({ "insert": 2 }));
    run(&mut session, "mixer.set_input", json!({ "insert": 2, "channels": [3] }));
    run(&mut session, "mixer.set_record_tap", json!({ "insert": 2, "tap": "postFader" }));
    run(&mut session, "mixer.set_record_format", json!({ "insert": 2, "format": "int24" }));
    let insert = session.project().mixer.inserts[&session.project().mixer.order[1]].clone();
    assert!(insert.armed);
    assert_eq!(insert.input, Some(HwPort { channels: vec![3] }));
    assert_eq!((insert.record_tap, insert.record_format), (RecordTap::PostFader, RecordFormat::Int24));
    assert!(session.execute("mixer.set_input", json!({ "insert": 2, "channels": [0, 1, 2] })).is_err(), "at most stereo");
    run(&mut session, "mixer.set_input", json!({ "insert": 2, "channels": null }));
    run(&mut session, "mixer.arm", json!({ "insert": 2 }));
    let insert = session.project().mixer.inserts[&session.project().mixer.order[1]].clone();
    assert_eq!((insert.armed, insert.input.clone()), (false, None));
}

#[test]
fn the_record_toggle_starts_and_stops_recording() {
    let mut session = session();
    session.take_events();
    let started = run(&mut session, "transport.record", json!({}));
    assert_eq!((started["recording"].as_bool(), started["playing"].as_bool()), (Some(true), Some(true)));
    assert_eq!(session.take_events(), vec![Event::Record]);
    let stopped = run(&mut session, "transport.record", json!({}));
    assert_eq!((stopped["recording"].as_bool(), stopped["playing"].as_bool()), (Some(false), Some(false)));
    assert!(session.take_events().contains(&Event::Stop));
    run(&mut session, "transport.record", json!({}));
    run(&mut session, "transport.stop", json!({}));
    assert!(!session.transport().recording, "stopping the transport ends a recording");
}

#[test]
fn takes_land_on_free_tracks_at_their_compensated_start() {
    let dir = temp_dir("takes");
    let mut session = session();
    run(&mut session, "project.save", json!({ "path": dir.join("Song.bodo").display().to_string() }));
    let first = dir.join("Recorded").join("Insert 3_1.wav");
    let second = dir.join("Recorded").join("Insert 4_1.wav");
    let early = dir.join("Recorded").join("Insert 3_2.wav");
    take(&first, 48_000);
    take(&second, 48_000);
    take(&early, 48_000);
    let undo_steps = session.undo_labels().count();
    let result = run(
        &mut session,
        "recording.add_takes",
        json!({ "takes": [
            { "insert": 3, "path": first.display().to_string(), "start": "2:1:0", "frames": 48_000 },
            { "insert": 4, "path": second.display().to_string(), "start": "2:2:0", "frames": 48_000 },
        ] }),
    );
    assert_eq!(session.undo_labels().count(), undo_steps + 1, "a recording is one undo step");
    let project = session.project().clone();
    let takes = result["takes"].as_array().unwrap();
    assert_eq!(takes[0]["name"].as_str(), Some("Insert 3 take 1"));
    assert_eq!(takes[1]["name"].as_str(), Some("Insert 4 take 1"));

    // One second at 120 BPM is two beats: 1920 ticks. The second take overlaps the first, so it goes a track down.
    let track = |n: usize| project.tracks[&project.track_order[n]].clone();
    let one = track(0).clips.iter().find(|c| matches!(c.source, ClipSource::Audio(_))).cloned().unwrap();
    assert_eq!((one.start, one.len, one.offset), (Tick(3840), Tick(1920), ClipOffset::Frames(Frames(0))));
    let two = track(1).clips.iter().find(|c| matches!(c.source, ClipSource::Audio(_))).cloned().unwrap();
    assert_eq!(two.start, Tick(3840 + 960));

    // The channel plays the sample, stored relative to the project, into the insert that recorded it.
    let ClipSource::Audio(channel) = one.source else { panic!() };
    let channel = project.channels[&channel].clone();
    assert_eq!(channel.target, Some(project.mixer.order[2]));
    let ChannelKind::AudioClip { sample, .. } = channel.kind else { panic!("{:?}", channel.kind) };
    assert_eq!(project.samples[&sample].path, "Recorded/Insert 3_1.wav");
    assert!(project.samples[&sample].hash.is_some());

    // A take that began half a second before the song skips that half second.
    run(
        &mut session,
        "recording.add_takes",
        json!({ "takes": [{ "insert": 3, "path": early.display().to_string(), "start": -960, "frames": 48_000 }] }),
    );
    let project = session.project().clone();
    let named = project.channels.values().find(|c| c.name == "Insert 3 take 2").unwrap().id;
    let clip = project.tracks.values().flat_map(|t| t.clips.iter()).find(|c| c.source == ClipSource::Audio(named)).cloned().unwrap();
    assert_eq!((clip.start, clip.len, clip.offset), (Tick::ZERO, Tick(960), ClipOffset::Frames(Frames(24_000))));

    session.undo();
    session.undo();
    assert!(!session.project().channels.values().any(|c| c.name.contains("take")), "undo takes the recording back");
    std::fs::remove_dir_all(dir).unwrap();
}
