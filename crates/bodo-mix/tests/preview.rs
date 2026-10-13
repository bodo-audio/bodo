//! Previewing sounds (project.md section 14.1): the decoded file plays after the master at its own pitch, fades in,
//! hands over to the next sound or to silence with a fade out, and the audio side never allocates or frees.

#![allow(clippy::unwrap_used, clippy::panic, reason = "tests")]

mod common;

use std::time::{Duration, Instant};

use assert_no_alloc::{AllocDisabler, assert_no_alloc, reset_violation_count, violation_count};
use bodo_kit_audio::{WavFormat, WavWriter};
use bodo_kit_core::SampleRate;
use bodo_kit_dsp::AudioBuffer;
use bodo_mix::preview::{decode, preview};
use common::{temp_dir, write_tone};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

/// -3 dB, what previews play at.
const GAIN: f32 = 0.707_946;

#[test]
fn a_preview_plays_the_file_fading_in_then_hands_over_without_a_click() {
    let dir = temp_dir("preview");
    write_tone(&dir);
    let tone = decode(&dir.join("tone.wav"), SampleRate::HZ_48000, 240, 1).unwrap();
    assert_eq!(tone.frames(), 96_000, "two seconds at the file's own rate");

    let (mut preview, mut player) = preview(SampleRate::HZ_48000);
    let mut buffer = AudioBuffer::new(2, 512);
    buffer.set_frames(512);
    preview.play(&dir.join("tone.wav"));
    assert!(preview.current().is_some(), "loading counts as previewing");
    // Wait for the decoder, then let the player take the sound.
    let start = Instant::now();
    loop {
        assert!(preview.poll().is_none());
        for out in buffer.channels_mut() {
            out.fill(0.0);
        }
        player.mix_into(&mut buffer, 512);
        if buffer.channel(0).iter().any(|s| *s != 0.0) || start.elapsed() > Duration::from_secs(5) {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    // The first block fades in over 240 frames, then plays the file at -3 dB.
    let first = buffer.channel(0).to_vec();
    let left: Vec<f32> = (0..512).map(|i| ((i % 109) as f32 / 109.0) * 0.5 - 0.25).collect();
    assert!((first[0] - left[0] * GAIN / 240.0).abs() < 1e-6, "{} vs {}", first[0], left[0] * GAIN / 240.0);
    for i in 240..512 {
        assert!((first[i] - left[i] * GAIN).abs() < 1e-6, "frame {i}");
    }
    assert!(preview.current().is_some(), "playing");

    // Stopping fades out over 240 frames, then it is silent.
    preview.stop();
    for out in buffer.channels_mut() {
        out.fill(0.0);
    }
    player.mix_into(&mut buffer, 512);
    let out = buffer.channel(0);
    for (k, sample) in out[..240].iter().enumerate() {
        let next = ((512 + k) % 109) as f32 / 109.0 * 0.5 - 0.25;
        let expected = next * (1.0 - k as f32 / 240.0) * GAIN;
        assert!((sample - expected).abs() < 1e-6, "the sound fades out from where it was, frame {k}");
    }
    assert!(out[240..].iter().all(|s| *s == 0.0), "then silence");
    assert!(preview.current().is_none());
}

#[test]
fn the_audio_side_never_allocates_even_while_sounds_change() {
    let dir = temp_dir("preview-rt");
    write_tone(&dir);
    let mut impulse = vec![0.0f32; 4800];
    impulse[0] = 1.0;
    let mut writer = WavWriter::create(&dir.join("mono-44k.wav"), SampleRate::new(44_100).unwrap(), 1, WavFormat::Float32).unwrap();
    writer.write(&[&impulse], impulse.len()).unwrap();
    writer.finish().unwrap();

    let (mut preview, mut player) = preview(SampleRate::HZ_48000);
    let mut buffer = AudioBuffer::new(2, 256);
    buffer.set_frames(256);
    for (round, file) in ["tone.wav", "mono-44k.wav", "tone.wav"].iter().enumerate() {
        preview.play(&dir.join(file));
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            preview.poll();
            reset_violation_count();
            assert_no_alloc(|| player.mix_into(&mut buffer, 256));
            assert_eq!(violation_count(), 0, "round {round}");
            if preview.current().is_some_and(|_| buffer.channel(0).iter().any(|s| *s != 0.0)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    preview.stop();
    reset_violation_count();
    assert_no_alloc(|| player.mix_into(&mut buffer, 256));
    assert_eq!(violation_count(), 0, "taking the silence and handing the old sound back");
    assert!(preview.poll().is_none());
}

#[test]
fn a_mono_file_at_another_rate_plays_on_both_sides_at_the_engine_rate_and_bad_files_say_so() {
    let dir = temp_dir("preview-mono");
    let samples = vec![0.5f32; 44_100];
    let mut writer = WavWriter::create(&dir.join("dc.wav"), SampleRate::new(44_100).unwrap(), 1, WavFormat::Float32).unwrap();
    writer.write(&[&samples], samples.len()).unwrap();
    writer.finish().unwrap();
    let sound = decode(&dir.join("dc.wav"), SampleRate::HZ_48000, 0, 1).unwrap();
    assert!((sound.frames() as i64 - 48_000).abs() <= 2, "one second at 48 kHz: {}", sound.frames());

    std::fs::write(dir.join("broken.wav"), b"not audio").unwrap();
    let (mut preview, _player) = preview(SampleRate::HZ_48000);
    preview.play(&dir.join("broken.wav"));
    let start = Instant::now();
    let error = loop {
        if let Some(error) = preview.poll() {
            break error;
        }
        assert!(start.elapsed() < Duration::from_secs(5), "no error came back");
        std::thread::sleep(Duration::from_millis(2));
    };
    assert!(!error.is_empty());
    assert!(preview.current().is_none());
}
