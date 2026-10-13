//! The window model (project.md section 14.1, ADRs 0012 and 0032), driven with real pointer and key events: layouts
//! saved, switched, renamed and deleted through the Layouts menu; shortcuts rebound in the Keyboard shortcuts dialog
//! and then used, with a conflict taken over; windows detached and put back; and the Master layout.
//!
//! A detached window shows as its own OS window only where the backend can open more than one, which the headless
//! harness can't: there it stays in the workspace with its Attach button. These tests cover the detached state, its
//! place in layouts and the buttons; the OS window itself is eframe's.
//!
//! Snapshots show the Keyboard shortcuts dialog and the Layouts menu in both themes. After an intended change,
//! `UPDATE_SNAPSHOTS=1 cargo test -p bodo-ui --test windows` rewrites them.

#![allow(clippy::unwrap_used, clippy::panic, reason = "tests")]

mod common;

use bodo_ui::BodoApp;
use bodo_ui::windows::{Preset, WindowKind};
use common::{click, element};
use egui::{Event, Key, Modifiers, Theme};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

fn harness(theme: Theme) -> Harness<'static, BodoApp> {
    let mut h = common::harness(common::app(theme), theme, false);
    h.run_steps(3);
    h
}

/// Types `text` into the field that has the keyboard.
fn type_text(h: &mut Harness<'_, BodoApp>, text: &str) {
    h.event(Event::Text(text.to_owned()));
    h.run_steps(2);
}

/// Clicks the element whose label starts with `start`, such as a window button whose label ends with the title.
fn click_starting(h: &mut Harness<'_, BodoApp>, start: &str) {
    let found = bodo_ui::control::elements(&h.ctx).into_iter().rev().find(|e| e.label.starts_with(start));
    let at = found.unwrap_or_else(|| panic!("no element starting with `{start}`")).rect.center();
    common::click_at(h, at);
}

/// Narrows the shortcuts dialog to `text`, so a row further down is in view.
fn search_shortcuts(h: &mut Harness<'_, BodoApp>, text: &str) {
    click(h, "Search shortcuts");
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    type_text(h, text);
}

#[test]
fn layouts_save_switch_rename_and_delete_from_the_layouts_menu() {
    let mut h = harness(Theme::Dark);
    // Make the windows differ from the default: the piano roll open, the mixer closed.
    h.state_mut().run("ui.open_window", json!({ "window": "pianoRoll" })).unwrap();
    h.state_mut().run("ui.toggle_window", json!({ "window": "mixer" })).unwrap();
    h.run_steps(2);
    let arranged = h.state().prefs.layout.clone();

    click(&mut h, "Layouts");
    h.get_by_label("Save layout as...").click();
    h.run_steps(3);
    type_text(&mut h, "Two screens");
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().prefs.layouts.len(), 1);
    assert_eq!(h.state().prefs.layouts[0].name, "Two screens");
    assert_eq!(h.state().prefs.layout_name.as_deref(), Some("Two screens"));

    click(&mut h, "Layouts");
    h.get_by_label("Mix").click();
    h.run_steps(3);
    assert!(h.state().prefs.layout.is_open(WindowKind::Mixer) && !h.state().prefs.layout.is_open(WindowKind::PianoRoll));

    click(&mut h, "Layouts");
    h.get_by_label("Two screens").click();
    h.run_steps(3);
    assert_eq!(h.state().prefs.layout.windows, arranged.windows, "the saved windows come back as they were");

    // A preset's name is taken.
    click(&mut h, "Layouts");
    h.get_by_label("Save layout as...").click();
    h.run_steps(3);
    type_text(&mut h, "mix");
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert!(h.get_all_by_label_contains("one of Bodo's own layouts").count() > 0, "the dialog says why");
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert_eq!(h.state().prefs.layouts.len(), 1);

    click(&mut h, "Layouts");
    h.get_by_label_contains("Rename layout").click();
    h.run_steps(3);
    // The submenu's entry, drawn after the menu's own.
    h.get_all_by_label("Two screens").last().unwrap().click();
    h.run_steps(3);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    type_text(&mut h, "Studio");
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().prefs.layouts[0].name, "Studio");
    assert_eq!(h.state().prefs.layout_name.as_deref(), Some("Studio"));

    click(&mut h, "Layouts");
    h.get_by_label_contains("Delete layout").click();
    h.run_steps(3);
    h.get_all_by_label("Studio").last().unwrap().click();
    h.run_steps(3);
    assert!(h.state().prefs.layouts.is_empty());
}

#[test]
fn a_rebound_window_key_works_and_the_old_one_doesnt() {
    let mut h = harness(Theme::Dark);
    h.state_mut().view.dialogs.shortcuts = true;
    h.run_steps(3);
    click(&mut h, "Shortcut window:mixer");
    assert!(h.state().view.dialogs.recording_shortcut(), "the button listens for new keys");
    h.key_press(Key::F11);
    h.run_steps(3);
    assert!(!h.state().view.dialogs.recording_shortcut());
    assert_eq!(h.state().prefs.shortcuts.get("window:mixer").map(String::as_str), Some("F11"));
    h.state_mut().view.dialogs.shortcuts = false;
    h.run_steps(2);

    let open = |h: &Harness<'_, BodoApp>| h.state().prefs.layout.is_open(WindowKind::Mixer);
    let before = open(&h);
    h.key_press(Key::F9);
    h.run_steps(2);
    assert_eq!(open(&h), before, "F9 does nothing now");
    h.key_press(Key::F11);
    h.run_steps(2);
    assert_ne!(open(&h), before, "F11 shows or hides the mixer");
    assert!(element(&h, "Mixer").is_positive(), "the top bar's button is still there");
}

#[test]
fn keys_in_use_ask_first_and_can_be_taken_over() {
    let mut h = harness(Theme::Dark);
    h.state_mut().view.dialogs.shortcuts = true;
    h.run_steps(3);
    click(&mut h, "Shortcut tool:draw");
    h.key_press(Key::B);
    h.run_steps(3);
    assert!(h.state().prefs.shortcuts.is_empty(), "nothing changes until the user answers");
    assert!(h.get_all_by_label_contains("is used by Paint tool").count() > 0, "the dialog names the other shortcut");
    click(&mut h, "Use the keys here");
    assert_eq!(h.state().prefs.shortcuts.get("tool:draw").map(String::as_str), Some("B"));
    assert_eq!(h.state().prefs.shortcuts.get("tool:paint").map(String::as_str), Some(""), "paint gave its key up");

    // Escape stops listening without changing anything.
    search_shortcuts(&mut h, "project.save");
    click(&mut h, "Shortcut command:project.save");
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(!h.state().prefs.shortcuts.contains_key("command:project.save"));

    // B now picks the draw tool.
    h.state_mut().view.dialogs.shortcuts = false;
    h.state_mut().view.playlist.tool = bodo_ui::Tool::Zoom;
    h.run_steps(2);
    h.key_press(Key::B);
    h.run_steps(2);
    assert_eq!(h.state().view.playlist.tool, bodo_ui::Tool::Draw);

    // Reset puts draw's key back; Reset all puts every key back.
    h.state_mut().view.dialogs.shortcuts = true;
    h.run_steps(3);
    search_shortcuts(&mut h, "tool");
    click(&mut h, "Reset shortcut tool:draw");
    assert!(!h.state().prefs.shortcuts.contains_key("tool:draw"));
    click(&mut h, "Reset all shortcuts");
    assert!(h.state().prefs.shortcuts.is_empty());
}

#[test]
fn menus_show_the_keys_shortcuts_have_now() {
    let mut h = harness(Theme::Dark);
    let save_keys = |app: &BodoApp| {
        let menus = bodo_ui::menu::menus(app);
        let file = menus.iter().find(|m| m.title == "File").unwrap();
        file.items
            .iter()
            .find_map(|i| match i {
                bodo_ui::menu::Item::Action { id, keys, .. } if id == "file.save" => Some(keys.clone()),
                _ => None,
            })
            .unwrap()
    };
    assert_eq!(save_keys(h.state()).as_deref(), Some("Mod+S"));
    h.state_mut().run("ui.set_shortcut", json!({ "binding": "command:project.save", "keys": "Mod+Shift+W" })).unwrap();
    assert_eq!(save_keys(h.state()).as_deref(), Some("Mod+Shift+W"));
    // Keys another shortcut has are refused unless taken over.
    let taken = h.state_mut().run("ui.set_shortcut", json!({ "binding": "command:project.save", "keys": "Mod+Z" }));
    assert!(taken.unwrap_err().contains("already used"));
    h.state_mut().run("ui.set_shortcut", json!({ "binding": "command:project.save", "keys": "" })).unwrap();
    assert_eq!(save_keys(h.state()), None, "an empty binding removes the shortcut");
    h.state_mut().run("ui.reset_shortcuts", json!({})).unwrap();
    assert_eq!(save_keys(h.state()).as_deref(), Some("Mod+S"));
    // The in-window menu draws the same model.
    click(&mut h, "Menu File");
    h.run_steps(2);
    assert!(h.get_all_by_label_contains("Save as...").count() > 0, "File opens with the model's items");
}

#[test]
fn windows_detach_go_back_and_layouts_remember_it() {
    let mut h = harness(Theme::Dark);
    assert!(bodo_ui::control::elements(&h.ctx).iter().all(|e| !e.label.starts_with("Detach Channel rack")), "the channel rack stays in");
    click_starting(&mut h, "Detach Playlist");
    let state = h.state().prefs.layout.state(WindowKind::Playlist);
    assert!(state.detached && state.open);
    assert!(state.screen.is_some_and(|[_, _, w, h]| w > 100.0 && h > 100.0), "it opens the size it had: {:?}", state.screen);

    // Saved layouts keep the detached window detached.
    h.state_mut().run("ui.save_layout", json!({ "name": "Out" })).unwrap();
    h.state_mut().run("ui.set_layout", json!({ "layout": "Default" })).unwrap();
    assert!(!h.state().prefs.layout.state(WindowKind::Playlist).detached);
    h.state_mut().run("ui.set_layout", json!({ "layout": "Out" })).unwrap();
    h.run_steps(2);
    assert!(h.state().prefs.layout.state(WindowKind::Playlist).detached);

    // Without more OS windows, it stays in the workspace with its Attach button.
    click_starting(&mut h, "Attach Playlist");
    assert!(!h.state().prefs.layout.state(WindowKind::Playlist).detached);

    // Agents and the View menu do the same.
    h.state_mut().run("ui.detach_window", json!({ "window": "mixer", "detached": true })).unwrap();
    assert!(h.state().prefs.layout.state(WindowKind::Mixer).detached);
    assert!(h.state_mut().run("ui.detach_window", json!({ "window": "channelRack" })).is_err());
}

#[test]
fn the_master_layout_fills_the_workspace_with_the_mixer_on_the_master() {
    let mut h = harness(Theme::Dark);
    click(&mut h, "Layouts");
    h.get_by_label("Master").click();
    h.run_steps(3);
    let mixer = h.state().prefs.layout.state(WindowKind::Mixer);
    assert!(mixer.open && mixer.maximized);
    assert!(!h.state().prefs.layout.is_open(WindowKind::Playlist));
    let master = h.state().session.project().mixer.master.id;
    assert_eq!(h.state().session.editor().insert, Some(master));
    assert_eq!(h.state().prefs.layout_name.as_deref(), Some(Preset::Master.label()));
}

#[test]
fn old_preferences_still_load() {
    // Preferences saved before layouts had names, shortcuts or detached windows.
    let old = r#"{"theme":"dark","layout":{"windows":{"playlist":{"open":true,"frame":[0.5,0.0,0.5,1.0],"maximized":false,"minimized":false}},"browser":true,"browserWidth":230.0}}"#;
    let prefs: bodo_ui::Prefs = serde_json::from_str(old).unwrap();
    assert!(prefs.layouts.is_empty() && prefs.shortcuts.is_empty());
    assert!(!prefs.layout.state(WindowKind::Playlist).detached);
}

fn shortcuts_snapshot(name: &str, theme: Theme) {
    let mut app = common::app(theme);
    app.view.dialogs.shortcuts = true;
    app.prefs.shortcuts.insert(String::from("window:mixer"), String::from("F11"));
    let mut h = common::harness(app, theme, true);
    h.run_steps(6);
    h.snapshot_options(name, &common::snapshot_options(name));
}

#[test]
fn the_shortcuts_dialog_dark() {
    shortcuts_snapshot("shortcuts_dark", Theme::Dark);
}

#[test]
fn the_shortcuts_dialog_light() {
    shortcuts_snapshot("shortcuts_light", Theme::Light);
}

fn layouts_snapshot(name: &str, theme: Theme) {
    let mut app = common::app(theme);
    app.run("ui.save_layout", json!({ "name": "Two screens" })).unwrap();
    let mut h = common::harness(app, theme, true);
    h.run_steps(3);
    click(&mut h, "Layouts");
    h.run_steps(3);
    h.snapshot_options(name, &common::snapshot_options(name));
}

#[test]
fn the_layouts_menu_dark() {
    layouts_snapshot("layouts_dark", Theme::Dark);
}

#[test]
fn the_layouts_menu_light() {
    layouts_snapshot("layouts_light", Theme::Light);
}
