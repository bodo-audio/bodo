//! Recording, the metronome, count-in, the loop region and MIDI files, driven with real pointer events (project.md
//! sections 6.4, 9.7 and 14.1). The harness has no audio device, so these check what the buttons and menus do to the
//! session; the audio side has its own tests. Two snapshots show an armed insert and the transport toggles in both
//! themes; `UPDATE_SNAPSHOTS=1 cargo test -p bodo-ui --test recording` rewrites them after an intended change.

#![allow(clippy::unwrap_used, clippy::panic, reason = "tests")]

mod common;

use bodo_model::mixer::{MixerInsert, RecordFormat, RecordTap};
use bodo_ui::BodoApp;
use common::{click, click_at, element};
use egui::Theme;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

fn harness(app: BodoApp) -> Harness<'static, BodoApp> {
    let mut harness = common::harness(app, Theme::Dark, false);
    harness.run_steps(4);
    harness
}

/// Insert `number` (from 1) as the project has it now.
fn insert(harness: &Harness<'_, BodoApp>, number: usize) -> MixerInsert {
    let project = harness.state().session.project();
    let id = project.mixer.order[number - 1];
    (*project.mixer.inserts[&id]).clone()
}

fn steps(harness: &Harness<'_, BodoApp>) -> usize {
    harness.state().session.undo_labels().count()
}

/// Clicks the widget whose accessible label is `label`, such as a menu item, with the pointer.
fn click_labeled(harness: &mut Harness<'_, BodoApp>, label: &str) {
    let at = harness.get_by_label(label).rect().center();
    click_at(harness, at);
}

#[test]
fn arming_an_insert_and_choosing_its_input() {
    let mut h = harness(common::app(Theme::Dark));
    let before = steps(&h);
    click(&mut h, "Insert 1 record arm");
    assert!(insert(&h, 1).armed);
    assert!(!insert(&h, 2).armed, "only the clicked strip arms");

    // Without a device the selector offers four stereo pairs.
    click(&mut h, "Insert 1 input");
    click_labeled(&mut h, "In 1-2");
    assert_eq!(insert(&h, 1).input.map(|p| p.channels), Some(vec![0, 1]));
    click(&mut h, "Insert 1 input");
    click_labeled(&mut h, "In 3");
    assert_eq!(insert(&h, 1).input.map(|p| p.channels), Some(vec![2]));
    click(&mut h, "Insert 1 input");
    click_labeled(&mut h, "None");
    assert_eq!(insert(&h, 1).input, None);
    assert_eq!(steps(&h), before + 4, "each choice is one undo step");

    click(&mut h, "Insert 1 record arm");
    assert!(!insert(&h, 1).armed);
}

/// Right-clicks the number at the top of strip `label`, where no control sits, to open its menu.
fn strip_menu(harness: &mut Harness<'_, BodoApp>, label: &str) {
    let rect = element(harness, label);
    let at = egui::pos2(rect.center().x, rect.top() + 16.0);
    harness.event(egui::Event::PointerMoved(at));
    harness.step();
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: egui::Modifiers::NONE });
        harness.step();
    }
    harness.step();
}

#[test]
fn the_strip_menu_sets_what_and_how_it_records() {
    let mut h = harness(common::app(Theme::Dark));
    strip_menu(&mut h, "Insert 2 strip");
    // Submenu buttons end with an arrow.
    click_labeled(&mut h, "Record from ⏵");
    click_labeled(&mut h, "Input");
    assert_eq!(insert(&h, 2).record_tap, RecordTap::Input);

    strip_menu(&mut h, "Insert 2 strip");
    click_labeled(&mut h, "Record format ⏵");
    click_labeled(&mut h, "24-bit WAV");
    assert_eq!(insert(&h, 2).record_format, RecordFormat::Int24);
}

#[test]
fn the_metronome_and_count_in_toggle_without_entering_the_history() {
    let mut h = harness(common::app(Theme::Dark));
    let before = steps(&h);
    click(&mut h, "Metronome");
    assert!(h.state().session.transport().metronome);
    click(&mut h, "Count-in");
    assert!(h.state().session.transport().count_in);
    click(&mut h, "Metronome");
    click(&mut h, "Count-in");
    let transport = *h.state().session.transport();
    assert!(!transport.metronome && !transport.count_in);
    assert_eq!(steps(&h), before, "switches are not edits");
}

#[test]
fn the_record_button_starts_and_stops_a_recording() {
    let mut h = harness(common::app(Theme::Dark));
    click(&mut h, "Record");
    let transport = *h.state().session.transport();
    assert!(transport.recording && transport.playing, "recording plays: {transport:?}");
    assert!(element(&h, "Record") != egui::Rect::NOTHING);
    click(&mut h, "Record");
    let transport = *h.state().session.transport();
    assert!(!transport.recording && !transport.playing, "a second click stops: {transport:?}");
}

#[test]
fn the_loop_indicator_shows_the_region_and_clears_it() {
    let mut h = harness(common::app(Theme::Dark));
    assert!(bodo_ui::control::elements(&h.ctx).iter().all(|e| e.label != "Loop region"), "no loop, no indicator");
    h.state_mut().run("transport.set_loop", json!({ "start": "3:1:0", "end": "5:1:0" })).unwrap();
    h.run_steps(2);
    h.get_by_label("Loop 3:1 to 5:1");
    click(&mut h, "Loop region");
    assert_eq!(h.state().session.transport().loop_region, None);
    h.run_steps(2);
    assert!(bodo_ui::control::elements(&h.ctx).iter().all(|e| e.label != "Loop region"));
}

#[test]
fn midi_files_export_and_import_through_the_file_menu() {
    let dir = std::env::temp_dir().join(format!("bodo-ui-midi-menu-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("Verse.mid");

    let mut app = common::beat_app();
    app.run("ui.select_pattern", json!({ "pattern": "Verse" })).unwrap();
    let (asked, path) = (std::rc::Rc::new(std::cell::RefCell::new(String::new())), file.clone());
    let seen = std::rc::Rc::clone(&asked);
    app.services.pick_midi_save = Some(Box::new(move |name: &str| {
        *seen.borrow_mut() = name.to_owned();
        Some(path.clone())
    }));
    let mut h = harness(app);
    click_labeled(&mut h, "File");
    click_labeled(&mut h, "Export pattern as MIDI...");
    assert_eq!(asked.borrow().as_str(), "Verse.mid", "the pattern's name is suggested");
    assert!(file.exists(), "{}", file.display());
    drop(h);

    let mut app = common::app(Theme::Dark);
    let path = file.clone();
    app.services.pick_midi = Some(Box::new(move || Some(path.clone())));
    let mut h = harness(app);
    let patterns = h.state().session.project().patterns.len();
    click_labeled(&mut h, "File");
    click_labeled(&mut h, "Import MIDI file...");
    let project = h.state().session.project().clone();
    assert_eq!(project.patterns.len(), patterns + 1, "the file becomes a new pattern");
    assert!(project.tracks.values().any(|t| !t.clips.is_empty()), "and is placed on the playlist");
    let _ = std::fs::remove_dir_all(dir);
}

/// Insert 1 armed on inputs 1-2, insert 2 armed with none, the metronome and count-in on and a loop set, in `theme`.
fn snapshot(name: &str, theme: Theme) {
    let mut app = common::app(theme);
    for (id, params) in [
        ("mixer.arm", json!({ "insert": 1, "on": true })),
        ("mixer.set_input", json!({ "insert": 1, "channels": [0, 1] })),
        ("mixer.arm", json!({ "insert": 2, "on": true })),
        ("transport.metronome", json!({ "on": true })),
        ("transport.count_in", json!({ "on": true })),
        ("transport.set_loop", json!({ "start": "3:1:0", "end": "5:1:0" })),
    ] {
        app.run(id, params).unwrap();
    }
    let mut harness = common::harness(app, theme, true);
    harness.run_steps(6);
    harness.snapshot_options(name, &common::snapshot_options(name));
}

#[test]
fn armed_inserts_and_transport_toggles_dark() {
    snapshot("recording_dark", Theme::Dark);
}

#[test]
fn armed_inserts_and_transport_toggles_light() {
    snapshot("recording_light", Theme::Light);
}
