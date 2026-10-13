//! The sampler's settings (project.md section 10.2), through offline renders: the played part and start offset,
//! loops, slices and regions from the file's markers, declicking, the filter and the envelopes, precomputed
//! effects, resampling, stretching, the arpeggiator, layers and MIDI Out.

#![allow(clippy::unwrap_used, clippy::panic, reason = "tests")]

mod common;

use std::path::Path;
use std::sync::Arc;

use bodo_kit_audio::markers::{SampleMarkers, with_markers};
use bodo_kit_audio::{WavFormat, WavWriter};
use bodo_kit_core::{SampleRate, Seconds, Tick};
use bodo_kit_dsp::envelope::AdsrParams;
use bodo_mix::{OfflineOptions, render_offline};
use bodo_model::channel::{
    ArpDirection, Arpeggiator, ChannelKind, Declick, FilterKind, LoopMode, ModEnvelope, SampleEffects, SamplerFilter, SamplerParams, SamplerStretch,
    SamplerStretchMode,
};
use bodo_model::*;
use common::{Builder, temp_dir, write_impulse};
use imbl::Vector;

/// A note's level at the default velocity, mono and centered.
fn level() -> f32 {
    32_768.0 / 65_535.0 * core::f32::consts::FRAC_1_SQRT_2
}

fn write(dir: &Path, name: &str, rate: SampleRate, samples: &[f32]) {
    let mut writer = WavWriter::create(&dir.join(name), rate, 1, WavFormat::Float32).unwrap();
    writer.write(&[samples], samples.len()).unwrap();
    writer.finish().unwrap();
}

/// A 4800-frame ramp from 0 up to just below 1.
fn write_ramp(dir: &Path) {
    let ramp: Vec<f32> = (0..4800).map(|i| i as f32 / 4800.0).collect();
    write(dir, "ramp.wav", SampleRate::HZ_48000, &ramp);
}

fn render(project: &Project, dir: &Path, ticks: i64, block: usize) -> Vec<Vec<f32>> {
    let options = OfflineOptions { end: Tick(ticks), tail: Seconds(0.0), block, ..OfflineOptions::default() };
    render_offline(project, &SampleRoots::new(dir), &options).unwrap().channels
}

/// A project with one sampler of `file` playing `notes` `(key, start, length)` from tick 0.
fn project(file: &str, params: SamplerParams, notes: &[(u8, i64, i64)]) -> Project {
    let mut b = Builder::new();
    let sample = b.sample(file);
    let channel = b.sampler(sample, None, params);
    let notes: Vec<_> = notes.iter().map(|&(key, start, len)| (channel, key, start, len)).collect();
    let pattern = b.pattern(Some(7680), &notes);
    b.clip(pattern, 0, 7680, 0);
    b.build()
}

fn nonzero(channel: &[f32]) -> Vec<usize> {
    channel.iter().enumerate().filter(|(_, s)| s.abs() > 1e-7).map(|(i, _)| i).collect()
}

#[test]
fn a_note_plays_from_the_start_offset_to_the_end_marker() {
    let dir = temp_dir("sampler-region");
    write_ramp(&dir);
    let params = SamplerParams { start: 0.25, end: 0.75, start_offset: 0.5, ..SamplerParams::default() };
    let out = render(&project("ramp.wav", params, &[(60, 0, 3840)]), &dir, 3840, 512);
    // The played part is frames 1200 to 3600; half way in is frame 2400, the ramp's middle.
    assert!((out[0][0] - 0.5 * level()).abs() < 1e-5, "{}", out[0][0]);
    assert_eq!(nonzero(&out[0]).last(), Some(&1199), "it stops at the end marker");
}

#[test]
fn loops_sustain_the_note_forward_or_back_and_forth() {
    let dir = temp_dir("sampler-loops");
    write_ramp(&dir);
    for mode in [LoopMode::Forward, LoopMode::PingPong] {
        let params = SamplerParams { loop_mode: mode, loop_start: 0.5, loop_end: 0.75, one_shot: false, ..SamplerParams::default() };
        // A quarter note: 24 000 frames, five times the sample.
        let out = render(&project("ramp.wav", params, &[(60, 0, 960)]), &dir, 1920, 512);
        assert!(out[0][20_000].abs() > 0.4 * level(), "{mode:?} still sounds while the note is held");
        let looped = &out[0][4000..23_000];
        assert!(looped.iter().all(|s| (s / level()) >= 0.49 && (s / level()) < 0.76), "{mode:?} stays inside the loop");
        if mode == LoopMode::PingPong {
            // Going back down, the ramp falls.
            assert!(looped.windows(2).any(|w| w[1] < w[0] - 1e-6 && w[0] - w[1] < 0.01), "ping-pong plays backwards too");
        }
    }
}

#[test]
fn slices_and_regions_from_the_file_play_one_per_key() {
    let dir = temp_dir("sampler-slices");
    let ramp: Vec<f32> = (0..4800).map(|i| i as f32 / 4800.0).collect();
    write(&dir, "plain.wav", SampleRate::HZ_48000, &ramp);
    let bytes = std::fs::read(dir.join("plain.wav")).unwrap();
    let markers = SampleMarkers { cues: vec![1200, 2400], regions: vec![(3600, 600)], loops: Vec::new() };
    std::fs::write(dir.join("sliced.wav"), with_markers(&bytes, &markers)).unwrap();

    let slices = SamplerParams { load_slices: true, root_key: 36, ..SamplerParams::default() };
    // Key 37 is the second slice: frames 1200 to 2400, at its own pitch.
    let out = render(&project("sliced.wav", slices, &[(37, 0, 240)]), &dir, 960, 512);
    assert!((out[0][0] - 0.25 * level()).abs() < 1e-5, "{}", out[0][0]);
    assert_eq!(nonzero(&out[0]).last(), Some(&1199));

    let regions = SamplerParams { load_regions: true, root_key: 36, ..SamplerParams::default() };
    let out = render(&project("sliced.wav", regions, &[(36, 0, 240)]), &dir, 960, 512);
    assert!((out[0][0] - 0.75 * level()).abs() < 1e-5);
    assert_eq!(nonzero(&out[0]).last(), Some(&599), "the region is 600 frames long");
    // A key past the last region is silent.
    let regions = SamplerParams { load_regions: true, root_key: 36, ..SamplerParams::default() };
    assert!(nonzero(&render(&project("sliced.wav", regions, &[(40, 0, 240)]), &dir, 960, 512)[0]).is_empty());
}

#[test]
fn declicking_fades_the_end_of_a_cut_sample() {
    let dir = temp_dir("sampler-declick");
    write(&dir, "flat.wav", SampleRate::HZ_48000, &[0.5; 4800]);
    let plain = render(&project("flat.wav", SamplerParams { end: 0.5, ..SamplerParams::default() }, &[(60, 0, 240)]), &dir, 960, 512);
    let faded = render(
        &project("flat.wav", SamplerParams { end: 0.5, declick: Declick::Smooth, ..SamplerParams::default() }, &[(60, 0, 240)]),
        &dir,
        960,
        512,
    );
    assert!((plain[0][2399] - 0.5 * level()).abs() < 1e-5, "without declicking the last frame is full");
    assert!(faded[0][2399].abs() < 0.01 * level(), "with it the end fades to silence: {}", faded[0][2399]);
    assert!(faded[0][0].abs() < 1e-6, "and the start fades in");
    assert!((faded[0][1200] - 0.5 * level()).abs() < 1e-5, "the middle is untouched");
}

/// The energy of an alternating +-0.5 tone, the highest frequency there is, after the sampler's filter.
fn nyquist_energy(params: SamplerParams, range: std::ops::Range<usize>) -> f32 {
    let dir = temp_dir("sampler-filter");
    let tone: Vec<f32> = (0..48_000).map(|i| if i % 2 == 0 { 0.5 } else { -0.5 }).collect();
    write(&dir, "nyquist.wav", SampleRate::HZ_48000, &tone);
    let out = render(&project("nyquist.wav", params, &[(60, 0, 1920)]), &dir, 1920, 512);
    out[0][range].iter().map(|s| s * s).sum()
}

#[test]
fn the_filter_and_its_envelope_shape_the_tone() {
    let open = nyquist_energy(SamplerParams::default(), 1000..2000);
    let low = SamplerFilter { kind: FilterKind::LowPass, cutoff: 500.0, ..SamplerFilter::default() };
    let closed = nyquist_energy(SamplerParams { filter: low, ..SamplerParams::default() }, 1000..2000);
    assert!(closed < open * 1e-3, "a low pass at 500 Hz removes the top: {closed} vs {open}");
    let high = SamplerFilter { kind: FilterKind::HighPass, cutoff: 500.0, ..SamplerFilter::default() };
    assert!(nyquist_energy(SamplerParams { filter: high, ..SamplerParams::default() }, 1000..2000) > open * 0.9, "a high pass keeps it");
    // The envelope opens the filter five octaves at the start and lets it close.
    let envelope = ModEnvelope {
        enabled: true,
        adsr: AdsrParams { attack: Seconds(0.0), decay: Seconds(0.05), sustain: 0.0, release: Seconds(0.05) },
        amount: 5.5,
    };
    let swept = SamplerParams { filter: low, filter_envelope: envelope, ..SamplerParams::default() };
    let early = nyquist_energy(swept.clone(), 0..500);
    let late = nyquist_energy(swept, 9000..9500);
    assert!(early > late * 100.0, "open at first, closed later: {early} vs {late}");
}

#[test]
fn the_pitch_envelope_speeds_the_sample_up() {
    let dir = temp_dir("sampler-pitch-env");
    write_ramp(&dir);
    let envelope = ModEnvelope { enabled: true, adsr: AdsrParams { sustain: 1.0, ..AdsrParams::default() }, amount: 12.0 };
    let out = render(&project("ramp.wav", SamplerParams { pitch_envelope: envelope, ..SamplerParams::default() }, &[(60, 0, 960)]), &dir, 960, 512);
    // An octave up once the envelope's 2 ms attack is over: the 4800-frame sample ends after about 2450.
    let end = *nonzero(&out[0]).last().unwrap();
    assert!((2400..2470).contains(&end), "{end}");
}

#[test]
fn precomputed_effects_render_into_the_sample() {
    let dir = temp_dir("sampler-effects");
    write(&dir, "quiet.wav", SampleRate::HZ_48000, &[0.0, 0.0, 0.25, -0.125, 0.25, 0.0]);
    let effects = SampleEffects { normalize: true, reverse_polarity: true, trim: true, ..SampleEffects::default() };
    let out = render(&project("quiet.wav", SamplerParams { effects, ..SamplerParams::default() }, &[(60, 0, 240)]), &dir, 240, 512);
    // Trimmed to its loud part, normalized to full scale and inverted.
    assert_eq!(&out[0][..3].iter().map(|s| (s / level() * 1000.0).round() / 1000.0).collect::<Vec<_>>(), &[-1.0, 0.5, -1.0]);
    assert_eq!(nonzero(&out[0]).len(), 3);
    let reversed = render(&project("quiet.wav", SamplerParams { reverse: true, ..SamplerParams::default() }, &[(60, 0, 240)]), &dir, 240, 512);
    assert!((reversed[0][1] - 0.25 * level()).abs() < 1e-6, "reversed, the last loud frame comes second");
}

#[test]
fn a_sample_at_its_own_rate_plays_at_the_right_speed() {
    let dir = temp_dir("sampler-rate");
    // 2400 frames at 24 kHz: a tenth of a second, 4800 frames at 48 kHz either way.
    write(&dir, "half.wav", SampleRate::new(24_000).unwrap(), &vec![0.5; 2400]);
    let raw = render(&project("half.wav", SamplerParams { resample: false, ..SamplerParams::default() }, &[(60, 0, 240)]), &dir, 960, 512);
    let converted = render(&project("half.wav", SamplerParams::default(), &[(60, 0, 240)]), &dir, 960, 512);
    let len = |out: &[Vec<f32>]| nonzero(&out[0]).len() as i64;
    assert!((len(&raw) - 4800).abs() <= 2, "{}", len(&raw));
    assert!((len(&converted) - 4800).abs() <= 40, "{}", len(&converted));
}

#[test]
fn stretching_changes_the_length_in_either_mode() {
    let dir = temp_dir("sampler-stretch");
    let tone: Vec<f32> = (0..4800).map(|i| (i as f32 * 0.1).sin() * 0.5).collect();
    write(&dir, "tone.wav", SampleRate::HZ_48000, &tone);
    for mode in [SamplerStretchMode::Resample, SamplerStretchMode::Stretch] {
        let stretch = SamplerStretch { mode, multiplier: 2.0, ..SamplerStretch::default() };
        let out = render(&project("tone.wav", SamplerParams { stretch, ..SamplerParams::default() }, &[(60, 0, 960)]), &dir, 960, 512);
        let end = *nonzero(&out[0]).last().unwrap() as i64;
        assert!((end - 9600).abs() < 200, "{mode:?}: twice as long, {end}");
    }
}

#[test]
fn the_arpeggiator_plays_a_held_chord_one_note_a_step() {
    let dir = temp_dir("sampler-arp");
    write_impulse(&dir);
    let arp = Arpeggiator { enabled: true, direction: ArpDirection::Up, rate: 240, gate: 0.5, octaves: 1, repeat: 1 };
    let params = SamplerParams { arp, ..SamplerParams::default() };
    // Three keys held for a beat: four steps of a sixteenth, 6000 frames apart at 120 BPM.
    let out = render(&project("impulse.wav", params, &[(60, 0, 960), (64, 0, 960), (67, 0, 960)]), &dir, 1920, 256);
    assert_eq!(nonzero(&out[0]).iter().filter(|f| *f % 6000 == 0).count(), 4, "{:?}", nonzero(&out[0]));
    assert!(nonzero(&out[0]).iter().all(|f| *f < 24_000 + 10), "and stops with the chord");
}

#[test]
fn modulated_renders_are_the_same_at_any_block_size() {
    let dir = temp_dir("sampler-blocks");
    write_ramp(&dir);
    let params = SamplerParams {
        filter: SamplerFilter { kind: FilterKind::LowPass, cutoff: 2000.0, resonance: 0.4, key_tracking: 0.5 },
        filter_envelope: ModEnvelope { enabled: true, amount: 3.0, ..ModEnvelope::default() },
        pitch_envelope: ModEnvelope { enabled: true, amount: -5.0, ..ModEnvelope::default() },
        arp: Arpeggiator { enabled: true, rate: 120, ..Arpeggiator::default() },
        loop_mode: LoopMode::PingPong,
        loop_start: 0.2,
        loop_end: 0.6,
        one_shot: false,
        declick: Declick::Generic,
        ..SamplerParams::default()
    };
    let notes = [(60, 0, 1920), (67, 480, 960), (72, 960, 960)];
    let small = render(&project("ramp.wav", params.clone(), &notes), &dir, 3840, 64);
    let large = render(&project("ramp.wav", params, &notes), &dir, 3840, 1024);
    assert_eq!(small, large);
}

#[test]
fn a_layer_plays_every_child() {
    let dir = temp_dir("sampler-layer");
    write_impulse(&dir);
    let mut b = Builder::new();
    let sample = b.sample("impulse.wav");
    let one = b.sampler(sample, None, SamplerParams::default());
    let two = b.sampler(sample, None, SamplerParams::default());
    let layer = ChannelId::from_u128(b.id());
    let channel = Channel {
        id: layer,
        name: "Layer".into(),
        color: Color::GRAY,
        kind: ChannelKind::Layer { children: Vector::from(vec![one, two]) },
        target: None,
        volume: 1.0,
        pan: 0.0,
        pitch_cents: 0.0,
        swing: 0.0,
        muted: false,
        midi_in: None,
        group: None,
    };
    b.project.channels.insert(layer, Arc::new(channel));
    b.project.rack_order.push_back(layer);
    let pattern = b.pattern(Some(3840), &[(layer, 60, 0, 240)]);
    b.clip(pattern, 0, 3840, 0);
    let out = render(&b.build(), &dir, 960, 512);
    assert!((out[0][0] - 2.0 * level()).abs() < 1e-5, "both children: {}", out[0][0]);
}
