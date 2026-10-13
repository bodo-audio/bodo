//! The full mixer (project.md section 9): the strip in order, FX slots, sends at their taps, sidechains, delay
//! compensation, solo, direct outputs, meters, and the M4 exit criteria: delay compensation nulls on random graphs
//! and stems sum to the mix.

#![allow(clippy::unwrap_used, clippy::panic, reason = "tests")]

mod common;

use std::sync::Arc;

use bodo_kit_core::{Frames, SampleRate, Seconds, Tick};
use bodo_kit_dsp::AudioBuffer;
use bodo_mix::{EngineConfig, OfflineOptions, PlayMode, engine, mix_latency, render_offline, render_stems};
use bodo_model::channel::{LoopMode, PluginFormat, PluginRef, SamplerParams, SandboxMode};
use bodo_model::mixer::{HwPort, MixerInsert, ParamValue, PdcBypass, Route, RouteKind, SendTap};
use bodo_model::*;
use common::{Builder, temp_dir, write_tone};
use proptest::prelude::*;

/// Writes a stereo 48 kHz sine at `frequency`, two seconds long.
fn write_sine(dir: &std::path::Path, name: &str, frequency: f32) {
    let samples: Vec<f32> = (0..96_000).map(|i| (2.0 * core::f32::consts::PI * frequency * i as f32 / 48_000.0).sin() * 0.5).collect();
    let mut writer = bodo_kit_audio::WavWriter::create(&dir.join(name), SampleRate::HZ_48000, 2, bodo_kit_audio::WavFormat::Float32).unwrap();
    writer.write(&[&samples, &samples], samples.len()).unwrap();
    writer.finish().unwrap();
}

/// A looping tone channel holding one note through the first two bars, into `target`.
fn tone(b: &mut Builder, sample: SampleId, target: Option<InsertId>) -> ChannelId {
    let channel =
        b.sampler(sample, target, SamplerParams { root_key: 60, loop_mode: LoopMode::Forward, one_shot: false, ..SamplerParams::default() });
    let pattern = b.pattern(Some(7680), &[(channel, 60, 0, 7680)]);
    b.clip(pattern, 0, 7680, 0);
    channel
}

fn edit(b: &mut Builder, id: InsertId, f: impl FnOnce(&mut MixerInsert)) {
    let mut mixer = (*b.project.mixer).clone();
    let insert = if mixer.master.id == id { &mut mixer.master } else { mixer.inserts.get_mut(&id).unwrap() };
    f(Arc::make_mut(insert));
    b.project.mixer = Arc::new(mixer);
}

/// Loads native effect `uid` into `slot` of `insert`, with parameters `params` by id.
fn load(b: &mut Builder, insert: InsertId, slot: usize, uid: &str, params: &[(u32, f32)]) -> PluginInstanceId {
    let instance = PluginInstanceId::from_u128(b.id());
    edit(b, insert, |i| {
        i.slots[slot].plugin = Some(PluginRef {
            instance,
            format: PluginFormat::Native,
            uid: uid.into(),
            state: Arc::from(Vec::new()),
            sandbox: SandboxMode::InProcess,
        });
        i.slots[slot].params = params.iter().map(|&(id, value)| ParamValue { id, value }).collect();
    });
    instance
}

fn route(b: &mut Builder, from: RouteSource, to: InsertId, level: f32, tap: SendTap, kind: RouteKind) {
    let mut mixer = (*b.project.mixer).clone();
    mixer.routes.insert(RouteKey { from, to }, Route { send_level: level, tap, kind });
    b.project.mixer = Arc::new(mixer);
}

fn options(end: i64) -> OfflineOptions {
    OfflineOptions { end: Tick(end), tail: Seconds(0.05), block: 256, ..OfflineOptions::default() }
}

fn render(project: &Project, dir: &std::path::Path, end: i64) -> Vec<Vec<f32>> {
    render_offline(project, &SampleRoots::new(dir), &options(end)).unwrap().channels
}

fn rms(samples: &[f32]) -> f32 {
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len().max(1) as f32).sqrt()
}

#[test]
fn an_insert_at_its_defaults_is_bit_exact() {
    let dir = temp_dir("mixer-defaults");
    write_tone(&dir);
    let direct = {
        let mut b = Builder::new();
        let sample = b.sample("tone.wav");
        tone(&mut b, sample, None);
        b.build()
    };
    let through = {
        let mut b = Builder::new();
        let sample = b.sample("tone.wav");
        let insert = b.insert(3);
        tone(&mut b, sample, Some(insert));
        // A flat EQ at other frequencies, and empty slots, change nothing.
        edit(&mut b, insert, |i| {
            i.eq.mid.frequency = 2500.0;
            i.eq.low.q = 1.3;
        });
        b.build()
    };
    assert_eq!(render(&direct, &dir, 1920), render(&through, &dir, 1920));
}

#[test]
fn the_eq_boosts_its_band_and_leaves_the_rest() {
    let dir = temp_dir("mixer-eq");
    write_sine(&dir, "sine.wav", 1000.0);
    let level = |gain: f32| {
        let mut b = Builder::new();
        let sample = b.sample("sine.wav");
        let insert = b.insert(0);
        tone(&mut b, sample, Some(insert));
        edit(&mut b, insert, |i| {
            i.eq.mid = bodo_model::mixer::EqBand { frequency: 1000.0, gain_db: gain, q: 1.0 };
        });
        let out = render(&b.build(), &dir, 1920);
        rms(&out[0][24_000..48_000])
    };
    let ratio = 20.0 * (level(6.0) / level(0.0)).log10();
    assert!((ratio - 6.0).abs() < 0.1, "{ratio} dB");
}

#[test]
fn a_slot_effect_runs_and_partial_mixes_never_comb_filter() {
    let dir = temp_dir("mixer-slots");
    write_tone(&dir);
    let make = |mix: f32, enabled: bool| {
        let mut b = Builder::new();
        let sample = b.sample("tone.wav");
        let insert = b.insert(0);
        tone(&mut b, sample, Some(insert));
        load(&mut b, insert, 2, "bodo.latency-test", &[(0, 300.0)]);
        edit(&mut b, insert, |i| {
            i.slots[2].mix = mix;
            i.slots[2].enabled = enabled;
        });
        b.build()
    };
    let full = render(&make(1.0, true), &dir, 1920);
    // The dry path is delayed by the slot's latency, so half dry and half wet is the same signal.
    assert_eq!(render(&make(0.5, true), &dir, 1920), full);
    // A disabled slot passes the signal through, still delayed, so timing never jumps.
    assert_eq!(render(&make(1.0, false), &dir, 1920), full);
    let reference = {
        let mut b = Builder::new();
        let sample = b.sample("tone.wav");
        tone(&mut b, sample, None);
        render(&b.build(), &dir, 1920)
    };
    assert_eq!(&full[0][300..10_000], &reference[0][..9700], "300 frames late");
    assert_eq!(mix_latency(&make(1.0, true), false).unwrap().total, 300);

    let gained = {
        let mut b = Builder::new();
        let sample = b.sample("tone.wav");
        let insert = b.insert(0);
        tone(&mut b, sample, Some(insert));
        load(&mut b, insert, 0, "bodo.utility", &[(0, -6.0206)]);
        render(&b.build(), &dir, 1920)
    };
    let ratio = rms(&gained[0][4800..48_000]) / rms(&reference[0][4800..48_000]);
    assert!((ratio - 0.5).abs() < 1e-3, "{ratio}");
}

#[test]
fn track_latency_moves_a_track_or_everything_else() {
    let dir = temp_dir("mixer-latency");
    common::write_impulse(&dir);
    let make = |a: i64, b_offset: i64| {
        let mut b = Builder::new();
        let sample = b.sample("impulse.wav");
        let (one, two) = (b.insert(0), b.insert(1));
        let first = b.sampler(sample, Some(one), SamplerParams::default());
        let second = b.sampler(sample, Some(two), SamplerParams::default());
        let pattern = b.pattern(Some(3840), &[(first, 60, 0, 240), (second, 60, 480, 240)]);
        b.clip(pattern, 0, 3840, 0);
        edit(&mut b, one, |i| i.latency_offset = Frames(a));
        edit(&mut b, two, |i| i.latency_offset = Frames(b_offset));
        b.build()
    };
    let hits = |out: &[Vec<f32>]| out[0].iter().enumerate().filter(|(_, s)| s.abs() > 0.01).map(|(i, _)| i).collect::<Vec<_>>();
    let plain = hits(&render(&make(0, 0), &dir, 1920));
    assert_eq!(plain, [0, 12_000]);
    // A negative offset delays its own track; compensation doesn't see it.
    assert_eq!(hits(&render(&make(-100, 0), &dir, 1920)), [100, 12_000]);
    assert_eq!(mix_latency(&make(-100, 0), false).unwrap().total, 0);
    // A positive offset declares latency the track doesn't have: everything else waits 100 frames for it, as if a
    // plugin on it delayed it that much, so the track itself comes 100 frames earlier than the rest.
    assert_eq!(hits(&render(&make(100, 0), &dir, 1920)), [0, 12_100]);
    assert_eq!(mix_latency(&make(100, 0), false).unwrap().total, 100);
}

#[test]
fn delay_compensation_bypass_lets_a_track_through_early() {
    let dir = temp_dir("mixer-bypass");
    common::write_impulse(&dir);
    let make = |bypass: PdcBypass| {
        let mut b = Builder::new();
        let sample = b.sample("impulse.wav");
        let (slow, live) = (b.insert(0), b.insert(1));
        let first = b.sampler(sample, Some(slow), SamplerParams::default());
        let second = b.sampler(sample, Some(live), SamplerParams::default());
        let pattern = b.pattern(Some(3840), &[(first, 60, 0, 240), (second, 60, 0, 240)]);
        b.clip(pattern, 0, 3840, 0);
        load(&mut b, slow, 0, "bodo.latency-test", &[(0, 256.0)]);
        edit(&mut b, live, |i| i.pdc_bypass = bypass);
        b.build()
    };
    let hits = |out: &[Vec<f32>]| out[0].iter().enumerate().filter(|(_, s)| s.abs() > 0.01).map(|(i, _)| i).collect::<Vec<_>>();
    assert_eq!(hits(&render(&make(PdcBypass::Never), &dir, 960)), [256], "both lined up, on top of each other");
    assert_eq!(hits(&render(&make(PdcBypass::Always), &dir, 960)), [0, 256], "the bypassed track arrives undelayed");
    assert_eq!(
        mix_latency(&make(PdcBypass::WhileRecording), true).unwrap().inserts.iter().find(|(_, l)| l.own == 256).map(|(_, l)| l.output),
        Some(256)
    );
}

#[test]
fn sends_leave_at_their_tap() {
    let dir = temp_dir("mixer-taps");
    write_tone(&dir);
    let level = |tap: SendTap| {
        let mut b = Builder::new();
        let sample = b.sample("tone.wav");
        let (source, bus) = (b.insert(0), b.insert(1));
        tone(&mut b, sample, Some(source));
        // The source is muted and doesn't reach the master; only its send can be heard.
        edit(&mut b, source, |i| {
            i.muted = true;
            i.to_master = false;
            i.volume_db = -6.0;
        });
        route(&mut b, RouteSource::Insert(source), bus, 1.0, tap, RouteKind::Audio);
        rms(&render(&b.build(), &dir, 1920)[0][24_000..48_000])
    };
    let (pre_fx, pre_fader, post_fader) = (level(SendTap::PreFx), level(SendTap::PreFader), level(SendTap::PostFader));
    assert!(pre_fx > 0.01 && (pre_fx - pre_fader).abs() < 1e-6, "{pre_fx} {pre_fader}");
    assert!(post_fader < 1e-6, "a post-fader send of a muted insert is silent: {post_fader}");
}

#[test]
fn a_sidechain_ducks_a_slot_and_lines_up_with_it() {
    let dir = temp_dir("mixer-sidechain");
    write_tone(&dir);
    common::write_impulse(&dir);
    let make = |ducker: bool| {
        let mut b = Builder::new();
        let tone_sample = b.sample("tone.wav");
        let kick_sample = b.sample("impulse.wav");
        let (bass, kick) = (b.insert(0), b.insert(1));
        let held = SamplerParams { root_key: 60, loop_mode: LoopMode::Forward, one_shot: false, ..SamplerParams::default() };
        let bass_channel = b.sampler(tone_sample, Some(bass), held.clone());
        // The trigger holds from beat 2 on, frame 24 000.
        let trigger = b.sampler(tone_sample, Some(kick), held);
        let pattern = b.pattern(Some(7680), &[(bass_channel, 60, 0, 7680), (trigger, 60, 960, 2880)]);
        b.clip(pattern, 0, 7680, 0);
        let _ = kick_sample;
        load(&mut b, bass, 0, "bodo.latency-test", &[(0, 200.0)]);
        if ducker {
            load(&mut b, bass, 1, "bodo.ducker", &[(0, 48.0), (1, -60.0), (2, 0.1), (3, 300.0)]);
            route(&mut b, RouteSource::Insert(kick), bass, 1.0, SendTap::PreFader, RouteKind::Sidechain);
            edit(&mut b, bass, |i| i.slots[1].sidechain = Some(RouteSource::Insert(kick)));
        }
        edit(&mut b, kick, |i| i.to_master = false);
        b.build()
    };
    let (plain, ducked) = (render(&make(false), &dir, 3840), render(&make(true), &dir, 3840));
    // The trigger starts at frame 24 000; the ducker sees it lined up with the 200 frames of latency before it, so
    // the bass ducks from 24 200 in the output, and not a frame earlier.
    let window = |out: &Vec<Vec<f32>>, at: usize| rms(&out[0][at..at + 2400]);
    assert!(
        window(&ducked, 26_000) < 0.05 * window(&plain, 26_000),
        "ducked while the trigger plays: {} vs {}",
        window(&ducked, 26_000),
        window(&plain, 26_000)
    );
    assert!((window(&ducked, 21_000) - window(&plain, 21_000)).abs() < 1e-4, "untouched before the trigger");
    let (before, after) =
        (ducked[0][24_190..24_199].iter().map(|s| s.abs()).sum::<f32>(), plain[0][24_190..24_199].iter().map(|s| s.abs()).sum::<f32>());
    assert!((before - after).abs() < 1e-5, "the ducking starts with the trigger's sound, after the bass's 200 frames of latency");
}

#[test]
fn solo_keeps_the_soloed_insert_its_feeders_and_the_master() {
    let dir = temp_dir("mixer-solo");
    write_tone(&dir);
    let mut b = Builder::new();
    let sample = b.sample("tone.wav");
    let (lead, drums, bus) = (b.insert(0), b.insert(1), b.insert(2));
    tone(&mut b, sample, Some(lead));
    tone(&mut b, sample, Some(drums));
    route(&mut b, RouteSource::Insert(lead), bus, 1.0, SendTap::PostFader, RouteKind::Audio);
    edit(&mut b, lead, |i| i.to_master = false);
    edit(&mut b, bus, |i| i.solo = true);
    let project = b.build();
    let audible = bodo_mix::audible_set(&project).unwrap();
    assert!(audible.contains(&lead) && audible.contains(&bus) && audible.contains(&project.mixer.master.id));
    assert!(!audible.contains(&drums));
    // Only the lead, through the bus, is heard.
    let out = render(&project, &dir, 1920);
    let reference = {
        let mut b = Builder::new();
        let sample = b.sample("tone.wav");
        tone(&mut b, sample, None);
        render(&b.build(), &dir, 1920)
    };
    let tail = 9600..48_000;
    let difference: f32 = out[0][tail.clone()].iter().zip(&reference[0][tail]).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
    assert!(difference < 1e-6, "{difference}");
}

#[test]
fn direct_outputs_meters_and_the_scope() {
    let dir = temp_dir("mixer-outputs");
    write_tone(&dir);
    let mut b = Builder::new();
    let sample = b.sample("tone.wav");
    let insert = b.insert(0);
    tone(&mut b, sample, Some(insert));
    edit(&mut b, insert, |i| {
        i.output = Some(HwPort { channels: vec![2, 3] });
        i.to_master = false;
    });
    load(&mut b, insert, 0, "bodo.utility", &[(0, 24.0)]);
    let project = b.build();
    let (mut control, mut audio) = engine(&project, &SampleRoots::new(&dir), EngineConfig { max_frames: 256, ..EngineConfig::default() }).unwrap();
    control.play();
    let mut output = AudioBuffer::new(4, 256);
    for _ in 0..40 {
        audio.render(256, &mut output);
    }
    assert!(output.channel(0).iter().all(|s| *s == 0.0), "nothing reaches the master");
    assert!(rms(output.channel(2)) > 0.1 && rms(output.channel(3)) > 0.1, "the insert plays on outputs 3 and 4");
    let reading = control.meter(insert).unwrap();
    assert!(reading.peak[0] > 1.0 && reading.clipped, "+24 dB clips: {reading:?}");
    assert_eq!(control.meter(insert).unwrap().peak, [0.0, 0.0], "a read clears the peaks");
    control.clear_clip(insert);
    assert!(!control.meter(insert).unwrap().clipped);
    let mut scope = [1.0f32; 64];
    control.scope(&mut scope);
    assert!(scope.iter().all(|s| *s == 0.0), "the master is silent");
    let _ = control.insert_cpu(insert);
    for _ in 0..20 {
        audio.render(256, &mut output);
    }
    assert!(control.insert_cpu(insert) >= 0.0);
}

#[test]
fn effects_swap_into_a_running_engine_without_a_recompile() {
    let dir = temp_dir("mixer-swap");
    write_tone(&dir);
    let mut b = Builder::new();
    let sample = b.sample("tone.wav");
    let insert = b.insert(0);
    tone(&mut b, sample, Some(insert));
    let before = b.project.clone();
    load(&mut b, insert, 4, "bodo.utility", &[(0, -60.0)]);
    let after = b.build();
    let (mut control, mut audio) = engine(&before, &SampleRoots::new(&dir), EngineConfig { max_frames: 256, ..EngineConfig::default() }).unwrap();
    control.play();
    let mut output = AudioBuffer::new(2, 256);
    for _ in 0..20 {
        audio.render(256, &mut output);
    }
    let loud = rms(output.channel(0));
    control.set_project(&after).unwrap();
    for _ in 0..20 {
        audio.render(256, &mut output);
        control.flush();
    }
    let quiet = rms(output.channel(0));
    assert!(quiet < loud * 0.01, "{loud} then {quiet}");
}

#[test]
fn the_running_engine_reports_latency_per_insert_and_in_total() {
    let dir = temp_dir("mixer-latency");
    write_tone(&dir);
    let mut b = Builder::new();
    let sample = b.sample("tone.wav");
    let (one, two) = (b.insert(0), b.insert(1));
    tone(&mut b, sample, Some(one));
    load(&mut b, one, 2, "bodo.latency-test", &[(0, 300.0)]);
    edit(&mut b, two, |i| i.latency_offset = Frames(-50));
    route(&mut b, RouteSource::Insert(one), two, 1.0, SendTap::PostFader, RouteKind::Audio);
    let project = b.build();
    let (mut control, mut audio) = engine(&project, &SampleRoots::new(&dir), EngineConfig { max_frames: 256, ..EngineConfig::default() }).unwrap();
    let mut output = AudioBuffer::new(2, 256);
    audio.render(256, &mut output);
    control.flush();
    let first = control.insert_latency(one).unwrap();
    assert_eq!((first.input, first.own, first.output), (0, 300, 300));
    // Insert 2 waits for insert 1. Its negative track latency delays it 50 frames more, on purpose: compensation
    // doesn't see it, so nothing else waits for it.
    let second = control.insert_latency(two).unwrap();
    assert_eq!((second.input, second.own, second.output), (300, 0, 300));
    assert_eq!(control.total_latency(), 300);
    assert_eq!(control.status().latency, 300);
    assert_eq!(mix_latency(&project, false).unwrap().total, 300, "the same answer without an engine");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// M4 exit criterion: delay compensation nulls on random graphs. Random inserts, routes at random taps, and slots
    /// with random latencies play exactly like the same graph without latency, only later by the total latency.
    #[test]
    fn delay_compensation_nulls_on_random_graphs(
        inserts in 2usize..6,
        routes in prop::collection::vec((0usize..6, 0usize..6, 0u8..3, 0.2f32..1.0), 0..8),
        latencies in prop::collection::vec((0usize..6, 0usize..10, 0u32..400), 0..8),
        targets in prop::collection::vec(0usize..6, 1..3),
        off_master in prop::collection::vec(0usize..6, 0..2),
    ) {
        let dir = temp_dir("mixer-pdc");
        write_tone(&dir);
        let make = |with_latency: bool| {
            let mut b = Builder::new();
            let sample = b.sample("tone.wav");
            for &target in &targets {
                let insert = b.insert(target % inserts);
                tone(&mut b, sample, Some(insert));
            }
            for &(from, to, tap, level) in &routes {
                let (from, to) = (from % inserts, to % inserts);
                if from < to {
                    let tap = [SendTap::PreFx, SendTap::PreFader, SendTap::PostFader][tap as usize];
                    let (from, to) = (b.insert(from), b.insert(to));
                    route(&mut b, RouteSource::Insert(from), to, level, tap, RouteKind::Audio);
                }
            }
            for &(insert, slot, latency) in &latencies {
                let id = b.insert(insert % inserts);
                load(&mut b, id, slot, "bodo.latency-test", &[(0, if with_latency { latency as f32 } else { 0.0 })]);
            }
            for &insert in &off_master {
                let id = b.insert(insert % inserts);
                edit(&mut b, id, |i| i.to_master = false);
            }
            b.build()
        };
        let (with, without) = (make(true), make(false));
        let total = mix_latency(&with, false).unwrap().total as usize;
        prop_assert_eq!(mix_latency(&without, false).unwrap().total, 0);
        let (late, reference) = (render(&with, &dir, 960), render(&without, &dir, 960));
        for side in 0..2 {
            let n = reference[side].len() - total;
            let shifted = &late[side][total..total + n];
            prop_assert!(late[side][..total].iter().all(|s| *s == 0.0), "silence before the latency");
            let worst = shifted.iter().zip(&reference[side][..n]).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
            prop_assert!(worst == 0.0 || 20.0 * worst.log10() <= -100.0, "nulls to {} dBFS", 20.0 * worst.log10());
        }
    }
}

#[test]
fn stems_sum_to_the_mix() {
    let dir = temp_dir("mixer-stems");
    write_tone(&dir);
    common::write_impulse(&dir);
    let mut b = Builder::new();
    let tone_sample = b.sample("tone.wav");
    let kick_sample = b.sample("impulse.wav");
    let (lead, drums, bus, fx) = (b.insert(0), b.insert(1), b.insert(2), b.insert(3));
    tone(&mut b, tone_sample, Some(lead));
    tone(&mut b, tone_sample, None);
    let kick = b.sampler(kick_sample, Some(drums), SamplerParams::default());
    let pattern = b.pattern(Some(3840), &[(kick, 60, 0, 240), (kick, 60, 1920, 240)]);
    b.clip(pattern, 0, 3840, 0);
    load(&mut b, lead, 0, "bodo.filter", &[(1, 800.0)]);
    load(&mut b, lead, 1, "bodo.latency-test", &[(0, 128.0)]);
    load(&mut b, fx, 0, "bodo.delay", &[(2, 100.0)]);
    load(&mut b, drums, 0, "bodo.utility", &[(0, 3.0), (1, -0.4)]);
    route(&mut b, RouteSource::Insert(lead), bus, 0.7, SendTap::PostFader, RouteKind::Audio);
    route(&mut b, RouteSource::Insert(drums), fx, 0.5, SendTap::PreFader, RouteKind::Audio);
    edit(&mut b, drums, |i| i.pan = 0.3);
    let project = b.build();
    let roots = SampleRoots::new(&dir);
    let options = options(3840);
    let mix = render_offline(&project, &roots, &options).unwrap();
    let stems = render_stems(&project, &roots, &options).unwrap();
    let names: Vec<&str> = stems.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(stems.len(), 5, "four inserts and the channel playing straight into the master: {names:?}");
    for side in 0..2 {
        let mut worst = 0.0f32;
        for (frame, mixed) in mix.channels[side].iter().enumerate() {
            let sum: f32 = stems.iter().map(|s| s.audio.channels[side][frame]).sum();
            worst = worst.max((sum - mixed).abs());
        }
        assert!(worst == 0.0 || 20.0 * worst.log10() <= -100.0, "stems null against the mix to {} dBFS", 20.0 * worst.log10());
    }
    let _ = PlayMode::Song;
}
