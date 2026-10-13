//! Snapshots of the main window, empty and with a beat, in the dark and light themes (project.md section 19.4). They
//! are drawn with the wgpu renderer and compared with the PNGs in `tests/snapshots` with a small tolerance; a platform
//! whose rasterizer differs more keeps its own baseline in `tests/snapshots/<os>/`.
//!
//! After an intended change, `UPDATE_SNAPSHOTS=1 cargo test -p bodo-ui --test layout` rewrites them.

#![allow(clippy::unwrap_used, clippy::panic, reason = "tests")]

mod common;

use egui::Theme;
use egui_kittest::{OsThreshold, SnapshotOptions};
use serde_json::Value;

fn options(name: &str) -> SnapshotOptions {
    let os_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots").join(std::env::consts::OS);
    let options = SnapshotOptions::new().threshold(OsThreshold::new(1.5).windows(2.0)).max_failed_pixels(OsThreshold::new(400).windows(2000));
    if os_dir.join(format!("{name}.png")).exists() { options.output_path(os_dir) } else { options }
}

/// The beat of the M2 exit criterion, through the session's commands.
fn with_beat(app: &mut bodo_ui::BodoApp) {
    let script: Value = serde_json::from_str(include_str!("../../bodo-engine/tests/fixtures/beat.json")).unwrap();
    let calls: Vec<bodo_engine::Call> = serde_json::from_value(script["calls"].clone()).unwrap();
    for call in calls {
        app.invoke(call).unwrap();
    }
    app.run("ui.select_pattern", serde_json::json!({ "pattern": "Verse" })).unwrap();
    app.run("ui.open_piano_roll", serde_json::json!({ "channel": "808" })).unwrap();
}

/// Draws the main window in `theme`, with the beat when `beat` is set, and compares it with snapshot `name`.
fn snapshot(name: &str, theme: Theme, beat: bool) {
    let mut app = common::app(theme);
    if beat {
        with_beat(&mut app);
    }
    let mut harness = common::harness(app, theme, true);
    harness.run_steps(6);
    harness.snapshot_options(name, &options(name));
}

#[test]
fn the_main_window_dark() {
    snapshot("main_dark", Theme::Dark, false);
}

#[test]
fn the_main_window_light() {
    snapshot("main_light", Theme::Light, false);
}

#[test]
fn a_beat_dark() {
    snapshot("beat_dark", Theme::Dark, true);
}

#[test]
fn a_beat_light() {
    snapshot("beat_light", Theme::Light, true);
}
