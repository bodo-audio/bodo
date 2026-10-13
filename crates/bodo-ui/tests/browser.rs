//! The Browser and the user library (project.md section 14.1, ADR 0029), driven with real pointer events: entering
//! folders with back and up, filtering by tag, loading presets by double-click and by dropping them on a window,
//! starting a project from a template, previewing without a sound device, and the Marketplace's "new" mark. Two
//! snapshots show the Browser with its folders open, in the dark and light themes.
//!
//! After an intended change, `UPDATE_SNAPSHOTS=1 cargo test -p bodo-ui --test browser` rewrites the snapshots.

#![allow(clippy::unwrap_used, clippy::panic, reason = "tests")]

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bodo_engine::library::UserLibrary;
use bodo_engine::library::user::{MarketFolder, PresetFolder};
use bodo_ui::BodoApp;
use common::{click, double_click_at, drag, element, line};
use egui::{Modifiers, PointerButton, Theme};
use egui_kittest::kittest::Queryable;
use egui_kittest::{Harness, OsThreshold, SnapshotOptions};
use serde_json::json;

/// A user library in a fresh folder, with a few files of the user's own: a sound in Audio/Drums, a pad, and a MIDI
/// file.
fn library(name: &str) -> UserLibrary {
    let root = std::env::temp_dir().join(format!("bodo-ui-browser-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let library = UserLibrary::new(&root);
    library.ensure().unwrap();
    let factory = bodo_engine::locate_factory().unwrap();
    let drums = library.folder("Audio").join("Drums");
    std::fs::create_dir_all(&drums).unwrap();
    std::fs::copy(factory.join("Packs/Bodo Drums/Rim.wav"), drums.join("Tight Rim.wav")).unwrap();
    std::fs::copy(factory.join("Packs/Bodo Drums/Shaker.wav"), library.folder("Audio").join("Soft Pad.wav")).unwrap();
    library
}

/// The app with `library` as its user library, set up by `setup`, in a harness.
fn harness(library: &UserLibrary, theme: Theme, wgpu: bool, setup: impl FnOnce(&mut BodoApp)) -> Harness<'static, BodoApp> {
    let env = bodo_engine::Env { factory: bodo_engine::locate_factory(), library: Vec::new(), user: Some(library.root().to_path_buf()) };
    let prefs = bodo_ui::Prefs {
        theme: if theme == Theme::Light { bodo_ui::theme::ThemePref::Light } else { bodo_ui::theme::ThemePref::Dark },
        user_library: Some(library.root().to_path_buf()),
        ..bodo_ui::Prefs::default()
    };
    let mut app = BodoApp::new(bodo_engine::Session::deterministic(env), prefs, bodo_ui::Services::default(), false);
    setup(&mut app);
    let mut harness = common::harness(app, theme, wgpu);
    harness.run_steps(4);
    harness
}

/// Steps until the library's index is read, so tags and search are ready.
fn wait_for_index(h: &mut Harness<'_, BodoApp>) {
    let start = Instant::now();
    h.run_steps(2);
    while h.state().view.browser.indexing() && start.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(5));
        h.step();
    }
    h.run_steps(2);
}

fn has(h: &Harness<'_, BodoApp>, label: &str) -> bool {
    bodo_ui::control::elements(&h.ctx).iter().any(|e| e.label == label)
}

fn double_click(h: &mut Harness<'_, BodoApp>, label: &str) {
    let at = element(h, label).center();
    double_click_at(h, at);
    h.run_steps(2);
}

fn location(h: &Harness<'_, BodoApp>) -> Option<PathBuf> {
    h.state().view.browser.location().map(Path::to_path_buf)
}

#[test]
fn double_clicking_enters_folders_and_back_and_up_move_around() {
    let library = library("navigate");
    let mut h = harness(&library, Theme::Dark, false, |_| {});
    double_click(&mut h, "Folder Audio");
    assert_eq!(location(&h), Some(library.folder("Audio")));
    assert!(has(&h, "Browser in Library > Audio"));
    assert!(has(&h, "Sound Soft Pad"), "the folder's sounds are listed");
    double_click(&mut h, "Folder Drums");
    assert_eq!(location(&h), Some(library.folder("Audio").join("Drums")));
    assert!(has(&h, "Sound Tight Rim"));
    click(&mut h, "Browser back");
    assert_eq!(location(&h), Some(library.folder("Audio")), "back returns to where the Browser was");
    click(&mut h, "Browser up");
    assert_eq!(location(&h), Some(library.root().to_path_buf()), "up goes to the folder above");
    assert!(has(&h, "Folder Templates"));
    click(&mut h, "Browser up");
    assert_eq!(location(&h), None, "above the library is the overview");
    assert!(has(&h, "Folder Current project"));
    click(&mut h, "Browser back");
    assert_eq!(location(&h), Some(library.root().to_path_buf()));
}

#[test]
fn tags_filter_the_library_and_the_users_own_tags_count() {
    let library = library("tags");
    let mut h = harness(&library, Theme::Dark, false, |app| {
        let pad = library.folder("Audio").join("Soft Pad.wav").display().to_string();
        app.run("library.tag", json!({ "path": pad, "add": ["warm"] })).unwrap();
    });
    wait_for_index(&mut h);
    click(&mut h, "Tag warm");
    h.run_steps(2);
    assert!(has(&h, "Sound Soft Pad"));
    assert!(!has(&h, "Sound Tight Rim") && !has(&h, "Sound Kick"), "only the tagged sound shows");
    assert!(h.state().view.browser.tags.contains("warm"));
    click(&mut h, "Tag warm");
    assert!(h.state().view.browser.tags.is_empty());
    // A folder name is a tag of everything in it.
    click(&mut h, "Tag bodo drums");
    h.run_steps(2);
    assert!(has(&h, "Sound Kick") && !has(&h, "Sound Soft Pad"));
}

#[test]
fn a_channel_preset_loads_by_double_click_as_one_undo_step() {
    let library = library("channel-preset");
    let mut h = harness(&library, Theme::Dark, false, |app| {
        app.run("channel.set_pan", json!({ "channel": "Kick", "pan": -0.4 })).unwrap();
        app.run("preset.save_channel", json!({ "channel": "Kick", "name": "Wide Kick" })).unwrap();
    });
    assert!(library.presets(PresetFolder::ChannelPresets).join("Wide Kick.bodochannel").is_file());
    click(&mut h, "Folder Channel presets");
    let steps = h.state().session.undo_labels().count();
    double_click(&mut h, "Channel preset Wide Kick");
    let project = h.state().session.project();
    let kick = project.channels.values().find(|c| c.name == "Wide Kick").unwrap_or_else(|| panic!("no Wide Kick"));
    assert_eq!(kick.pan, -0.4);
    assert_eq!(h.state().session.undo_labels().count(), steps + 1);
}

#[test]
fn a_mixer_preset_dropped_on_a_strip_loads_on_that_insert() {
    let library = library("mixer-preset");
    let mut h = harness(&library, Theme::Dark, false, |app| {
        app.run("mixer.set_volume", json!({ "insert": 1, "db": -9.0 })).unwrap();
        app.run("preset.save_mixer", json!({ "insert": 1, "name": "Quiet" })).unwrap();
    });
    click(&mut h, "Folder Mixer presets");
    let from = element(&h, "Mixer preset Quiet").center();
    let to = element(&h, "Insert 3 strip").center();
    drag(&mut h, PointerButton::Primary, from, &line(from, to, 16), Modifiers::NONE);
    h.run_steps(2);
    let project = h.state().session.project();
    let third = &project.mixer.inserts[&project.mixer.order[2]];
    assert_eq!((third.name.as_str(), third.volume_db), ("Insert 3", -9.0), "insert 3 took the preset's level and kept its name");
}

#[test]
fn a_template_starts_a_new_project_and_a_score_lands_in_the_piano_roll() {
    let library = library("template");
    let mut h = harness(&library, Theme::Dark, false, |app| {
        app.run("project.set_info", json!({ "title": "Starter" })).unwrap();
        app.run("channel.add", json!({ "sample": "factory:Packs/Bodo Drums/Rim.wav" })).unwrap();
        app.run("project.save_template", json!({})).unwrap();
        app.run("pattern.set_steps", json!({ "channel": "Kick", "steps": "x... x... x... x..." })).unwrap();
        app.run("preset.save_score", json!({ "channel": "Kick", "name": "Four" })).unwrap();
        app.run("project.new", json!({ "template": "basic", "discardChanges": true })).unwrap();
    });
    click(&mut h, "Folder Scores");
    double_click(&mut h, "Score Four");
    let project = h.state().session.project().clone();
    let notes: usize = project.patterns.values().map(|p| p.notes.values().map(|s| s.notes().len()).sum::<usize>()).sum();
    assert_eq!(notes, 4, "the score's four notes went to the selected channel");
    click(&mut h, "Folder Templates");
    // The score made changes, so starting a new project asks first; discard them.
    double_click(&mut h, "Project Starter");
    h.get_by_label("Don't save").click();
    h.run_steps(3);
    assert!(h.state().session.project().channels.values().any(|c| c.name == "Rim"));
    assert!(h.state().session.file().is_none(), "the template stays a template");
}

#[test]
fn clicking_a_sound_without_a_device_previews_nothing_and_says_so() {
    let library = library("preview");
    let mut h = harness(&library, Theme::Dark, false, |_| {});
    click(&mut h, "Folder Bodo Drums");
    click(&mut h, "Sound Kick");
    h.run_steps(2);
    assert!(h.state().audio.previewing().is_none());
    assert!(h.state().status().is_some_and(|(s, _)| s.contains("no sound")), "{:?}", h.state().status());
}

#[test]
fn the_marketplace_marks_what_arrived_until_it_is_looked_at() {
    let library = library("market");
    let mut h = harness(&library, Theme::Dark, false, |_| {});
    assert!(!has(&h, "Marketplace new"));
    std::fs::copy(
        bodo_engine::locate_factory().unwrap().join("Packs/Bodo Drums/Crash.wav"),
        library.market(MarketFolder::Stems).join("Rhodes Dust.wav"),
    )
    .unwrap();
    let start = Instant::now();
    while !has(&h, "Marketplace new") && start.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(50));
        h.step();
    }
    assert!(has(&h, "Marketplace new"), "the arrival shows while Bodo runs");
    assert!(has(&h, "Folder Stems (owned) new"));
    click(&mut h, "Folder Stems (owned) new");
    h.run_steps(3);
    assert!(!has(&h, "Marketplace new"), "looking at it clears the mark");
    assert!(has(&h, "Sound Rhodes Dust"));
}

fn options(name: &str) -> SnapshotOptions {
    let os_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots").join(std::env::consts::OS);
    let options = SnapshotOptions::new().threshold(OsThreshold::new(1.5).windows(2.0)).max_failed_pixels(OsThreshold::new(400).windows(2000));
    if os_dir.join(format!("{name}.png")).exists() { options.output_path(os_dir) } else { options }
}

/// The Browser with the channel presets and the Marketplace's stems open, a preset and a stem in them.
fn snapshot(name: &str, theme: Theme) {
    let library = library(name);
    std::fs::copy(
        bodo_engine::locate_factory().unwrap().join("Packs/Bodo Drums/Crash.wav"),
        library.market(MarketFolder::Stems).join("Keys Haze.wav"),
    )
    .unwrap();
    let mut h = harness(&library, theme, true, |app| {
        app.run("preset.save_channel", json!({ "channel": "Snare", "name": "Crack Snare" })).unwrap();
    });
    click(&mut h, "Folder Channel presets");
    click(&mut h, "Folder Audio");
    h.run_steps(4);
    h.snapshot_options(name, &options(name));
}

#[test]
fn the_browser_dark() {
    snapshot("browser_dark", Theme::Dark);
}

#[test]
fn the_browser_light() {
    snapshot("browser_light", Theme::Light);
}
