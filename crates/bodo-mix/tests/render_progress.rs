//! Offline renders count their progress and stop when cancelled.

#![allow(clippy::unwrap_used, clippy::panic, reason = "tests")]

mod common;

use bodo_kit_core::Tick;
use bodo_mix::{MixError, OfflineOptions, RenderProgress, render_offline_with, render_stems_with};
use bodo_model::SampleRoots;
use bodo_model::channel::SamplerParams;
use common::{Builder, temp_dir, write_tone};

fn project(dir: &std::path::Path) -> bodo_model::Project {
    write_tone(dir);
    let mut b = Builder::new();
    let sample = b.sample("tone.wav");
    let channel = b.sampler(sample, None, SamplerParams::default());
    let pattern = b.pattern(Some(3840), &[(channel, 60, 0, 960)]);
    b.clip(pattern, 0, 3840 * 4, 0);
    b.build()
}

#[test]
fn a_finished_render_has_counted_every_frame() {
    let dir = temp_dir("progress-done");
    let project = project(&dir);
    let options = OfflineOptions { end: Tick(3840 * 4), ..OfflineOptions::default() };
    let progress = RenderProgress::default();
    assert_eq!(progress.fraction(), 0.0);
    render_offline_with(&project, &SampleRoots::new(&dir), &options, &progress).unwrap();
    assert_eq!(progress.fraction(), 1.0);
}

#[test]
fn a_cancelled_render_stops_with_an_error() {
    let dir = temp_dir("progress-cancel");
    let project = project(&dir);
    let options = OfflineOptions { end: Tick(3840 * 4), ..OfflineOptions::default() };
    let progress = RenderProgress::default();
    progress.cancel();
    assert!(matches!(render_offline_with(&project, &SampleRoots::new(&dir), &options, &progress), Err(MixError::Cancelled)));
    assert!(progress.fraction() < 0.05, "it stopped at the first block");
    let stems = RenderProgress::default();
    stems.cancel();
    assert!(matches!(render_stems_with(&project, &SampleRoots::new(&dir), &options, &stems), Err(MixError::Cancelled)));
}
