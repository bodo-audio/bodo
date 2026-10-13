//! End-to-end tests of the engine through offline renders.

#![allow(clippy::unwrap_used, clippy::panic, reason = "test helpers outside #[test] functions")]

mod common;

use std::path::Path;
use std::sync::Arc;

use bodo_kit_core::{Seconds, Tick};
use bodo_kit_dsp::AudioBuffer;
use bodo_mix::{EngineConfig, OfflineOptions, PlayMode, engine, render_offline};
use bodo_model::channel::SamplerParams;
use bodo_model::*;
use common::{Builder, temp_dir, write_impulse, write_tone};

fn render(project: &Project, dir: &Path, options: OfflineOptions) -> Vec<Vec<f32>> {
    render_offline(project, &SampleRoots::new(dir), &options).unwrap().channels
}

/// The level of a center-panned mono impulse at the default note velocity.
fn impulse_level() -> f32 {
    let velocity = 32_768.0 / 65_535.0;
    velocity * core::f32::consts::FRAC_1_SQRT_2
}

fn nonzero(channel: &[f32]) -> Vec<usize> {
    channel.iter().enumerate().filter(|(_, s)| s.abs() > 1e-9).map(|(i, _)| i).collect()
}

#[test]
fn notes_land_on_their_exact_frames() {
    let dir = temp_dir("frames");
    write_impulse(&dir);
    let mut b = Builder::new();
    let sample = b.sample("impulse.wav");
    let insert = b.insert(0);
    let channel = b.sampler(sample, Some(insert), SamplerParams::default());
    let pattern = b.pattern(Some(7680), &[(channel, 60, 0, 240), (channel, 60, 960, 240), (channel, 60, 1440, 240)]);
    b.clip(pattern, 0, 7680, 0);
    let project = b.build();
    let out = render(&project, &dir, OfflineOptions { end: Tick(7680), tail: Seconds(0.0), ..OfflineOptions::default() });
    // 120 BPM: a quarter note is half a second, 24 000 frames.
    assert_eq!(nonzero(&out[0]), [0, 24_000, 36_000]);
    assert!((out[0][24_000] - impulse_level()).abs() < 1e-6, "{}", out[0][24_000]);
    assert_eq!(out[0], out[1], "a centered mono sample is equal on both sides");
}

#[test]
fn pattern_mode_loops_and_long_clips_repeat() {
    let dir = temp_dir("loops");
    write_impulse(&dir);
    let mut b = Builder::new();
    let sample = b.sample("impulse.wav");
    let channel = b.sampler(sample, None, SamplerParams::default());
    // One note per one-bar pattern; a bar is 2 s, 96 000 frames.
    let pattern = b.pattern(Some(3840), &[(channel, 60, 0, 240)]);
    // A clip three bars long that skips the first half bar of the pattern.
    b.clip(pattern, 0, 3 * 3840, 1920);
    let project = b.build();
    let looped = render(
        &project,
        &dir,
        OfflineOptions { end: Tick(3 * 3840), tail: Seconds(0.0), mode: PlayMode::Pattern(pattern), ..OfflineOptions::default() },
    );
    assert_eq!(nonzero(&looped[0]), [0, 96_000, 192_000]);
    let song = render(&project, &dir, OfflineOptions { end: Tick(3 * 3840), tail: Seconds(0.0), ..OfflineOptions::default() });
    // The clip skips half a bar, so its repeats start at 1, 3 and 5 seconds.
    assert_eq!(nonzero(&song[0]), [48_000, 144_000, 240_000]);
}

#[test]
fn inserts_at_default_settings_are_transparent() {
    let dir = temp_dir("null");
    write_tone(&dir);
    let make = |through_insert: bool| {
        let mut b = Builder::new();
        let sample = b.sample("tone.wav");
        let target = through_insert.then(|| b.insert(3));
        let channel = b.sampler(sample, target, SamplerParams { root_key: 60, ..SamplerParams::default() });
        let pattern = b.pattern(Some(3840), &[(channel, 60, 0, 3840), (channel, 67, 960, 1920)]);
        b.clip(pattern, 0, 3840, 0);
        b.build()
    };
    let options = OfflineOptions { end: Tick(3840), ..OfflineOptions::default() };
    assert_eq!(render(&make(true), &dir, options), render(&make(false), &dir, options));
}

#[test]
fn sends_add_up_at_the_master() {
    let dir = temp_dir("sends");
    write_impulse(&dir);
    let mut b = Builder::new();
    let sample = b.sample("impulse.wav");
    let (one, two) = (b.insert(0), b.insert(1));
    let channel = b.sampler(sample, Some(one), SamplerParams::default());
    b.route(one, two, 0.5);
    let pattern = b.pattern(Some(3840), &[(channel, 60, 0, 240)]);
    b.clip(pattern, 0, 3840, 0);
    let project = b.build();
    let out = render(&project, &dir, OfflineOptions { end: Tick(960), tail: Seconds(0.0), ..OfflineOptions::default() });
    assert!((out[0][0] - 1.5 * impulse_level()).abs() < 1e-6, "{}", out[0][0]);
}

#[test]
fn renders_are_bit_identical_for_any_workers_and_block_size() {
    let dir = temp_dir("determinism");
    write_tone(&dir);
    let mut b = Builder::new();
    let sample = b.sample("tone.wav");
    let inserts: Vec<InsertId> = (0..4).map(|i| b.insert(i)).collect();
    let mut channels = Vec::new();
    for (i, &insert) in inserts.iter().enumerate() {
        let params = SamplerParams { loop_mode: bodo_model::channel::LoopMode::Forward, polyphony: 8, ..SamplerParams::default() };
        let channel = b.sampler(sample, Some(insert), params);
        let mut c = (*b.project.channels[&channel]).clone();
        c.pan = -0.6 + 0.4 * i as f32;
        b.project.channels.insert(channel, Arc::new(c));
        channels.push(channel);
    }
    b.route(inserts[0], inserts[2], 0.7);
    b.route(inserts[1], inserts[3], 0.3);
    let notes: Vec<(ChannelId, u8, i64, i64)> =
        (0..32).map(|i| (channels[i % 4], 48 + (i * 5 % 24) as u8, i as i64 * 240, 480 + (i as i64 % 3) * 240)).collect();
    let pattern = b.pattern(Some(7680), &notes);
    b.clip(pattern, 0, 15_360, 0);
    let project = b.build();
    let base = OfflineOptions { end: Tick(15_360), ..OfflineOptions::default() };
    let reference = render(&project, &dir, base);
    assert!(reference[0].iter().any(|s| s.abs() > 0.01), "the render is not silent");
    for (workers, block) in [(0, 512), (3, 512), (2, 64), (1, 1000)] {
        let again = render(&project, &dir, OfflineOptions { workers, block, ..base });
        let bits = |c: &[Vec<f32>]| c.iter().flatten().map(|s| s.to_bits()).collect::<Vec<_>>();
        assert!(bits(&again) == bits(&reference), "workers {workers}, block {block} differs");
    }
}

#[test]
fn fader_changes_apply_without_a_new_plan() {
    let dir = temp_dir("fader");
    write_tone(&dir);
    let mut b = Builder::new();
    let sample = b.sample("tone.wav");
    let insert = b.insert(0);
    let channel = b.sampler(sample, Some(insert), SamplerParams { loop_mode: bodo_model::channel::LoopMode::Forward, ..SamplerParams::default() });
    let pattern = b.pattern(Some(3840), &[(channel, 60, 0, 3840)]);
    b.clip(pattern, 0, 3840, 0);
    let project = b.build();
    let (mut control, mut audio) = engine(&project, &SampleRoots::new(&dir), EngineConfig { max_frames: 256, ..EngineConfig::default() }).unwrap();
    control.play();
    let mut buffer = AudioBuffer::new(2, 256);
    audio.render(256, &mut buffer);
    let before: f32 = buffer.channel(0).iter().map(|s| s.abs()).sum();
    let mut quieter = project.clone();
    let mut mixer = (*quieter.mixer).clone();
    let mut strip = (*mixer.inserts[&insert]).clone();
    strip.volume_db = -6.020_6;
    mixer.inserts.insert(insert, Arc::new(strip));
    quieter.mixer = Arc::new(mixer);
    control.set_project(&quieter).unwrap();
    audio.render(256, &mut buffer);
    audio.render(256, &mut buffer);
    let after: f32 = buffer.channel(0).iter().map(|s| s.abs()).sum();
    assert!((after / before - 0.5).abs() < 0.05, "{after} vs {before}");
    assert_eq!(control.status().frame, 768);
}
