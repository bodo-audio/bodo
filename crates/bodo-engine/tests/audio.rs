//! Audio clip channels and clips through commands: default lengths, stretch modes, reuse, fades and refusals.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use bodo_engine::ErrorCode;
use bodo_model::channel::{ChannelKind, StretchMode};
use bodo_model::playlist::ClipSource;
use common::{run, session};
use serde_json::json;

const CRASH: &str = "factory:Packs/Bodo Drums/Crash.wav";

#[test]
fn an_audio_channel_records_the_tempo_and_its_clips_last_the_whole_sample() {
    let mut session = session();
    run(&mut session, "project.set_tempo", json!({ "bpm": 120 }));
    let made = run(&mut session, "channel.add_audio", json!({ "sample": CRASH }));
    assert_eq!(made["name"], "Crash");
    assert_eq!(made["tempo"], 120.0, "the project's tempo when the channel was made");
    let channel = made["channel"].as_str().unwrap().to_owned();
    let kind = session.project().channels.values().find(|c| c.name == "Crash").map(|c| c.kind.clone()).unwrap();
    assert!(matches!(kind, ChannelKind::AudioClip { stretch: StretchMode::None, tempo: Some(t), .. } if t == 120.0));

    // Crash.wav lasts 3.142 s: 6.284 beats at 120 BPM.
    let placed = run(&mut session, "clip.place_audio", json!({ "channel": channel, "start": "2:1:0", "track": 2 }));
    let ticks = placed["ticks"].as_i64().unwrap();
    assert!((ticks - 6033).abs() <= 2, "{ticks} ticks");
    // At its own speed, a slower tempo makes the same file fewer ticks long.
    run(&mut session, "project.set_tempo", json!({ "bpm": 60 }));
    let slow = run(&mut session, "clip.place_audio", json!({ "channel": channel, "start": "5:1:0" }));
    assert!((slow["ticks"].as_i64().unwrap() - ticks / 2).abs() <= 2);
    // Following the tempo, the length in beats stays the sample's.
    run(&mut session, "clip.set_stretch", json!({ "channel": channel, "mode": "stretch" }));
    let stretched = run(&mut session, "clip.place_audio", json!({ "channel": channel, "start": "9:1:0" }));
    assert!((stretched["ticks"].as_i64().unwrap() - ticks).abs() <= 2);
    let length = run(&mut session, "clip.place_audio", json!({ "channel": channel, "start": "13:1:0", "length": "1 bar" }));
    assert_eq!(length["ticks"], 3840);
}

#[test]
fn placing_a_sound_reuses_the_channel_that_plays_it() {
    let mut session = session();
    let first = run(&mut session, "clip.place_sound", json!({ "sample": CRASH, "start": 0 }));
    let second = run(&mut session, "clip.place_sound", json!({ "sample": CRASH, "start": "3:1:0", "track": 2 }));
    assert_eq!(first["newChannel"], true);
    assert_eq!(second["newChannel"], false);
    assert_eq!(first["channel"], second["channel"]);
    let clips: usize = session.project().tracks.values().map(|t| t.clips.iter().filter(|c| matches!(c.source, ClipSource::Audio(_))).count()).sum();
    assert_eq!(clips, 2);
}

#[test]
fn fades_are_set_clamped_and_removed() {
    let mut session = session();
    run(&mut session, "clip.place_sound", json!({ "sample": CRASH, "start": 0 }));
    run(&mut session, "clip.set_fades", json!({ "fadeIn": "1 beat", "fadeOut": "1000 bars" }));
    let clip = session.project().tracks.values().flat_map(|t| t.clips.iter()).next().cloned().unwrap();
    let fades = clip.fades.unwrap();
    assert_eq!(fades.fade_in.0, 960);
    assert_eq!(fades.fade_out, clip.len, "a fade can't be longer than its clip");
    run(&mut session, "clip.set_fades", json!({ "fadeIn": 0, "fadeOut": "0 beats" }));
    let clip = session.project().tracks.values().flat_map(|t| t.clips.iter()).next().cloned().unwrap();
    assert_eq!(clip.fades, None);
    assert_eq!(session.execute("clip.set_fades", json!({})).unwrap_err().code, ErrorCode::InvalidParams);
}

#[test]
fn audio_commands_refuse_what_they_cant_do() {
    let mut session = session();
    let not_audio = session.execute("clip.place_audio", json!({ "channel": "Kick", "start": 0 })).unwrap_err();
    assert_eq!(not_audio.code, ErrorCode::Rejected);
    assert!(not_audio.to_string().contains("not an audio clip channel"), "{not_audio}");
    let missing = session.execute("channel.add_audio", json!({ "sample": "nowhere/nothing.wav" })).unwrap_err();
    assert_eq!(missing.code, ErrorCode::NotFound);
    run(&mut session, "playlist.set_track_props", json!({ "track": 1, "locked": true }));
    let locked = session.execute("clip.place_sound", json!({ "sample": CRASH, "start": 0, "track": 1 })).unwrap_err();
    assert_eq!(locked.code, ErrorCode::Rejected);
    let tempo = session.execute("channel.add_audio", json!({ "sample": CRASH, "tempo": 0.5 })).unwrap_err();
    assert_eq!(tempo.code, ErrorCode::InvalidParams);
    assert!(session.project().validate().is_ok());
}
