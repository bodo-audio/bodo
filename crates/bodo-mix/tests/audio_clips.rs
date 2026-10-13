//! Audio clips on the playlist: exact placement, offsets and lengths, fades, resampling and stretching to the tempo,
//! mutes, seeking into a clip, and determinism.

#![allow(clippy::unwrap_used, clippy::panic, reason = "test helpers outside #[test] functions")]

mod common;

use std::path::Path;
use std::sync::Arc;

use bodo_kit_audio::{WavFormat, WavWriter};
use bodo_kit_core::{Frames, SampleRate, Seconds, Tick};
use bodo_kit_dsp::AudioBuffer;
use bodo_kit_dsp::pan::PanLaw;
use bodo_mix::{EngineConfig, OfflineOptions, PlayMode, engine, render_offline};
use bodo_model::channel::{ChannelKind, StretchMode};
use bodo_model::playlist::{ClipOffset, ClipSource, Fades};
use bodo_model::*;
use common::{Builder, temp_dir};

/// The value of frame `i` of the ramp: distinct for every frame, below 0 dBFS.
fn ramp(i: usize) -> f32 {
    (i as f32 + 1.0) / 200_000.0
}

/// Writes a mono 48 kHz ramp of two seconds.
fn write_ramp(dir: &Path) {
    let samples: Vec<f32> = (0..96_000).map(ramp).collect();
    let mut writer = WavWriter::create(&dir.join("ramp.wav"), SampleRate::HZ_48000, 1, WavFormat::Float32).unwrap();
    writer.write(&[&samples], samples.len()).unwrap();
    writer.finish().unwrap();
}

/// Writes a mono 48 kHz sine at 440 Hz lasting two seconds.
fn write_sine(dir: &Path) {
    let samples: Vec<f32> = (0..96_000).map(|i| (2.0 * std::f64::consts::PI * 440.0 * i as f64 / 48_000.0).sin() as f32 * 0.5).collect();
    let mut writer = WavWriter::create(&dir.join("sine.wav"), SampleRate::HZ_48000, 1, WavFormat::Float32).unwrap();
    writer.write(&[&samples], samples.len()).unwrap();
    writer.finish().unwrap();
}

/// A project at 120 BPM with an audio clip channel playing `file`, and one clip of it shaped by `clip`.
fn project(file: &str, mode: StretchMode, tempo: Option<f64>, clip: impl FnOnce(&mut Clip), track_muted: bool) -> Project {
    let mut b = Builder::new();
    let sample = b.sample(file);
    let id = ChannelId::from_u128(b.id());
    let channel = Channel {
        id,
        name: "Audio".into(),
        color: Color::GRAY,
        kind: ChannelKind::AudioClip { sample, stretch: mode, tempo },
        target: None,
        volume: 1.0,
        pan: 0.0,
        pitch_cents: 0.0,
        swing: 0.0,
        muted: false,
        midi_in: None,
        group: None,
    };
    b.project.channels.insert(id, Arc::new(channel));
    b.project.rack_order.push_back(id);
    let track = TrackId::from_u128(b.id());
    let mut value = Clip {
        id: ClipId::from_u128(b.id()),
        start: Tick(3840),
        len: Tick(960),
        offset: ClipOffset::default(),
        muted: false,
        color: None,
        source: ClipSource::Audio(id),
        fades: None,
    };
    clip(&mut value);
    let mut playlist = PlaylistTrack::new(track, "Track", Color::GRAY);
    playlist.muted = track_muted;
    playlist.insert_clip(value);
    b.project.tracks.insert(track, Arc::new(playlist));
    b.project.track_order.push_back(track);
    b.build()
}

/// The left channel of `project` rendered from the start for three bars.
fn render(project: &Project, dir: &Path, block: usize) -> Vec<f32> {
    let options = OfflineOptions { end: Tick(3 * 3840), tail: Seconds(0.0), mode: PlayMode::Song, block, ..OfflineOptions::default() };
    render_offline(project, &SampleRoots::new(dir), &options).unwrap().channels.swap_remove(0)
}

/// The gain a centered mono channel gets on each side.
fn center() -> f32 {
    PanLaw::default().mono_gains(0.0).left
}

#[test]
fn a_clip_plays_its_sample_at_exactly_its_frames() {
    let dir = temp_dir("audio-exact");
    write_ramp(&dir);
    // Bar 2 at 120 BPM starts at 2 s (frame 96,000); one beat lasts 24,000 frames; an offset of half a beat skips
    // 12,000 frames of the sample.
    let out = render(&project("ramp.wav", StretchMode::None, None, |c| c.offset = ClipOffset::Ticks(Tick(480)), false), &dir, 512);
    let g = center();
    assert!(out[..96_000].iter().all(|s| *s == 0.0), "silent before the clip");
    for k in [0usize, 1, 777, 23_999] {
        let expected = ramp(12_000 + k) * g;
        assert!((out[96_000 + k] - expected).abs() < 1e-7, "frame {k}: {} against {expected}", out[96_000 + k]);
    }
    assert!(out[120_000..].iter().all(|s| *s == 0.0), "silent after the clip's end");

    // An offset in frames reads from that frame of the file, whatever the tempo.
    let out = render(&project("ramp.wav", StretchMode::None, None, |c| c.offset = ClipOffset::Frames(Frames(100)), false), &dir, 333);
    assert!((out[96_000] - ramp(100) * g).abs() < 1e-7);
}

#[test]
fn fades_shape_the_gain() {
    let dir = temp_dir("audio-fades");
    write_ramp(&dir);
    let fades = Fades { fade_in: Tick(480), fade_out: Tick(240) };
    let out = render(&project("ramp.wav", StretchMode::None, None, |c| c.fades = Some(fades), false), &dir, 512);
    let g = center();
    assert_eq!(out[96_000], 0.0, "a fade-in starts from silence");
    let half = 6_000;
    assert!((out[96_000 + half] - ramp(half) * g * 0.5).abs() < 1e-7, "halfway through the 12,000-frame fade-in");
    let end = 120_000;
    assert!((out[end - 3_000] - ramp(24_000 - 3_000) * g * 0.5).abs() < 1e-7, "halfway through the 6,000-frame fade-out");
    assert!((out[96_000 + 15_000] - ramp(15_000) * g).abs() < 1e-7, "full gain between the fades");
}

#[test]
fn resampling_follows_the_tempo_pitch_and_all() {
    let dir = temp_dir("audio-resample");
    write_ramp(&dir);
    // The sample's tempo is 60 BPM and the project's 120: it plays twice as fast, reading every second frame.
    let out = render(&project("ramp.wav", StretchMode::Resample, Some(60.0), |c| c.len = Tick(3840), false), &dir, 512);
    let g = center();
    for k in [0usize, 10, 1_000, 47_999] {
        assert!((out[96_000 + k] - ramp(2 * k) * g).abs() < 1e-7, "frame {k}");
    }
    // Two seconds of sample last one second.
    assert!(out[96_000 + 48_000 + 8..].iter().all(|s| s.abs() < 1e-9), "the sample ends after half its length");
}

#[test]
fn stretching_follows_the_tempo_and_keeps_the_pitch() {
    let dir = temp_dir("audio-stretch");
    write_sine(&dir);
    let out = render(&project("sine.wav", StretchMode::Stretch, Some(60.0), |c| c.len = Tick(3840), false), &dir, 512);
    let audible = |from: usize, to: usize| out[from..to].iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(audible(96_000 + 1_000, 96_000 + 40_000) > 0.2, "it plays");
    assert!(audible(96_000 + 52_000, 96_000 + 96_000) < 1e-3, "two seconds of sample last one second");
    // The pitch stays at 440 Hz: count zero crossings in the steady middle.
    let middle = &out[96_000 + 8_000..96_000 + 40_000];
    let crossings = middle.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count();
    let hz = crossings as f64 / 2.0 / (middle.len() as f64 / 48_000.0);
    assert!((hz - 440.0).abs() < 15.0, "{hz} Hz");
}

#[test]
fn muted_clips_and_tracks_are_silent() {
    let dir = temp_dir("audio-mute");
    write_ramp(&dir);
    let clip_muted = render(&project("ramp.wav", StretchMode::None, None, |c| c.muted = true, false), &dir, 512);
    let track_muted = render(&project("ramp.wav", StretchMode::None, None, |_| {}, true), &dir, 512);
    assert!(clip_muted.iter().chain(&track_muted).all(|s| *s == 0.0));
}

#[test]
fn renders_are_deterministic_for_any_block_size_and_workers() {
    let dir = temp_dir("audio-determinism");
    write_ramp(&dir);
    let project =
        project("ramp.wav", StretchMode::Resample, Some(90.0), |c| c.fades = Some(Fades { fade_in: Tick(100), fade_out: Tick(300) }), false);
    let reference = render(&project, &dir, 512);
    for block in [64, 333, 4096] {
        assert_eq!(render(&project, &dir, block), reference, "block {block}");
    }
    let options = OfflineOptions { end: Tick(3 * 3840), tail: Seconds(0.0), mode: PlayMode::Song, workers: 3, ..OfflineOptions::default() };
    let threaded = render_offline(&project, &SampleRoots::new(&dir), &options).unwrap().channels.swap_remove(0);
    assert_eq!(threaded, reference);
}

#[test]
fn seeking_into_a_clip_starts_mid_sample() {
    let dir = temp_dir("audio-seek");
    write_ramp(&dir);
    let project = project("ramp.wav", StretchMode::None, None, |c| c.len = Tick(3840), false);
    let (mut control, mut audio) = engine(&project, &SampleRoots::new(&dir), EngineConfig { max_frames: 256, ..EngineConfig::default() }).unwrap();
    control.set_mode(PlayMode::Song);
    // Bar 2, beat 2: half a second into the clip.
    control.seek(Tick(3840 + 960));
    control.play();
    let mut buffer = AudioBuffer::new(2, 256);
    audio.render(256, &mut buffer);
    let g = center();
    assert!((buffer.channel(0)[0] - ramp(24_000) * g).abs() < 1e-7, "{}", buffer.channel(0)[0]);
    // Stopping and playing again from the same place picks up where the clip is.
    control.stop();
    audio.render(256, &mut buffer);
    control.seek(Tick(3840 + 960));
    control.play();
    audio.render(256, &mut buffer);
    assert!((buffer.channel(0)[10] - ramp(24_010) * g).abs() < 1e-7);
}
