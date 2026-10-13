//! Snapshot tests: a gallery of every widget and a grid of every icon, in the dark and light palettes, drawn with
//! the wgpu renderer and compared with the PNGs in `tests/snapshots` with a small tolerance. A platform whose
//! rasterizer differs more can keep its own baseline in `tests/snapshots/<os>/` (`macos`, `linux` or `windows`).
//!
//! After an intended change, `UPDATE_SNAPSHOTS=1 cargo test -p bodo-kit-ui --test snapshots` rewrites them.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, reason = "tests")]

use bodo_kit_ui::{Fader, Knob, Led, Meter, Palette, StepRow, icons, knob};
use egui::{Rect, Theme, Ui, Vec2, vec2};
use egui_kittest::{Harness, OsThreshold, SnapshotOptions};

/// The tolerance: GPU rasterizers differ slightly in antialiasing, and Windows CI draws with WARP.
fn options(name: &str) -> SnapshotOptions {
    let os_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots").join(std::env::consts::OS);
    let options = SnapshotOptions::new().threshold(OsThreshold::new(1.5).windows(2.0)).max_failed_pixels(OsThreshold::new(100).windows(600));
    if os_dir.join(format!("{name}.png")).exists() { options.output_path(os_dir) } else { options }
}

fn palette(theme: Theme) -> Palette {
    match theme {
        Theme::Dark => Palette::dark(),
        Theme::Light => Palette::light(),
    }
}

/// The panel background each theme's widgets sit on (project.md section 14.2).
fn background(theme: Theme) -> egui::Color32 {
    match theme {
        Theme::Dark => egui::Color32::from_rgb(0x14, 0x14, 0x14),
        Theme::Light => egui::Color32::from_rgb(0xfa, 0xfa, 0xfa),
    }
}

fn snapshot(theme: Theme, size: Vec2, name: &str, draw: impl FnMut(&mut Ui) + 'static) {
    let mut draw = draw;
    let mut harness = Harness::builder().with_size(size).with_theme(theme).wgpu().build_ui(move |ui| {
        palette(theme).install(ui.ctx());
        ui.painter().rect_filled(ui.max_rect().expand(16.0), 0.0, background(theme));
        draw(ui);
    });
    harness.run();
    harness.snapshot_options(name, &options(name));
}

/// Every widget in a few states.
fn gallery(ui: &mut Ui) {
    ui.spacing_mut().item_spacing = vec2(10.0, 10.0);
    ui.horizontal(|ui| {
        let (mut a, mut b, mut c, mut d) = (0.0, 0.35, -0.5, 1.0);
        ui.add(Knob::new(&mut a, 0.0..=1.0).label("Empty").diameter(32.0));
        ui.add(Knob::new(&mut b, 0.0..=1.0).label("Volume").diameter(32.0));
        ui.add(knob::pan(&mut c).label("Pan").diameter(32.0));
        ui.add(Knob::new(&mut d, 0.0..=1.0).label("Full").diameter(24.0));
        let (mut on, mut off) = (true, false);
        ui.add(Led::new(&mut on).label("On"));
        ui.add(Led::new(&mut off).label("Off"));
    });
    let kick: Vec<bool> = (0..16).map(|i| i % 4 == 0).collect();
    let hats: Vec<bool> = (0..16).map(|i| i % 2 == 1).collect();
    let _ = StepRow::new("Kick", &kick).show(ui);
    let _ = StepRow::new("Hats", &hats).show(ui);
    ui.horizontal(|ui| {
        let (mut zero, mut silent, mut loud) = (0.0, f32::NEG_INFINITY, 6.0);
        ui.add(Fader::new(&mut zero).label("Zero").ticks(true));
        ui.add(Fader::new(&mut silent).label("Silent"));
        ui.add(Fader::new(&mut loud).label("Loud"));
        ui.add(Meter::mono(0.25).label("Mono"));
        ui.add(Meter::stereo(0.7, 0.06).label("Stereo"));
        ui.add(Meter::stereo(1.2, 0.9).label("Clipping"));
        ui.add(Meter::stereo(0.0, 0.0).label("Quiet"));
    });
}

/// Every icon at 24 points in rows of twelve, and the first row again at 16 points.
fn icon_grid(ui: &mut Ui, theme: Theme) {
    let color = palette(theme).text;
    let origin = ui.max_rect().min + vec2(8.0, 8.0);
    for (index, icon) in icons::ALL.iter().enumerate() {
        let (row, column) = (index / 12, index % 12);
        let rect = Rect::from_min_size(origin + vec2(column as f32 * 32.0, row as f32 * 32.0), Vec2::splat(24.0));
        icon.paint(ui.painter(), rect, color);
    }
    let rows = icons::ALL.len().div_ceil(12);
    for (column, icon) in icons::ALL.iter().take(12).enumerate() {
        let rect = Rect::from_min_size(origin + vec2(column as f32 * 32.0 + 4.0, rows as f32 * 32.0 + 4.0), Vec2::splat(16.0));
        icon.paint(ui.painter(), rect, color);
    }
}

fn icon_size() -> Vec2 {
    let rows = icons::ALL.len().div_ceil(12) + 1;
    vec2(12.0 * 32.0 + 8.0, rows as f32 * 32.0 + 8.0)
}

#[test]
fn widgets_dark() {
    snapshot(Theme::Dark, vec2(420.0, 300.0), "widgets_dark", gallery);
}

#[test]
fn widgets_light() {
    snapshot(Theme::Light, vec2(420.0, 300.0), "widgets_light", gallery);
}

#[test]
fn icons_dark() {
    snapshot(Theme::Dark, icon_size(), "icons_dark", |ui| icon_grid(ui, Theme::Dark));
}

#[test]
fn icons_light() {
    snapshot(Theme::Light, icon_size(), "icons_light", |ui| icon_grid(ui, Theme::Light));
}
