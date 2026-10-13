//! The top bar's one-click audio recording and the memory in its performance readout (project.md section 14.1).

#![allow(clippy::unwrap_used, clippy::panic, reason = "tests")]

mod common;

use bodo_engine::PlayMode;
use egui::Theme;
use egui_kittest::kittest::Queryable;

#[test]
fn one_click_recording_arms_a_free_insert_and_records_into_the_playlist() {
    let mut h = common::harness(common::app(Theme::Dark), Theme::Dark, false);
    h.run_steps(3);
    common::click(&mut h, "One-click audio recording");
    h.run_steps(2);
    let transport = *h.state().session.transport();
    assert!(transport.recording && transport.mode == PlayMode::Song, "{transport:?}");
    let project = h.state().session.project();
    let armed: Vec<_> = project.mixer.order.iter().filter_map(|i| project.mixer.inserts.get(i)).filter(|i| i.armed).collect();
    assert_eq!(armed.len(), 1);
    let used: Vec<_> = project.channels.values().filter_map(|c| c.target).collect();
    assert!(!used.contains(&armed[0].id), "the insert nothing plays into: {}", armed[0].name);
    assert_eq!(armed[0].input.as_ref().map(|p| p.channels.clone()), Some(vec![0, 1]), "the first two inputs");
    assert_eq!(armed[0].record_tap, bodo_model::mixer::RecordTap::Input, "it records what comes in");
    common::click(&mut h, "One-click audio recording");
    h.run_steps(2);
    assert!(!h.state().session.transport().recording, "a second click stops");
}

#[test]
fn the_performance_readout_shows_the_memory_in_use() {
    let mut app = common::app(Theme::Dark);
    app.services.memory = Some(|| Some(256_000_000));
    let mut h = common::harness(app, Theme::Dark, false);
    h.run_steps(3);
    h.get_by_label_contains("256 MB");
}
