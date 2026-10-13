//! Checks that the audio side of the engine never allocates, also while switching to new plans and song indexes.
//! Debug builds only; the RealtimeSanitizer job covers the workers and every build.

#![cfg(debug_assertions)]
#![allow(clippy::unwrap_used, reason = "test helpers outside #[test] functions")]

mod common;

use std::sync::Arc;

use assert_no_alloc::{AllocDisabler, assert_no_alloc, reset_violation_count, violation_count};
use bodo_kit_dsp::AudioBuffer;
use bodo_mix::{EngineConfig, engine};
use bodo_model::channel::{LoopMode, SamplerParams};
use bodo_model::mixer::{Route, RouteKind, SendTap};
use bodo_model::{RouteKey, RouteSource, SampleRoots};
use common::{Builder, temp_dir, write_tone};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

#[test]
fn the_audio_engine_does_not_allocate() {
    let dir = temp_dir("rt");
    write_tone(&dir);
    let mut b = Builder::new();
    let sample = b.sample("tone.wav");
    let (one, two) = (b.insert(0), b.insert(1));
    let channel = b.sampler(sample, Some(one), SamplerParams { loop_mode: LoopMode::Forward, ..SamplerParams::default() });
    let pattern = b.pattern(Some(3840), &[(channel, 60, 0, 1920), (channel, 64, 960, 1920), (channel, 67, 1920, 1920)]);
    b.clip(pattern, 0, 3840 * 4, 0);
    let project = b.build();
    let (mut control, mut audio) = engine(&project, &SampleRoots::new(&dir), EngineConfig { max_frames: 128, ..EngineConfig::default() }).unwrap();
    control.play();
    let mut buffer = AudioBuffer::new(2, 128);
    let mut render = |audio: &mut bodo_mix::AudioEngine| {
        reset_violation_count();
        assert_no_alloc(|| {
            for _ in 0..200 {
                audio.render(128, &mut buffer);
            }
        });
        assert_eq!(violation_count(), 0);
    };
    render(&mut audio);

    // A new route changes the graph, and a new note changes the song: both arrive while playing.
    let mut changed = project.clone();
    let mut mixer = (*changed.mixer).clone();
    mixer
        .routes
        .insert(RouteKey { from: RouteSource::Insert(one), to: two }, Route { send_level: 0.5, tap: SendTap::PostFader, kind: RouteKind::Audio });
    changed.mixer = Arc::new(mixer);
    let mut pattern_value = (*changed.patterns[&pattern]).clone();
    pattern_value.notes.entry(channel).or_default().insert(bodo_model::Note::new(
        bodo_model::NoteId(3),
        72,
        bodo_kit_core::Tick(480),
        bodo_kit_core::Tick(240),
    ));
    pattern_value.next_note_id = 4;
    changed.patterns.insert(pattern, Arc::new(pattern_value));
    control.set_project(&changed).unwrap();
    render(&mut audio);
    control.flush();
    control.stop();
    render(&mut audio);
    assert_eq!(audio.faults(), 0);
}
