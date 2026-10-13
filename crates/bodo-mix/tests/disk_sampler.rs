//! Samplers that keep their sample on disk (project.md section 10.2): a real-time engine streams the sample and
//! sounds the same as one that loads it, through loops too, without allocating on the audio side; offline renders
//! load it.

#![allow(clippy::unwrap_used, clippy::panic, reason = "tests")]

mod common;

use std::path::Path;

use assert_no_alloc::{AllocDisabler, assert_no_alloc, reset_violation_count, violation_count};
use bodo_kit_audio::{WavFormat, WavWriter};
use bodo_kit_core::{SampleRate, Seconds, Tick};
use bodo_kit_dsp::AudioBuffer;
use bodo_mix::{EngineConfig, OfflineOptions, engine, render_offline};
use bodo_model::channel::{LoopMode, SamplerParams};
use bodo_model::*;
use common::{Builder, temp_dir};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

/// Three seconds of a mono tone that changes all the time, so any misplaced frame shows.
fn write_long(dir: &Path) {
    let samples: Vec<f32> = (0..144_000).map(|i| (i as f32 * 0.013).sin() * 0.4 + (i as f32 * 0.0007).cos() * 0.3).collect();
    let mut writer = WavWriter::create(&dir.join("long.wav"), SampleRate::HZ_48000, 1, WavFormat::Float32).unwrap();
    writer.write(&[&samples], samples.len()).unwrap();
    writer.finish().unwrap();
}

/// One note at tick 0, two bars long, on a sampler playing long.wav with `params`.
fn project(params: SamplerParams) -> Project {
    let mut b = Builder::new();
    let sample = b.sample("long.wav");
    let channel = b.sampler(sample, None, params);
    let pattern = b.pattern(Some(7680), &[(channel, 62, 0, 7680)]);
    b.clip(pattern, 0, 7680, 0);
    b.build()
}

/// Renders `blocks` blocks of 256 frames in real time, near enough: the disk thread gets time to read ahead.
fn play(project: &Project, dir: &Path, blocks: usize, check_allocations: bool) -> Vec<f32> {
    let config = EngineConfig { workers: 0, max_frames: 256, realtime_buffer: Some(256) };
    let (mut control, mut audio) = engine(project, &SampleRoots::new(dir), config).unwrap();
    control.play();
    let mut buffer = AudioBuffer::new(2, 256);
    let mut out = Vec::with_capacity(blocks * 256);
    for _ in 0..blocks {
        reset_violation_count();
        if check_allocations {
            assert_no_alloc(|| audio.render(256, &mut buffer));
            assert_eq!(violation_count(), 0, "streaming allocated on the audio side");
        } else {
            audio.render(256, &mut buffer);
        }
        out.extend_from_slice(buffer.channel(0));
        std::thread::sleep(std::time::Duration::from_micros(700));
    }
    out
}

#[test]
fn a_streamed_sample_sounds_like_a_loaded_one() {
    let dir = temp_dir("disk-sampler");
    write_long(&dir);
    // Key 62 reads faster than the file's rate, so voices outrun their one-second heads and live on the stream.
    let params = SamplerParams { start_offset: 0.1, ..SamplerParams::default() };
    let loaded = play(&project(params.clone()), &dir, 400, false);
    let streamed = play(&project(SamplerParams { keep_on_disk: true, ..params }), &dir, 400, cfg!(debug_assertions));
    let first_difference = loaded.iter().zip(&streamed).position(|(a, b)| a != b);
    assert_eq!(first_difference, None, "the stream must give the same frames");
    assert!(loaded[60_000..61_000].iter().any(|s| s.abs() > 0.01), "and well past the head, the note still sounds");
}

#[test]
fn a_streamed_loop_jumps_back_to_its_head() {
    let dir = temp_dir("disk-sampler-loop");
    write_long(&dir);
    let params = SamplerParams { loop_mode: LoopMode::Forward, loop_start: 0.6, loop_end: 0.9, one_shot: false, ..SamplerParams::default() };
    let loaded = play(&project(params.clone()), &dir, 600, false);
    let streamed = play(&project(SamplerParams { keep_on_disk: true, ..params }), &dir, 600, false);
    assert_eq!(loaded.iter().zip(&streamed).position(|(a, b)| a != b), None);
}

#[test]
fn offline_renders_load_the_sample_whatever_it_keeps() {
    let dir = temp_dir("disk-sampler-offline");
    write_long(&dir);
    let render = |params: SamplerParams| {
        let options = OfflineOptions { end: Tick(3840), tail: Seconds(0.0), ..OfflineOptions::default() };
        render_offline(&project(params), &SampleRoots::new(&dir), &options).unwrap().channels
    };
    assert_eq!(render(SamplerParams::default()), render(SamplerParams { keep_on_disk: true, ..SamplerParams::default() }));
}
