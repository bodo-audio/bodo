//! Recording formats and loop recording (project.md section 9.6): takes in FLAC and Wave64 read back like any sample,
//! and with loop recording on, every pass of the Song loop or the pattern becomes its own take.

#![allow(clippy::unwrap_used, clippy::panic, reason = "tests")]

mod common;

use std::sync::Arc;

use bodo_kit_core::Tick;
use bodo_kit_dsp::AudioBuffer;
use bodo_mix::{Compensation, EngineConfig, PlayMode, Take, engine};
use bodo_model::mixer::{HwPort, RecordFormat, RecordTap};
use bodo_model::*;
use common::{Builder, temp_dir};

/// A project whose first insert records device inputs 1 and 2 at its input tap, in `format`.
fn armed(format: RecordFormat) -> (Project, InsertId) {
    let mut b = Builder::new();
    let insert = b.insert(0);
    let mut mixer = (*b.project.mixer).clone();
    let strip = Arc::make_mut(mixer.inserts.get_mut(&insert).unwrap());
    strip.armed = true;
    strip.input = Some(HwPort { channels: vec![0, 1] });
    strip.record_tap = RecordTap::Input;
    strip.record_format = format;
    b.project.mixer = Arc::new(mixer);
    (b.build(), insert)
}

/// Records `frames` frames of a steady input, with the transport set up by `setup`, and returns the takes.
fn record(project: &Project, dir: &std::path::Path, frames: usize, setup: impl FnOnce(&mut bodo_mix::Engine)) -> Vec<Take> {
    let (mut control, mut audio) = engine(project, &SampleRoots::new(dir), EngineConfig { max_frames: 512, ..EngineConfig::default() }).unwrap();
    setup(&mut control);
    assert_eq!(control.start_recording(project, dir, Compensation::default()).unwrap(), 1);
    control.record();
    let mut input = AudioBuffer::new(2, 512);
    input.channel_mut(0).fill(0.25);
    input.channel_mut(1).fill(-0.125);
    let mut output = AudioBuffer::new(2, 512);
    let mut done = 0;
    while done < frames {
        let n = 512.min(frames - done);
        audio.render_with_input(n, Some(&input), &mut output);
        done += n;
    }
    control.stop();
    control.stop_recording();
    (0..5000)
        .find_map(|_| {
            audio.render_with_input(512, Some(&input), &mut output);
            control.flush();
            std::thread::sleep(std::time::Duration::from_millis(1));
            control.finish_recording()
        })
        .unwrap()
        .unwrap()
}

#[test]
fn flac_and_wave64_takes_read_back() {
    for (format, extension) in [(RecordFormat::Flac, "flac"), (RecordFormat::W64, "w64"), (RecordFormat::Int24, "wav")] {
        let dir = temp_dir(&format!("record-{extension}"));
        let (project, _) = armed(format);
        let takes = record(&project, &dir, 10_000, |_| {});
        assert_eq!(takes.len(), 1);
        let take = &takes[0];
        assert_eq!(take.path.extension().and_then(|e| e.to_str()), Some(extension));
        let audio = bodo_kit_audio::decode_file(&take.path).unwrap();
        assert_eq!(audio.frames() as u64, take.frames);
        assert!(audio.frames() >= 10_000);
        let step = 1.0 / 8_000_000.0;
        assert!(audio.channels[0].iter().all(|s| (s - 0.25).abs() <= step), "{extension}");
        assert!(audio.channels[1].iter().all(|s| (s + 0.125).abs() <= step), "{extension}");
    }
}

#[test]
fn each_pass_of_the_song_loop_is_its_own_take() {
    let dir = temp_dir("record-loop");
    let (project, _) = armed(RecordFormat::Float32);
    // Loop the first beat, 24 000 frames, for two and a half passes.
    let takes = record(&project, &dir, 60_000, |control| {
        control.set_mode(PlayMode::Song);
        control.set_loop(Some((Tick(0), Tick(960))));
        control.set_loop_record(true);
    });
    let shape: Vec<(i64, u64)> = takes.iter().map(|t| (t.start_frame, t.frames)).collect();
    assert_eq!(shape.len(), 3, "{shape:?}");
    assert_eq!((shape[0], shape[1]), ((0, 24_000), (0, 24_000)), "every pass starts where the loop starts");
    assert_eq!(shape[2].0, 0);
    assert!(shape[2].1 >= 12_000, "the last pass runs until the stop: {shape:?}");
    let paths: std::collections::HashSet<_> = takes.iter().map(|t| t.path.clone()).collect();
    assert_eq!(paths.len(), 3, "a file per pass");

    // Without loop recording, one take runs across the passes.
    let dir = temp_dir("record-no-loop");
    let takes = record(&project, &dir, 60_000, |control| {
        control.set_mode(PlayMode::Song);
        control.set_loop(Some((Tick(0), Tick(960))));
    });
    assert_eq!(takes.len(), 1);
}

#[test]
fn each_pass_of_the_pattern_is_its_own_take_in_pattern_mode() {
    let dir = temp_dir("record-pattern-loop");
    let (mut project, _) = armed(RecordFormat::Float32);
    let id = PatternId::from_u128(77);
    let mut pattern = Pattern::new(id, "Loop", Color::GRAY);
    pattern.length = bodo_model::pattern::PatternLen::Fixed(Tick(960));
    project.patterns.insert(id, Arc::new(pattern));
    let takes = record(&project, &dir, 50_000, |control| {
        control.set_mode(PlayMode::Pattern(id));
        control.set_loop_record(true);
    });
    let shape: Vec<(i64, u64)> = takes.iter().map(|t| (t.start_frame, t.frames)).collect();
    assert_eq!(shape.len(), 3, "{shape:?}");
    assert_eq!((shape[0], shape[1]), ((0, 24_000), (0, 24_000)), "passes stack on the pattern's start");
}
