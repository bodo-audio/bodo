//! Snapshots of the main window, empty, with a beat, and with the piano roll in front, in the dark and light themes
//! (project.md section 19.4). They
//! are drawn with the wgpu renderer and compared with the PNGs in `tests/snapshots` with a small tolerance; a platform
//! whose rasterizer differs more keeps its own baseline in `tests/snapshots/<os>/`.
//!
//! After an intended change, `UPDATE_SNAPSHOTS=1 cargo test -p bodo-ui --test layout` rewrites them.

#![allow(clippy::unwrap_used, clippy::panic, reason = "tests")]

mod common;

use bodo_kit_music::{PitchClass, Scale, ScaleKind};
use bodo_ui::BodoApp;
use bodo_ui::windows::WindowKind;
use egui::Theme;
use egui_kittest::{OsThreshold, SnapshotOptions};
use serde_json::{Value, json};

fn options(name: &str) -> SnapshotOptions {
    let os_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots").join(std::env::consts::OS);
    let options = SnapshotOptions::new().threshold(OsThreshold::new(1.5).windows(2.0)).max_failed_pixels(OsThreshold::new(400).windows(2000));
    if os_dir.join(format!("{name}.png")).exists() { options.output_path(os_dir) } else { options }
}

/// The beat of the M2 exit criterion, through the session's commands.
fn with_beat(app: &mut BodoApp) {
    let script: Value = serde_json::from_str(include_str!("../../bodo-engine/tests/fixtures/beat.json")).unwrap();
    let calls: Vec<bodo_engine::Call> = serde_json::from_value(script["calls"].clone()).unwrap();
    for call in calls {
        app.invoke(call).unwrap();
    }
    app.run("ui.select_pattern", serde_json::json!({ "pattern": "Verse" })).unwrap();
    app.run("ui.open_piano_roll", serde_json::json!({ "channel": "808" })).unwrap();
}

/// The beat with the piano roll filling the workspace: A minor highlighted, a slide note, and a selected A minor
/// chord, which the readout names.
fn with_piano(app: &mut BodoApp) {
    with_beat(app);
    let mut state = app.prefs.layout.state(WindowKind::PianoRoll);
    state.maximized = true;
    app.prefs.layout.windows.insert(WindowKind::PianoRoll, state);
    app.view.piano.scale = Some(Scale::new(PitchClass::new(9), ScaleKind::Minor));
    app.view.piano.top_key = 72.0;
    app.run("note.add_many", json!({ "channel": "808", "notes": [{ "key": 55, "start": 2880, "length": 480 }] })).unwrap();
    app.run("note.set_props", json!({ "slide": true, "velocity": 0.5 })).unwrap();
    let chord = [57, 60, 64].map(|key| json!({ "key": key, "start": 1920, "length": 720 }));
    app.run("note.add_many", json!({ "channel": "808", "notes": chord })).unwrap();
}

/// Draws the main window in `theme`, set up by `setup`, and compares it with snapshot `name`.
fn snapshot(name: &str, theme: Theme, setup: fn(&mut BodoApp)) {
    let mut app = common::app(theme);
    setup(&mut app);
    let mut harness = common::harness(app, theme, true);
    harness.run_steps(6);
    harness.snapshot_options(name, &options(name));
}

#[test]
fn the_main_window_dark() {
    snapshot("main_dark", Theme::Dark, |_| {});
}

#[test]
fn the_main_window_light() {
    snapshot("main_light", Theme::Light, |_| {});
}

#[test]
fn a_beat_dark() {
    snapshot("beat_dark", Theme::Dark, with_beat);
}

#[test]
fn a_beat_light() {
    snapshot("beat_light", Theme::Light, with_beat);
}

#[test]
fn the_piano_roll_dark() {
    snapshot("piano_dark", Theme::Dark, with_piano);
}

#[test]
fn the_piano_roll_light() {
    snapshot("piano_light", Theme::Light, with_piano);
}
