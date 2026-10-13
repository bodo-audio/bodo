//! Checks that reading and seeking a stream never allocates on the audio thread. Debug builds only.

#![cfg(debug_assertions)]
#![allow(clippy::unwrap_used, reason = "test helpers outside #[test] functions")]

use std::time::Duration;

use assert_no_alloc::{AllocDisabler, assert_no_alloc, reset_violation_count, violation_count};
use bodo_kit_audio::{DiskThread, StreamConfig, WavFormat, WavWriter};
use bodo_kit_core::SampleRate;

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

#[test]
fn stream_reads_and_seeks_do_not_allocate() {
    let path = std::env::temp_dir().join(format!("bodo-kit-audio-{}-rt.wav", std::process::id()));
    let mut writer = WavWriter::create(&path, SampleRate::HZ_48000, 2, WavFormat::Float32).unwrap();
    let ramp: Vec<f32> = (0..48_000).map(|i| i as f32 / 48_000.0).collect();
    writer.write(&[&ramp, &ramp], ramp.len()).unwrap();
    writer.finish().unwrap();

    let disk = DiskThread::spawn().unwrap();
    let mut reader = disk.open(&path, StreamConfig { chunk_frames: 1024, chunks: 8 }).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    let (mut left, mut right) = (vec![0.0f32; 256], vec![0.0f32; 256]);
    reset_violation_count();
    assert_no_alloc(|| {
        for block in 0..40 {
            reader.read(&mut [&mut left, &mut right]);
            if block == 20 {
                reader.seek(1000);
            }
        }
    });
    assert_eq!(violation_count(), 0);
    drop(reader);
    std::fs::remove_file(path).unwrap();
}
