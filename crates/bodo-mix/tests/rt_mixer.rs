//! The full mixer never allocates on the audio thread: every effect in slots, a sidechain, sends at every tap, a
//! solo, a direct output, meters and the scope, while effects swap into slots and plans with crossfades switch in.
//! Debug builds only; the RealtimeSanitizer job covers every build.

#![cfg(debug_assertions)]
#![allow(clippy::unwrap_used, reason = "tests")]

mod common;

use std::sync::Arc;

use assert_no_alloc::{AllocDisabler, assert_no_alloc, reset_violation_count, violation_count};
use bodo_kit_core::Frames;
use bodo_kit_dsp::AudioBuffer;
use bodo_mix::{EngineConfig, engine};
use bodo_model::channel::{LoopMode, PluginFormat, PluginRef, SamplerParams, SandboxMode};
use bodo_model::mixer::{HwPort, ParamValue, Route, RouteKind, SendTap};
use bodo_model::*;
use common::{Builder, temp_dir, write_tone};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

fn effect(instance: u128, uid: &str, params: &[(u32, f32)]) -> (Option<PluginRef>, Vec<ParamValue>) {
    let plugin = PluginRef {
        instance: PluginInstanceId::from_u128(instance),
        format: PluginFormat::Native,
        uid: uid.into(),
        state: Arc::from(Vec::new()),
        sandbox: SandboxMode::InProcess,
    };
    (Some(plugin), params.iter().map(|&(id, value)| ParamValue { id, value }).collect())
}

#[test]
fn the_full_mixer_does_not_allocate() {
    let dir = temp_dir("rt-mixer");
    write_tone(&dir);
    let mut b = Builder::new();
    let sample = b.sample("tone.wav");
    let (bass, kick, bus) = (b.insert(0), b.insert(1), b.insert(2));
    let held = SamplerParams { loop_mode: LoopMode::Forward, one_shot: false, ..SamplerParams::default() };
    let first = b.sampler(sample, Some(bass), held.clone());
    let second = b.sampler(sample, Some(kick), held);
    let pattern = b.pattern(Some(3840), &[(first, 60, 0, 3840), (second, 64, 960, 960)]);
    b.clip(pattern, 0, 3840 * 8, 0);
    let mut mixer = (*b.project.mixer).clone();
    for (from, to, tap, kind) in [
        (RouteSource::Insert(kick), bass, SendTap::PreFader, RouteKind::Sidechain),
        (RouteSource::Insert(bass), bus, SendTap::PreFx, RouteKind::Audio),
        (RouteSource::Channel(second), bus, SendTap::PostFader, RouteKind::Audio),
    ] {
        mixer.routes.insert(RouteKey { from, to }, Route { send_level: 0.5, tap, kind });
    }
    let strip = Arc::make_mut(mixer.inserts.get_mut(&bass).unwrap());
    for (slot, (instance, uid, params)) in [
        (0, (1u128, "bodo.utility", vec![(0, -3.0), (1, 0.2), (2, 150.0)])),
        (1, (2, "bodo.filter", vec![(1, 2000.0)])),
        (2, (3, "bodo.delay", vec![(2, 40.0), (3, 1.0)])),
        (3, (4, "bodo.ducker", vec![])),
        (4, (5, "bodo.latency-test", vec![(0, 64.0)])),
    ] {
        let (plugin, values) = effect(instance, uid, &params);
        strip.slots[slot].plugin = plugin;
        strip.slots[slot].params = values;
        strip.slots[slot].mix = 0.8;
    }
    strip.slots[3].sidechain = Some(RouteSource::Insert(kick));
    strip.eq.mid.gain_db = 4.0;
    strip.latency_offset = Frames(-32);
    let bus_strip = Arc::make_mut(mixer.inserts.get_mut(&bus).unwrap());
    bus_strip.output = Some(HwPort { channels: vec![2, 3] });
    bus_strip.solo = true;
    b.project.mixer = Arc::new(mixer);
    let project = b.build();

    let (mut control, mut audio) = engine(&project, &SampleRoots::new(&dir), EngineConfig { max_frames: 128, ..EngineConfig::default() }).unwrap();
    control.play();
    let mut buffer = AudioBuffer::new(4, 128);
    let mut render = |audio: &mut bodo_mix::AudioEngine| {
        reset_violation_count();
        assert_no_alloc(|| {
            for _ in 0..100 {
                audio.render(128, &mut buffer);
            }
        });
        assert_eq!(violation_count(), 0);
    };
    render(&mut audio);
    let mut scope = vec![0.0; 512];
    control.scope(&mut scope);
    assert!(scope.iter().any(|s| *s != 0.0));
    assert!(control.meter(bus).is_some_and(|m| m.peak[0] > 0.0));

    // An effect swaps into a slot without a recompile, and its old instance comes back to be dropped here.
    let mut swapped = project.clone();
    let mut mixer = (*swapped.mixer).clone();
    let (plugin, values) = effect(6, "bodo.filter", &[(0, 1.0)]);
    let strip = Arc::make_mut(mixer.inserts.get_mut(&bass).unwrap());
    strip.slots[1].plugin = plugin;
    strip.slots[1].params = values;
    swapped.mixer = Arc::new(mixer);
    control.set_project(&swapped).unwrap();
    render(&mut audio);
    control.flush();

    // A route goes and another comes: the plan switches with crossfades, then without the faded-out route.
    let mut rerouted = swapped.clone();
    let mut mixer = (*rerouted.mixer).clone();
    mixer.routes.remove(&RouteKey { from: RouteSource::Insert(bass), to: bus });
    mixer
        .routes
        .insert(RouteKey { from: RouteSource::Insert(kick), to: bus }, Route { send_level: 0.7, tap: SendTap::PostFader, kind: RouteKind::Audio });
    rerouted.mixer = Arc::new(mixer);
    control.set_project(&rerouted).unwrap();
    render(&mut audio);
    std::thread::sleep(std::time::Duration::from_millis(60));
    control.flush();
    render(&mut audio);
    control.flush();
    assert_eq!(audio.faults(), 0);
}
