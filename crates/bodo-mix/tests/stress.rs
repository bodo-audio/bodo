//! The M1 performance gate: 100 sampler voices at a 64-frame buffer and 48 kHz without a dropout.
//!
//! It runs the engine on the virtual device, paced like real hardware, and counts every callback that overran its
//! 1.33 ms deadline. Timing depends on the machine, so it runs by hand on reference machines, in release mode:
//! `cargo test -p bodo-mix --release --test stress -- --ignored --nocapture`.

#![allow(clippy::unwrap_used, reason = "test helpers outside #[test] functions")]

mod common;

use std::time::Duration;

use bodo_kit_core::SampleRate;
use bodo_kit_device::{Pacing, VirtualConfig, open_virtual};
use bodo_mix::{EngineConfig, engine};
use bodo_model::SampleRoots;
use bodo_model::channel::{LoopMode, SamplerParams};
use common::{Builder, temp_dir, write_tone};

const VOICES: usize = 100;
const CHANNELS: usize = 10;
const BUFFER: u32 = 64;

#[test]
#[ignore = "timing gate for reference machines; run in release by hand"]
fn hundred_voices_at_64_frames_without_dropouts() {
    let dir = temp_dir("stress");
    write_tone(&dir);
    let mut b = Builder::new();
    let sample = b.sample("tone.wav");
    let mut notes = Vec::new();
    for c in 0..CHANNELS {
        let insert = b.insert(c);
        let params = SamplerParams { loop_mode: LoopMode::Forward, polyphony: 16, ..SamplerParams::default() };
        let channel = b.sampler(sample, Some(insert), params);
        for v in 0..VOICES / CHANNELS {
            // Long notes at different pitches, all held for the whole run.
            notes.push((channel, 36 + (c * 3 + v * 5) as u8 % 60, 0, 3840 * 64));
        }
    }
    let pattern = b.pattern(Some(3840 * 64), &notes);
    b.clip(pattern, 0, 3840 * 64, 0);
    let project = b.build();

    let workers = std::env::var("BODO_STRESS_WORKERS")
        .ok()
        .and_then(|w| w.parse().ok())
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get().saturating_sub(1)).min(7));
    let config = EngineConfig { workers, max_frames: BUFFER as usize, realtime_buffer: Some(BUFFER) };
    let (mut control, audio) = engine(&project, &SampleRoots::new(&dir), config).unwrap();
    control.play();
    let device = VirtualConfig { sample_rate: SampleRate::HZ_48000, buffer_frames: BUFFER, pacing: Pacing::RealTime, ..VirtualConfig::default() };
    let stream = open_virtual(device, audio).unwrap();
    std::thread::sleep(Duration::from_secs(10));
    let status = stream.status();
    drop(stream);
    println!("{VOICES} voices, {BUFFER} frames at 48 kHz, {workers} workers: {status:?}");
    assert!(status.callbacks > 7000, "the device ran: {status:?}");
    assert_eq!(status.xruns, 0, "{status:?}");
    assert!(status.peak_load < 1.0, "{status:?}");
}
