//! Recording through a loopback (the M3 exit criterion): a sampler plays an impulse at bar 2, the master goes out of
//! a virtual device whose outputs come back into its inputs after its reported latency, and an armed insert records
//! that input. After latency compensation the recorded impulse must sit at bar 2 within 1 ms.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, reason = "test helpers outside #[test] functions")]

mod common;

use std::sync::Arc;

use bodo_kit_core::{Frames, SampleRate, Tick};
use bodo_kit_device::{Pacing, VirtualConfig, VirtualDevice};
use bodo_mix::{Compensation, EngineConfig, Take, engine};
use bodo_model::channel::SamplerParams;
use bodo_model::mixer::{HwPort, RecordTap};
use bodo_model::{InsertId, Project, SampleRoots};
use common::{Builder, temp_dir, write_impulse};

/// 1 ms at 48 kHz.
const ONE_MS: i64 = 48;
/// Bar 2 at 120 BPM: two seconds.
const BAR_2: i64 = 96_000;

/// A project whose sampler plays an impulse at bar 2 into the master, and whose insert 6 records device inputs 1 and
/// 2, muted so the recording never feeds back into the output.
fn project(dir: &std::path::Path) -> (Project, InsertId) {
    write_impulse(dir);
    let mut b = Builder::new();
    let sample = b.sample("impulse.wav");
    let channel = b.sampler(sample, None, SamplerParams { root_key: 60, ..SamplerParams::default() });
    let pattern = b.pattern(Some(3 * 3840), &[(channel, 60, 3840, 240)]);
    b.clip(pattern, 0, 3 * 3840, 0);
    let recorder = b.insert(5);
    let mut project = b.build();
    let mut mixer = (*project.mixer).clone();
    let mut insert = (*mixer.inserts[&recorder]).clone();
    insert.input = Some(HwPort { channels: vec![0, 1] });
    insert.armed = true;
    insert.record_tap = RecordTap::Input;
    insert.muted = true;
    mixer.inserts.insert(recorder, Arc::new(insert));
    project.mixer = Arc::new(mixer);
    project.validate().unwrap();
    (project, recorder)
}

/// Records three bars through a loopback whose delay is the reported `input` plus `output` latency, and returns the
/// take.
fn record(name: &str, input: u32, output: u32) -> (Take, Project) {
    let dir = temp_dir(name);
    let (project, _) = project(&dir);
    let (mut control, audio) = engine(&project, &SampleRoots::new(&dir), EngineConfig { max_frames: 128, ..EngineConfig::default() }).unwrap();
    let config = VirtualConfig {
        buffer_frames: 128,
        input_channels: 2,
        loopback: Some(input + output),
        input_latency: input,
        output_latency: output,
        pacing: Pacing::Freewheel,
        ..VirtualConfig::default()
    };
    let mut device = VirtualDevice::new(config, audio);
    let compensation = Compensation::from_status(&device.status());
    assert_eq!(compensation, Compensation { input: Frames(i64::from(input)), output: Frames(i64::from(output)) });
    assert_eq!(control.start_recording(&project, &dir.join("Recorded"), compensation).unwrap(), 1);
    control.seek(Tick::ZERO);
    control.play();
    let three_bars = 3 * BAR_2 / 2;
    while control.status().frame < three_bars {
        device.render();
    }
    control.stop();
    control.stop_recording();
    let takes = (0..5_000)
        .find_map(|_| {
            device.render();
            let done = control.finish_recording();
            if done.is_none() {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            done
        })
        .expect("the takes come back")
        .unwrap();
    assert_eq!(takes.len(), 1);
    (takes.into_iter().next().unwrap(), project)
}

/// The first frame of `samples` louder than -20 dBFS.
fn onset(samples: &[f32]) -> i64 {
    samples.iter().position(|s| s.abs() > 0.1).unwrap() as i64
}

#[test]
fn recorded_audio_lands_within_a_millisecond_after_latency_compensation() {
    for (input, output) in [(64u32, 64u32), (400, 600), (37, 251)] {
        let (take, project) = record(&format!("loop-{input}-{output}"), input, output);
        assert_eq!(take.path.file_name().and_then(|n| n.to_str()), Some("Insert 6_1.wav"));
        assert_eq!(take.overflows, 0);
        let audio = bodo_kit_audio::decode_file(&take.path).unwrap();
        assert_eq!(audio.channels.len(), 2);
        assert_eq!(audio.frames() as u64, take.frames);
        let heard = onset(&audio.channels[0]);

        // Where the impulse sits on the timeline once the take moves back by the round trip.
        let landed = take.timeline_frame() + heard;
        assert!((landed - BAR_2).abs() <= ONE_MS, "{input}+{output}: the impulse lands at frame {landed}, not {BAR_2}");
        // The loopback is exact and the sampler is sample accurate, so compensation leaves no error at all.
        assert_eq!(landed, BAR_2, "{input}+{output}: frame-exact placement");
        // Uncompensated, it would be late by the whole round trip.
        let raw = take.start_frame + heard;
        assert_eq!(raw - landed, i64::from(input + output));

        // The clip starts at a tick; rounding to ticks keeps it within the millisecond too.
        let rate = SampleRate::HZ_48000;
        let tick = take.timeline_tick(&project.tempo, rate);
        let tick_frame = |tick: i64| {
            let seconds = |t: i64| project.tempo.seconds_at(Tick(t)).0 * f64::from(rate.hz());
            if tick >= 0 { seconds(tick).round() as i64 } else { -(seconds(-tick).round() as i64) }
        };
        let placed = tick_frame(tick) + heard;
        assert!((placed - BAR_2).abs() <= ONE_MS, "{input}+{output}: the clip puts the impulse at {placed}, not {BAR_2}");
    }
}

#[test]
fn the_post_fader_tap_records_what_the_insert_plays() {
    let dir = temp_dir("post-fader");
    let (mut project, recorder) = project(&dir);
    // Now the insert isn't muted, records after its fader, and the fader is at -6 dB: the take is half as loud.
    let mut mixer = (*project.mixer).clone();
    let mut insert = (*mixer.inserts[&recorder]).clone();
    insert.muted = false;
    insert.to_master = false;
    insert.volume_db = -6.020_6;
    insert.record_tap = RecordTap::PostFader;
    mixer.inserts.insert(recorder, Arc::new(insert));
    project.mixer = Arc::new(mixer);
    let (mut control, audio) = engine(&project, &SampleRoots::new(&dir), EngineConfig { max_frames: 128, ..EngineConfig::default() }).unwrap();
    let config = VirtualConfig {
        buffer_frames: 128,
        input_channels: 2,
        loopback: Some(128),
        output_latency: 128,
        pacing: Pacing::Freewheel,
        ..VirtualConfig::default()
    };
    let mut device = VirtualDevice::new(config, audio);
    control.start_recording(&project, &dir.join("Recorded"), Compensation::from_status(&device.status())).unwrap();
    control.play();
    while control.status().frame < 120_000 {
        device.render();
    }
    control.stop();
    control.stop_recording();
    let take = (0..5_000)
        .find_map(|_| {
            device.render();
            std::thread::sleep(std::time::Duration::from_millis(1));
            control.finish_recording()
        })
        .unwrap()
        .unwrap()
        .remove(0);
    let audio = bodo_kit_audio::decode_file(&take.path).unwrap();
    let recorded = audio.channels[0].iter().fold(0.0f32, |m, s| m.max(s.abs()));
    let played = {
        let dry = bodo_kit_audio::decode_file(&dir.join("impulse.wav")).unwrap();
        dry.channels[0].iter().fold(0.0f32, |m, s| m.max(s.abs()))
    };
    assert!(recorded > 0.0 && recorded < played, "the take is quieter than the sample: {recorded} vs {played}");
}

#[test]
fn nothing_armed_means_nothing_to_record() {
    let dir = temp_dir("unarmed");
    let (mut project, recorder) = project(&dir);
    let mut mixer = (*project.mixer).clone();
    let mut insert = (*mixer.inserts[&recorder]).clone();
    insert.armed = false;
    mixer.inserts.insert(recorder, Arc::new(insert));
    project.mixer = Arc::new(mixer);
    let (mut control, _audio) = engine(&project, &SampleRoots::new(&dir), EngineConfig::default()).unwrap();
    let error = control.start_recording(&project, &dir.join("Recorded"), Compensation::default()).unwrap_err();
    assert!(error.to_string().contains("no insert is armed"), "{error}");
    assert!(!control.is_recording());
}
