//! Tests for decoding, writing, resampling, peaks and streaming.

#![allow(clippy::unwrap_used, clippy::panic, reason = "test helpers outside #[test] functions")]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use bodo_kit_audio::peaks::ChannelPeaks;
use bodo_kit_audio::{AudioData, AudioError, Decoder, DiskThread, StreamConfig, StreamReader, WavFormat, WavWriter, decode_file, resample};
use bodo_kit_core::SampleRate;
use proptest::prelude::*;

/// A path in the system temp directory, unique to this process and test.
fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("bodo-kit-audio-{}-{name}", std::process::id()))
}

/// A stereo test signal: a 440 Hz sine on the left and a slower, quieter one on the right.
fn signal(frames: usize, rate: u32) -> Vec<Vec<f32>> {
    let sine = |freq: f32, gain: f32| (0..frames).map(|i| (i as f32 * freq * std::f32::consts::TAU / rate as f32).sin() * gain).collect();
    vec![sine(440.0, 0.8), sine(110.0, 0.5)]
}

fn write_wav(name: &str, channels: &[Vec<f32>], rate: u32, format: WavFormat) -> PathBuf {
    let path = temp_path(name);
    let mut writer = WavWriter::create(&path, SampleRate::new(rate).unwrap(), channels.len(), format).unwrap();
    let slices: Vec<&[f32]> = channels.iter().map(Vec::as_slice).collect();
    // Write in uneven blocks to exercise block boundaries.
    let frames = channels[0].len();
    let mut start = 0;
    while start < frames {
        let len = 1000.min(frames - start);
        let block: Vec<&[f32]> = slices.iter().map(|c| &c[start..start + len]).collect();
        writer.write(&block, len).unwrap();
        start += len;
    }
    writer.finish().unwrap();
    path
}

#[test]
fn wav_round_trips_in_every_format() {
    let original = signal(4800, 48_000);
    for (format, tolerance) in [(WavFormat::Float32, 0.0), (WavFormat::Int24, 1.0 / 8_388_608.0), (WavFormat::Int16, 1.0 / 32_768.0)] {
        let path = write_wav(&format!("roundtrip-{format:?}.wav"), &original, 48_000, format);
        let decoded = decode_file(&path).unwrap();
        assert_eq!(decoded.rate, SampleRate::HZ_48000);
        assert_eq!(decoded.channel_count(), 2);
        assert_eq!(decoded.frames(), 4800);
        for (a, b) in decoded.channels.iter().flatten().zip(original.iter().flatten()) {
            assert!((a - b).abs() <= tolerance, "{format:?}: {a} vs {b}");
        }
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn decoding_reports_missing_and_foreign_files() {
    assert!(matches!(decode_file(&temp_path("missing.wav")), Err(AudioError::Io { .. })));
    let path = temp_path("not-audio.txt");
    std::fs::write(&path, "hello, this is not audio").unwrap();
    assert!(matches!(decode_file(&path), Err(AudioError::Unsupported(_))));
    std::fs::remove_file(path).unwrap();
}

#[test]
fn seeking_is_sample_accurate() {
    let original = signal(20_000, 44_100);
    let path = write_wav("seek.wav", &original, 44_100, WavFormat::Float32);
    let mut decoder = Decoder::open(&path).unwrap();
    assert_eq!(decoder.info().frames, Some(20_000));
    for target in [12_345u64, 1, 19_999, 7_000] {
        assert_eq!(decoder.seek(target).unwrap(), target);
        let mut out = vec![Vec::new(), Vec::new()];
        decoder.decode_next(&mut out).unwrap().unwrap();
        assert_eq!(out[0][0], original[0][target as usize], "seek to {target}");
        assert_eq!(out[1][0], original[1][target as usize]);
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn resampling_keeps_length_and_pitch() {
    let audio = AudioData { rate: SampleRate::HZ_44100, channels: signal(44_100, 44_100) };
    let out = resample(audio, SampleRate::HZ_48000).unwrap();
    assert_eq!(out.rate, SampleRate::HZ_48000);
    assert!((out.frames() as i64 - 48_000).abs() <= 1, "{} frames", out.frames());
    // Compare the middle of the left channel with an ideal 440 Hz sine at 48 kHz. The FFT resampler may shift the
    // signal by a constant fraction of a sample, so allow the best alignment within one sample either way.
    let error_at = |shift: f32| {
        (10_000..38_000)
            .map(|i| (out.channels[0][i] - ((i as f32 + shift) * 440.0 * std::f32::consts::TAU / 48_000.0).sin() * 0.8).abs())
            .fold(0.0f32, f32::max)
    };
    let best = (-20..=20).map(|step| error_at(step as f32 / 20.0)).fold(f32::INFINITY, f32::min);
    assert!(best < 3e-3, "best error {best}");
    let same = AudioData { rate: SampleRate::HZ_48000, channels: vec![vec![0.5; 10]] };
    assert_eq!(resample(same.clone(), SampleRate::HZ_48000).unwrap(), same);
}

/// Reads `frames` frames from `reader` in blocks of `block`, waiting for the disk thread when it falls behind.
fn read_all(reader: &mut StreamReader, frames: usize, block: usize) -> Vec<Vec<f32>> {
    let mut out = vec![Vec::new(); 2];
    let deadline = Instant::now() + Duration::from_secs(20);
    let (mut left, mut right) = (vec![0.0f32; block], vec![0.0f32; block]);
    while out[0].len() < frames && !reader.at_end() {
        while !reader.is_ready() && !reader.at_end() {
            assert!(Instant::now() < deadline, "the disk thread never delivered");
            std::thread::sleep(Duration::from_millis(1));
        }
        let got = reader.read(&mut [&mut left, &mut right]);
        out[0].extend_from_slice(&left[..got]);
        out[1].extend_from_slice(&right[..got]);
    }
    out
}

#[test]
fn streaming_matches_decoding_and_seeks() {
    let original = signal(100_000, 48_000);
    let path = write_wav("stream.wav", &original, 48_000, WavFormat::Float32);
    let disk = DiskThread::spawn().unwrap();
    // Small chunks, so they circulate many times.
    let mut reader = disk.open(&path, StreamConfig { chunk_frames: 512, chunks: 8 }).unwrap();
    assert_eq!(reader.info().channels, 2);
    let streamed = read_all(&mut reader, 100_000, 300);
    assert_eq!(streamed, original);
    assert!(reader.at_end());

    reader.seek(37_000);
    let rest = read_all(&mut reader, 63_000, 1024);
    assert_eq!(rest[0], original[0][37_000..]);
    assert_eq!(rest[1], original[1][37_000..]);

    // Past the end, reads are silent.
    let (mut left, mut right) = (vec![1.0f32; 16], vec![1.0f32; 16]);
    assert_eq!(reader.read(&mut [&mut left, &mut right]), 0);
    assert_eq!(left, [0.0; 16]);
    drop(reader);
    std::fs::remove_file(path).unwrap();
}

proptest! {
    #[test]
    fn peak_ranges_match_brute_force(samples in prop::collection::vec(-1.0f32..1.0, 1..5000), a in 0usize..200, b in 0usize..200) {
        let peaks = ChannelPeaks::build(&samples);
        // Ranges aligned to the 32-frame blocks are exact.
        let blocks = samples.len().div_ceil(32);
        let (x, y) = (a % blocks, b % blocks);
        let (lo, hi) = (x.min(y), x.max(y) + 1);
        let (start, end) = (lo * 32, (hi * 32).min(samples.len()));
        let expected = samples[start..end].iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(l, h), &s| (l.min(s), h.max(s)));
        if start < end {
            prop_assert_eq!(peaks.range(start, end), Some(expected));
        }
    }
}
