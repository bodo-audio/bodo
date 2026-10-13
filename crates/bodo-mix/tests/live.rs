//! Notes played live, the metronome, count-in and MIDI recording, driven block by block through the audio side.

#![allow(clippy::unwrap_used, clippy::panic, reason = "test helpers outside #[test] functions")]

mod common;

use std::sync::Arc;

use bodo_kit_core::{SampleRate, TempoCurve, TempoMap, TempoPoint, Tick};
use bodo_kit_dsp::AudioBuffer;
use bodo_mix::{AudioEngine, Engine, EngineConfig, TakeTarget, engine, midi_record};
use bodo_model::channel::SamplerParams;
use bodo_model::{ChannelId, Project, SampleRoots};
use common::{Builder, temp_dir, write_tone};

const BLOCK: usize = 256;

/// A project with one sampler channel playing a two-second tone, and its engine.
fn tone_engine(name: &str, one_shot: bool) -> (Project, ChannelId, Engine, AudioEngine) {
    let dir = temp_dir(name);
    write_tone(&dir);
    let mut b = Builder::new();
    let sample = b.sample("tone.wav");
    let channel = b.sampler(sample, None, SamplerParams { root_key: 60, one_shot, ..SamplerParams::default() });
    b.pattern(Some(3840), &[]);
    let project = b.build();
    let (control, audio) = engine(&project, &SampleRoots::new(&dir), EngineConfig { max_frames: BLOCK, ..EngineConfig::default() }).unwrap();
    (project, channel, control, audio)
}

/// Renders `blocks` blocks and returns the left channel.
fn render(audio: &mut AudioEngine, blocks: usize) -> Vec<f32> {
    let mut buffer = AudioBuffer::new(2, BLOCK);
    let mut out = Vec::with_capacity(blocks * BLOCK);
    for _ in 0..blocks {
        audio.render(BLOCK, &mut buffer);
        out.extend_from_slice(&buffer.channel(0)[..BLOCK]);
    }
    out
}

fn energy(samples: &[f32]) -> f32 {
    samples.iter().map(|s| s * s).sum()
}

#[test]
fn live_notes_sound_at_the_next_block_and_stop_on_note_off() {
    let (_, channel, control, mut audio) = tone_engine("live", false);
    let live = control.live_input();
    assert_eq!(energy(&render(&mut audio, 2)), 0.0, "silent until a key goes down");
    live.note(channel, 60, 1.0, true);
    let held = render(&mut audio, 4);
    assert!(held[..32].iter().any(|s| *s != 0.0), "the note starts at the first frame of the next block");
    assert!(energy(&held[BLOCK * 3..]) > 0.0, "and holds while the key is down");
    live.note(channel, 60, 0.0, false);
    let released = render(&mut audio, 8);
    // The default release is 10 ms: 480 frames.
    assert!(energy(&released[..BLOCK]) > 0.0);
    assert!(energy(&released[BLOCK * 3..]) < 1e-9, "the note-off released it, down to an inaudible envelope tail");
    assert!(!control.status().playing, "the transport never moved");
}

#[test]
fn stopping_releases_live_one_shots() {
    let (_, channel, mut control, mut audio) = tone_engine("live-stop", true);
    control.live_input().note(channel, 60, 1.0, true);
    control.live_input().note(channel, 60, 0.0, false);
    let ringing = render(&mut audio, 4);
    assert!(energy(&ringing[BLOCK * 3..]) > 0.0, "a one-shot rings on after its note-off");
    control.stop();
    let stopped = render(&mut audio, 8);
    assert!(energy(&stopped[BLOCK * 3..]) < 1e-9, "stop releases it");
}

/// A project at 120 BPM that jumps to 90 BPM at bar 2, with no channels.
fn tempo_change() -> Project {
    let mut project = Builder::new().build();
    let points = vec![TempoPoint::new(Tick(0), 120.0, TempoCurve::Jump), TempoPoint::new(Tick(3840), 90.0, TempoCurve::Jump)];
    project.tempo = Arc::new(TempoMap::new(points).unwrap());
    project
}

/// Where each expected click starts, checked against the output: silence just before, sound just after.
fn assert_clicks_at(out: &[f32], frames: &[i64]) {
    for &frame in frames {
        let f = usize::try_from(frame).unwrap();
        assert_eq!(out[f - 1], 0.0, "nothing rings just before the beat at {frame}");
        assert_eq!(out[f], 0.0, "a click starts from zero at {frame}");
        assert!(out[f + 1] != 0.0, "a click starts at {frame}");
    }
}

#[test]
fn metronome_clicks_land_on_exact_beat_frames_through_a_tempo_change() {
    let project = tempo_change();
    let dir = temp_dir("metronome");
    let (mut control, mut audio) = engine(&project, &SampleRoots::new(&dir), EngineConfig { max_frames: BLOCK, ..EngineConfig::default() }).unwrap();
    control.set_metronome(true);
    control.play();
    let out = render(&mut audio, 48_000 * 5 / BLOCK);
    let beat_frames: Vec<i64> =
        (1..9).map(|beat| (project.tempo.seconds_at(Tick(beat * 960)).0 * f64::from(SampleRate::HZ_48000.hz())).round() as i64).collect();
    assert_eq!(beat_frames[3], 48_000 * 2, "bar 2 starts after four beats at 120 BPM");
    assert_eq!(beat_frames[4] - beat_frames[3], 32_000, "then beats are 2/3 s apart at 90 BPM");
    assert!(out[1] != 0.0, "the first downbeat clicks at frame 0");
    assert_clicks_at(&out, &beat_frames);
    let peak = |at: i64| out[usize::try_from(at).unwrap()..usize::try_from(at).unwrap() + 400].iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(peak(beat_frames[3]) > peak(beat_frames[2]) * 1.2, "the downbeat of bar 2 is the higher, louder click");
    // Between clicks, silence: the next beat after frame 0 is at 24000.
    assert_eq!(energy(&out[4_000..23_000]), 0.0);

    control.set_metronome(false);
    let quiet = render(&mut audio, 8);
    assert_eq!(energy(&quiet), 0.0, "off means no clicks");
}

#[test]
fn count_in_clicks_a_bar_before_the_playhead_moves() {
    let project = Builder::new().build();
    let dir = temp_dir("count-in");
    let (mut control, mut audio) = engine(&project, &SampleRoots::new(&dir), EngineConfig { max_frames: BLOCK, ..EngineConfig::default() }).unwrap();
    control.set_count_in(true);
    control.record();
    // One bar of 4/4 at 120 BPM is two seconds: 96000 frames, which isn't a whole number of these device buffers, so
    // the count-in ends inside one.
    const DEVICE: usize = 333;
    let mut buffer = AudioBuffer::new(2, DEVICE);
    let mut out = Vec::new();
    let mut step = |audio: &mut AudioEngine, out: &mut Vec<f32>| {
        audio.render(DEVICE, &mut buffer);
        out.extend_from_slice(&buffer.channel(0)[..DEVICE]);
    };
    while out.len() < 96_000 - DEVICE {
        step(&mut audio, &mut out);
        assert_eq!(control.status().frame, 0, "the playhead waits during the count-in");
        assert!(control.status().counting_in);
    }
    for _ in 0..4 {
        step(&mut audio, &mut out);
    }
    assert_clicks_at(&out, &[24_000, 48_000, 72_000]);
    assert!(out[1] != 0.0, "the count-in starts with a click");
    let status = control.status();
    assert!(!status.counting_in && status.playing && status.recording);
    let moved = usize::try_from(status.frame).unwrap();
    assert_eq!(moved, out.len() - 96_000, "the playhead started exactly when the bar of clicks ended");
}

#[test]
fn recorded_live_notes_become_notes_of_the_current_pattern() {
    let (project, channel, mut control, mut audio) = tone_engine("record", true);
    let pattern = project.patterns.keys().next().copied().unwrap_or_else(|| panic!("the builder makes no pattern"));
    let live = control.live_input();
    control.record();
    render(&mut audio, 2);
    // Key down in block 2 (frame 512), up in block 4 (frame 1024).
    live.note(channel, 64, 0.5, true);
    render(&mut audio, 2);
    live.note(channel, 64, 0.0, false);
    render(&mut audio, 2);
    assert!(control.take_midi_recording().is_none(), "the take isn't finished while recording");
    control.stop();
    render(&mut audio, 1);
    let take = control.take_midi_recording().unwrap();
    assert_eq!((take.start, take.end), (0, 1536));
    let frames: Vec<(i64, bool)> = take.notes.iter().map(|n| (n.frame, n.on)).collect();
    assert_eq!(frames, [(512, true), (1024, false)]);
    let notes = midi_record::place(&take, &project, SampleRate::HZ_48000, TakeTarget::Pattern(pattern));
    assert_eq!(notes.len(), 1);
    // 512 frames at 120 BPM and 48 kHz are 20.48 ticks.
    assert_eq!((notes[0].channel, notes[0].key, notes[0].start, notes[0].len), (channel, 64, Tick(20), Tick(21)));
}
