//! Automation clips, pattern lanes, swing and the Song mode loop, through offline renders and the live engine.

#![allow(clippy::unwrap_used, clippy::panic, reason = "test helpers outside #[test] functions")]

mod common;

use std::sync::Arc;

use bodo_kit_core::{Seconds, Tick};
use bodo_kit_dsp::AudioBuffer;
use bodo_mix::{EngineConfig, OfflineOptions, PlayMode, engine, render_offline};
use bodo_model::automation::{AutoPoint, AutomationLane, ChannelParam, CurveShape, InsertParam, ParamTarget};
use bodo_model::channel::{ChannelKind, SamplerParams};
use bodo_model::playlist::{ClipOffset, ClipSource};
use bodo_model::*;
use common::{Builder, temp_dir, write_impulse, write_tone};
use imbl::Vector;

/// Frames per tick at 120 BPM and 48 kHz.
const FRAMES_PER_TICK: usize = 25;
/// The energy of a stretch of audio at the fader's silence floor, -144 dB: inaudible.
const SILENT: f32 = 1e-9;

fn lane(target: ParamTarget, points: &[(i64, f32, CurveShape)]) -> AutomationLane {
    AutomationLane {
        target,
        points: points.iter().map(|&(at, value, curve)| AutoPoint { at: Tick(at), value, curve }).collect::<Vector<_>>(),
        default: 0.8,
    }
}

/// Adds an automation channel holding `lane` and a clip of it from `start` lasting `len`, on the first track.
fn automation_clip(b: &mut Builder, lane: AutomationLane, start: i64, len: i64) {
    let id = ChannelId::from_u128(b.id());
    let channel = Channel {
        id,
        name: "Automation".into(),
        color: Color::GRAY,
        kind: ChannelKind::Automation(Arc::new(lane)),
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
    let track = *b.project.track_order.front().unwrap();
    let clip = Clip {
        id: ClipId::from_u128(b.id()),
        start: Tick(start),
        len: Tick(len),
        offset: ClipOffset::default(),
        muted: false,
        color: None,
        source: ClipSource::Automation(id),
        fades: None,
    };
    let mut value = (*b.project.tracks[&track]).clone();
    value.insert_clip(clip);
    b.project.tracks.insert(track, Arc::new(value));
}

fn render(project: &Project, dir: &std::path::Path, end: i64, block: usize) -> Vec<Vec<f32>> {
    let options = OfflineOptions { end: Tick(end), tail: Seconds(0.0), block, ..OfflineOptions::default() };
    render_offline(project, &SampleRoots::new(dir), &options).unwrap().channels
}

/// A tone on a channel into insert 1, one bar long, with or without a fader ride on insert 1 from 0 dB down to
/// silence over that bar.
fn ride(automated: bool) -> Project {
    let mut b = Builder::new();
    let sample = b.sample("tone.wav");
    let insert = b.insert(0);
    let channel = b.sampler(sample, Some(insert), SamplerParams { root_key: 60, ..SamplerParams::default() });
    let pattern = b.pattern(Some(3840), &[(channel, 60, 0, 3840)]);
    b.clip(pattern, 0, 3840, 0);
    if automated {
        let target = ParamTarget::Insert(insert, InsertParam::Volume);
        automation_clip(&mut b, lane(target, &[(0, 0.8, CurveShape::Linear), (3840, 0.0, CurveShape::Linear)]), 0, 3840);
    }
    b.build()
}

#[test]
fn a_fader_ride_sets_the_level_at_every_block_start() {
    let dir = temp_dir("ride");
    write_tone(&dir);
    let block = 512;
    let (dry, wet) = (render(&ride(false), &dir, 3840, block), render(&ride(true), &dir, 3840, block));
    let mut checked = 0;
    for k in 1..180 {
        // The gain ramps across a block and reaches the block's value at its last frame.
        let frame = k * block + block - 1;
        let tick = (k * block) as f64 / FRAMES_PER_TICK as f64;
        let normalized = 0.8 * (1.0 - tick / 3840.0);
        let expected = ParamTarget::Insert(InsertId::from_u128(0), InsertParam::Volume).value_of(normalized as f32);
        let gain = 10f32.powf(expected / 20.0);
        if dry[0][frame].abs() > 0.02 {
            let ratio = wet[0][frame] / dry[0][frame];
            assert!((ratio - gain).abs() < 1e-3, "block {k}: {ratio} vs {gain}");
            checked += 1;
        }
    }
    assert!(checked > 100, "{checked} blocks checked");
    assert_eq!(dry[0][..block], wet[0][..block], "the first block starts at 0 dB, the fader's own value");
}

/// A looping tone held for two bars into insert 1, and with `mute_until`, an automation clip that silences insert 1
/// from the start until that tick.
fn held_tone(mute_until: Option<i64>) -> Project {
    let mut b = Builder::new();
    let sample = b.sample("tone.wav");
    let insert = b.insert(0);
    let channel = b.sampler(
        sample,
        Some(insert),
        SamplerParams { root_key: 60, loop_mode: bodo_model::channel::LoopMode::Forward, ..SamplerParams::default() },
    );
    let pattern = b.pattern(Some(7680), &[(channel, 60, 0, 7680)]);
    b.clip(pattern, 0, 7680, 0);
    if let Some(end) = mute_until {
        let target = ParamTarget::Insert(insert, InsertParam::Volume);
        automation_clip(&mut b, lane(target, &[(0, 0.0, CurveShape::Hold)]), 0, end);
    }
    b.build()
}

fn energy(channel: &[f32], from: usize, to: usize) -> f32 {
    channel[from..to].iter().map(|s| s * s).sum()
}

#[test]
fn a_value_holds_after_its_clip_ends() {
    let dir = temp_dir("hold");
    write_tone(&dir);
    let out = render(&held_tone(Some(1920)), &dir, 7680, 512);
    let half_bar = 1920 * FRAMES_PER_TICK;
    assert!(energy(&out[0], 1024, half_bar) < SILENT, "silent while the clip plays");
    assert!(energy(&out[0], half_bar + 1024, 7680 * FRAMES_PER_TICK) < SILENT, "the last value holds after the clip");
    assert!(energy(&render(&held_tone(None), &dir, 7680, 512)[0], half_bar + 1024, 7680 * FRAMES_PER_TICK) > 1.0, "without it, the tone plays");
}

#[test]
fn stopping_returns_parameters_to_the_project() {
    let dir = temp_dir("reset");
    write_tone(&dir);
    // A clip that silences insert 1 for the first half bar of a looping two-bar tone.
    let project = held_tone(Some(1920));
    let (mut control, mut audio) = engine(&project, &SampleRoots::new(&dir), EngineConfig { max_frames: 256, ..EngineConfig::default() }).unwrap();
    let mut buffer = AudioBuffer::new(2, 256);
    control.play();
    for _ in 0..20 {
        audio.render(256, &mut buffer);
    }
    assert!(energy(buffer.channel(0), 0, 256) < SILENT, "the clip silences the insert");
    // Stop, then play again from the start in Pattern mode, where no automation clip plays: the fader is back.
    control.stop();
    audio.render(256, &mut buffer);
    let pattern = *project.patterns.keys().next().unwrap();
    control.set_mode(PlayMode::Pattern(pattern));
    control.seek(Tick(0));
    control.play();
    for _ in 0..4 {
        audio.render(256, &mut buffer);
    }
    assert!(energy(buffer.channel(0), 0, 256) > 1.0, "the project's 0 dB is back after the stop");
}

#[test]
fn pattern_lanes_repeat_with_the_pattern() {
    let dir = temp_dir("lanes");
    write_tone(&dir);
    let mut b = Builder::new();
    let sample = b.sample("tone.wav");
    let insert = b.insert(0);
    let channel = b.sampler(
        sample,
        Some(insert),
        SamplerParams { root_key: 60, loop_mode: bodo_model::channel::LoopMode::Forward, ..SamplerParams::default() },
    );
    // One bar, the note held through it; the channel's volume is full for half a bar, then silent.
    let pattern = b.pattern(Some(3840), &[(channel, 60, 0, 3840)]);
    let target = ParamTarget::Channel(channel, ChannelParam::Volume);
    let mut value = (*b.project.patterns[&pattern]).clone();
    value.automation.push_back(Arc::new(lane(target, &[(0, 0.8, CurveShape::Hold), (1920, 0.0, CurveShape::Hold)])));
    b.project.patterns.insert(pattern, Arc::new(value));
    b.clip(pattern, 0, 7680, 0);
    let project = b.build();
    let out = render(&project, &dir, 7680, 256);
    let at = |tick: usize| tick * FRAMES_PER_TICK;
    assert!(energy(&out[0], at(100), at(1800)) > 1.0, "first pass, first half: full");
    assert_eq!(energy(&out[0], at(1920) + 512, at(3840)), 0.0, "first pass, second half: silent");
    assert!(energy(&out[0], at(3840) + 512, at(5600)) > 1.0, "the lane starts over with the second pass");
    assert_eq!(energy(&out[0], at(5760) + 512, at(7680)), 0.0, "and silences its second half too");
}

/// Frames where `channel` isn't silent.
fn onsets(channel: &[f32]) -> Vec<usize> {
    channel.iter().enumerate().filter(|(_, s)| s.abs() > 1e-9).map(|(i, _)| i).collect()
}

#[test]
fn swing_delays_exactly_the_odd_steps() {
    let dir = temp_dir("swing");
    write_impulse(&dir);
    let make = |swing: f32| {
        let mut b = Builder::new();
        let sample = b.sample("impulse.wav");
        let channel = b.sampler(sample, None, SamplerParams { root_key: 60, ..SamplerParams::default() });
        let mut value = (*b.project.channels[&channel]).clone();
        value.swing = swing;
        b.project.channels.insert(channel, Arc::new(value));
        let pattern = b.pattern(Some(3840), &[(channel, 60, 0, 120), (channel, 60, 240, 120), (channel, 60, 480, 120), (channel, 60, 720, 120)]);
        b.clip(pattern, 0, 3840, 0);
        b.build()
    };
    let straight = onsets(&render(&make(0.0), &dir, 1920, 512)[0]);
    assert_eq!(straight, [0, 240 * FRAMES_PER_TICK, 480 * FRAMES_PER_TICK, 720 * FRAMES_PER_TICK]);
    let swung = onsets(&render(&make(1.0), &dir, 1920, 512)[0]);
    assert_eq!(swung, [0, 360 * FRAMES_PER_TICK, 480 * FRAMES_PER_TICK, 840 * FRAMES_PER_TICK], "odd steps move half a step");
    let half = onsets(&render(&make(0.5), &dir, 1920, 512)[0]);
    assert_eq!(half, [0, 300 * FRAMES_PER_TICK, 480 * FRAMES_PER_TICK, 780 * FRAMES_PER_TICK]);
}

#[test]
fn the_loop_wraps_exactly_at_its_end() {
    let dir = temp_dir("loop");
    write_impulse(&dir);
    let mut b = Builder::new();
    let sample = b.sample("impulse.wav");
    let channel = b.sampler(sample, None, SamplerParams { root_key: 60, ..SamplerParams::default() });
    // A hit on every beat of a bar, placed twice; the loop covers the first half bar.
    let pattern = b.pattern(Some(3840), &[(channel, 60, 0, 120), (channel, 60, 960, 120), (channel, 60, 1920, 120), (channel, 60, 2880, 120)]);
    b.clip(pattern, 0, 7680, 0);
    let project = b.build();
    let (mut control, mut audio) = engine(&project, &SampleRoots::new(&dir), EngineConfig { max_frames: 512, ..EngineConfig::default() }).unwrap();
    control.set_loop(Some((Tick(0), Tick(1920))));
    control.play();
    let mut buffer = AudioBuffer::new(2, 512);
    let mut out = Vec::new();
    // Three times round the loop (48,000 frames each), in blocks that don't divide it.
    for _ in 0..(3 * 48_000 / 500) {
        audio.render(500, &mut buffer);
        out.extend_from_slice(&buffer.channel(0)[..500]);
    }
    let beat = 960 * FRAMES_PER_TICK;
    assert_eq!(onsets(&out), [0, beat, 2 * beat, 3 * beat, 4 * beat, 5 * beat], "beats 1 and 2, three times; beat 3 never plays");
    // Without the loop the third beat plays.
    control.set_loop(None);
    control.stop();
    control.seek(Tick(0));
    control.play();
    let mut out = Vec::new();
    for _ in 0..(96_000 / 500) {
        audio.render(500, &mut buffer);
        out.extend_from_slice(&buffer.channel(0)[..500]);
    }
    assert_eq!(onsets(&out), [0, beat, 2 * beat, 3 * beat]);
    assert_eq!(audio.faults(), 0);
}

/// A tone into insert 1, which reaches the master only through its send to insert 2, with a Utility in slot 1 of
/// insert 1 that inverts polarity, and the automation `lanes` makes from insert 1, insert 2 and the Utility's instance.
fn slotted(lanes: impl FnOnce(InsertId, InsertId, PluginInstanceId) -> Vec<AutomationLane>) -> Project {
    use bodo_model::channel::{PluginFormat, PluginRef, SandboxMode};
    use bodo_model::mixer::ParamValue;
    let mut b = Builder::new();
    let sample = b.sample("tone.wav");
    let (one, two) = (b.insert(0), b.insert(1));
    let channel = b.sampler(sample, Some(one), SamplerParams { root_key: 60, ..SamplerParams::default() });
    let pattern = b.pattern(Some(3840), &[(channel, 60, 0, 3840)]);
    b.clip(pattern, 0, 3840, 0);
    b.route(one, two, 1.0);
    let instance = PluginInstanceId::from_u128(b.id());
    let mut mixer = (*b.project.mixer).clone();
    let strip = Arc::make_mut(mixer.inserts.get_mut(&one).unwrap());
    strip.slots[0].plugin = Some(PluginRef {
        instance,
        format: PluginFormat::Native,
        uid: "bodo.utility".into(),
        state: Arc::from(Vec::new()),
        sandbox: SandboxMode::InProcess,
    });
    strip.slots[0].params = vec![ParamValue { id: 3, value: 1.0 }];
    strip.to_master = false;
    b.project.mixer = Arc::new(mixer);
    for lane in lanes(one, two, instance) {
        automation_clip(&mut b, lane, 0, 3840);
    }
    b.build()
}

#[test]
fn slots_effect_parameters_and_sends_follow_automation() {
    use bodo_model::automation::SlotParam;
    let dir = temp_dir("slot-automation");
    write_tone(&dir);
    let hold = |target, value| lane(target, &[(0, value, CurveShape::Hold)]);
    // The second half of the bar, well after every ramp has settled.
    let level = |project: &Project| {
        let out = render(project, &dir, 3840, 256);
        energy(&out[0], 48_000, 90_000)
    };
    let plain = level(&slotted(|_, _, _| Vec::new()));
    assert!(plain > 1e-3, "the inverted tone plays: {plain}");
    // Half wet: the inverted and the dry signal cancel.
    let half = level(&slotted(|one, _, _| vec![hold(ParamTarget::Slot(one, 0, SlotParam::Mix), 0.5)]));
    assert!(half < plain * 1e-6, "{half} vs {plain}");
    // The slot switched off: the dry signal plays, as loud as the inverted one.
    let off = level(&slotted(|one, _, _| vec![hold(ParamTarget::Slot(one, 0, SlotParam::Enabled), 0.0)]));
    assert!((off / plain - 1.0).abs() < 1e-3, "{off} vs {plain}");
    // The Utility's gain at the bottom of its range, -60 dB: a thousandth of the amplitude, a millionth of the energy.
    let quiet = level(&slotted(|_, _, instance| vec![hold(ParamTarget::Plugin(instance, 0), 0.0)]));
    assert!((quiet / plain - 1e-6).abs() < 1e-7, "{quiet} vs {plain}");
    // The send to insert 2 at zero: nothing reaches the master.
    let unsent = level(&slotted(|one, two, _| vec![hold(ParamTarget::Route(RouteKey { from: RouteSource::Insert(one), to: two }), 0.0)]));
    assert!(unsent < plain * 1e-6, "{unsent} vs {plain}");
}
