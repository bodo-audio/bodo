//! Checks that automation, swing and the loop never allocate on the audio side, including while the loop wraps and
//! a new song index with new lanes arrives. Debug builds only; the RealtimeSanitizer job covers every build.

#![cfg(debug_assertions)]
#![allow(clippy::unwrap_used, reason = "test helpers outside #[test] functions")]

mod common;

use std::sync::Arc;

use assert_no_alloc::{AllocDisabler, assert_no_alloc, reset_violation_count, violation_count};
use bodo_kit_core::Tick;
use bodo_kit_dsp::AudioBuffer;
use bodo_mix::{EngineConfig, engine};
use bodo_model::automation::{AutoPoint, AutomationLane, ChannelParam, CurveShape, InsertParam, ParamTarget};
use bodo_model::channel::{ChannelKind, SamplerParams};
use bodo_model::playlist::{ClipOffset, ClipSource};
use bodo_model::*;
use common::{Builder, temp_dir, write_tone};
use imbl::Vector;

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

fn lane(target: ParamTarget) -> AutomationLane {
    let points =
        [(0, 0.1, CurveShape::Exp(0.4)), (960, 0.9, CurveShape::SCurve(0.2)), (1920, 0.3, CurveShape::Sine), (3840, 0.7, CurveShape::Linear)];
    AutomationLane {
        target,
        points: points.iter().map(|&(at, value, curve)| AutoPoint { at: Tick(at), value, curve }).collect::<Vector<_>>(),
        default: 0.5,
    }
}

#[test]
fn automation_and_the_loop_do_not_allocate() {
    let dir = temp_dir("automation-rt");
    write_tone(&dir);
    let mut b = Builder::new();
    let sample = b.sample("tone.wav");
    let insert = b.insert(0);
    let channel = b.sampler(sample, Some(insert), SamplerParams::default());
    let mut swung = (*b.project.channels[&channel]).clone();
    swung.swing = 0.6;
    b.project.channels.insert(channel, Arc::new(swung));
    let pattern = b.pattern(Some(3840), &[(channel, 60, 0, 240), (channel, 64, 240, 240), (channel, 67, 720, 480)]);
    let mut value = (*b.project.patterns[&pattern]).clone();
    value.automation.push_back(Arc::new(lane(ParamTarget::Channel(channel, ChannelParam::Pan))));
    b.project.patterns.insert(pattern, Arc::new(value));
    b.clip(pattern, 0, 3840 * 4, 0);
    let auto = ChannelId::from_u128(b.id());
    b.project.channels.insert(
        auto,
        Arc::new(Channel {
            id: auto,
            name: "Ride".into(),
            color: Color::GRAY,
            kind: ChannelKind::Automation(Arc::new(lane(ParamTarget::Insert(insert, InsertParam::Volume)))),
            target: None,
            volume: 1.0,
            pan: 0.0,
            pitch_cents: 0.0,
            swing: 0.0,
            muted: false,
            midi_in: None,
        }),
    );
    b.project.rack_order.push_back(auto);
    let track = *b.project.track_order.front().unwrap();
    let mut value = (*b.project.tracks[&track]).clone();
    value.insert_clip(Clip {
        id: ClipId::from_u128(b.id()),
        start: Tick(0),
        len: Tick(7680),
        offset: ClipOffset::default(),
        muted: false,
        color: None,
        source: ClipSource::Automation(auto),
        fades: None,
    });
    b.project.tracks.insert(track, Arc::new(value));
    let project = b.build();

    let (mut control, mut audio) = engine(&project, &SampleRoots::new(&dir), EngineConfig { max_frames: 128, ..EngineConfig::default() }).unwrap();
    // A two-beat loop wraps every 24,000 frames: the renders below cross it several times.
    control.set_loop(Some((Tick(0), Tick(1920))));
    control.play();
    let mut buffer = AudioBuffer::new(2, 128);
    let mut render = |audio: &mut bodo_mix::AudioEngine| {
        reset_violation_count();
        assert_no_alloc(|| {
            for _ in 0..800 {
                audio.render(100, &mut buffer);
            }
        });
        assert_eq!(violation_count(), 0);
    };
    render(&mut audio);

    // New lanes arrive with a new song index while playing.
    let mut changed = project.clone();
    let mut value = (*changed.patterns[&pattern]).clone();
    value.automation.push_back(Arc::new(lane(ParamTarget::Channel(channel, ChannelParam::Volume))));
    changed.patterns.insert(pattern, Arc::new(value));
    control.set_project(&changed).unwrap();
    render(&mut audio);
    control.set_loop(None);
    control.stop();
    render(&mut audio);
    assert_eq!(audio.faults(), 0);
}
