//! Keyboard use and the accessibility options (project.md section 14.7, ADR 0033), driven with real key events:
//! Tab through the top bar and into a window, the arrow keys on a knob, Space on a step, Escape out of dialogs and
//! menus, F10 to the menus, Mod+F6 between windows, the interface size keys and Auto, the high-contrast theme, the
//! color-blind-safe colors, and the interface in Turkish. Snapshots show the main window and a beat in the
//! high-contrast theme and in Turkish, and the Sampler in Turkish.
//!
//! After an intended change, `UPDATE_SNAPSHOTS=1 cargo test -p bodo-ui --test accessibility` rewrites the snapshots.

#![allow(clippy::unwrap_used, clippy::panic, reason = "tests")]

mod common;

use bodo_ui::i18n::LanguagePref;
use bodo_ui::theme::{Look, ThemePref, Tokens};
use bodo_ui::windows::WindowKind;
use bodo_ui::{BodoApp, Prefs};
use egui::{Key, Modifiers, Theme, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

/// The label of the element with keyboard focus, as the control channel records it, or its egui id.
fn focused(h: &Harness<'_, BodoApp>) -> Option<String> {
    let id = h.ctx.memory(|m| m.focused())?;
    let label = bodo_ui::control::elements(&h.ctx).iter().rev().find(|e| e.id == id).map(|e| e.label.clone());
    Some(label.unwrap_or_else(|| format!("{id:?}")))
}

fn press(h: &mut Harness<'_, BodoApp>, key: Key) {
    h.key_press(key);
    h.run_steps(2);
}

fn press_with(h: &mut Harness<'_, BodoApp>, modifiers: Modifiers, key: Key) {
    h.key_press_modifiers(modifiers, key);
    h.run_steps(3);
}

/// Presses Tab until the element labeled `label` has focus, as a person tabbing to it would.
fn tab_to(h: &mut Harness<'_, BodoApp>, label: &str) {
    let mut seen = Vec::new();
    for _ in 0..120 {
        press(h, Key::Tab);
        let now = focused(h);
        if now.as_deref() == Some(label) {
            return;
        }
        seen.push(now);
    }
    panic!("Tab never reached `{label}`; it went through {seen:?}");
}

fn app_with(prefs: Prefs) -> BodoApp {
    let env = bodo_engine::Env { factory: bodo_engine::locate_factory(), library: Vec::new(), user: None };
    BodoApp::new(bodo_engine::Session::deterministic(env), prefs, bodo_ui::Services::default(), false)
}

fn harness(app: BodoApp) -> Harness<'static, BodoApp> {
    let mut h = common::harness(app, Theme::Dark, false);
    h.run_steps(4);
    h
}

fn channel_pan(h: &Harness<'_, BodoApp>, name: &str) -> f32 {
    h.state().session.project().channels.values().find(|c| c.name == name).unwrap().pan
}

#[test]
fn tab_walks_the_top_bar_from_the_menus_to_the_transport_and_shift_tab_walks_back() {
    let mut h = harness(common::app(Theme::Dark));
    let mut order = Vec::new();
    for _ in 0..12 {
        press(&mut h, Key::Tab);
        order.push(focused(&h).unwrap());
    }
    let expected = [
        "Menu File",
        "Menu Edit",
        "Menu Add",
        "Menu Patterns",
        "Menu View",
        "Menu Options",
        "Menu Tools",
        "Menu Help",
        "Play mode",
        "Play",
        "Stop",
        "Record",
    ];
    assert_eq!(order, expected);
    press_with(&mut h, Modifiers::SHIFT, Key::Tab);
    assert_eq!(focused(&h).as_deref(), Some("Stop"));
}

#[test]
fn the_focused_control_shows_a_focus_ring_in_the_accent_color() {
    let mut h = harness(common::app(Theme::Dark));
    tab_to(&mut h, "Play");
    let ring = h.ctx.global_style().visuals.widgets.active.bg_stroke;
    assert_eq!(ring.color, Tokens::get().accent, "the focus ring is the accent token");
    assert!(ring.width >= 1.5, "and thick enough to see");
}

#[test]
fn f10_reaches_the_menus_and_escape_closes_an_open_one() {
    let mut h = harness(common::app(Theme::Dark));
    press(&mut h, Key::F10);
    h.run_steps(2);
    assert_eq!(focused(&h).as_deref(), Some("Menu File"));
    press(&mut h, Key::Space);
    assert!(egui::Popup::is_any_open(&h.ctx), "Space opens the focused menu");
    press(&mut h, Key::Escape);
    assert!(!egui::Popup::is_any_open(&h.ctx), "Escape closes it");
}

/// Presses Mod+F6 until window `kind`'s title bar has keyboard focus, and returns the title bars it went through.
fn to_window(h: &mut Harness<'_, BodoApp>, kind: WindowKind) -> Vec<Option<egui::Id>> {
    let title = bodo_ui::windows::area_id(kind).with("title");
    let mut fronts = Vec::new();
    for _ in 0..WindowKind::ALL.len() {
        press_with(h, Modifiers::COMMAND, Key::F6);
        fronts.push(h.ctx.memory(|m| m.focused()));
        if fronts.last() == Some(&Some(title)) {
            return fronts;
        }
    }
    panic!("Mod+F6 never reached the {kind:?}: {fronts:?}");
}

#[test]
fn mod_f6_moves_between_windows_and_tab_continues_inside_one() {
    let mut h = harness(common::app(Theme::Dark));
    let fronts = to_window(&mut h, WindowKind::ChannelRack);
    assert_eq!(fronts.iter().collect::<std::collections::HashSet<_>>().len(), fronts.len(), "each press goes to another window");
    tab_to(&mut h, "Kick pan");
    let mixer = to_window(&mut h, WindowKind::Mixer);
    assert!(!mixer.is_empty(), "Mod+F6 works from a focused knob too");
}

#[test]
fn the_arrow_keys_turn_a_focused_knob_and_delete_resets_it() {
    let mut h = harness(common::app(Theme::Dark));
    to_window(&mut h, WindowKind::ChannelRack);
    tab_to(&mut h, "Kick pan");
    assert_eq!(channel_pan(&h, "Kick"), 0.0);
    press(&mut h, Key::ArrowUp);
    press(&mut h, Key::ArrowUp);
    let turned = channel_pan(&h, "Kick");
    assert!(turned > 0.0, "up turns the pan right: {turned}");
    press(&mut h, Key::PageDown);
    assert!(channel_pan(&h, "Kick") < turned, "Page Down turns it a big step back");
    press(&mut h, Key::Delete);
    assert_eq!(channel_pan(&h, "Kick"), 0.0, "Delete puts the default back");
    assert!(h.get_by_label("Kick pan").accesskit_node().value().is_some(), "screen readers hear its value");
}

#[test]
fn space_toggles_a_focused_step_and_the_arrow_keys_move_along_the_row() {
    let mut h = harness(common::app(Theme::Dark));
    to_window(&mut h, WindowKind::ChannelRack);
    tab_to(&mut h, "Kick step 1");
    press(&mut h, Key::Space);
    press(&mut h, Key::ArrowRight);
    press(&mut h, Key::ArrowRight);
    assert_eq!(focused(&h).as_deref(), Some("Kick step 3"));
    press(&mut h, Key::Space);
    let app = h.state();
    let project = app.session.project();
    let pattern = app.session.editor().pattern.and_then(|p| project.patterns.get(&p)).unwrap();
    let kick = project.channels.values().find(|c| c.name == "Kick").unwrap().id;
    assert!(bodo_engine::pattern_step_text(project, pattern, kick).starts_with("x.x."));
}

#[test]
fn escape_closes_the_dialogs_and_the_palette() {
    let mut h = harness(common::app(Theme::Dark));
    h.state_mut().view.dialogs.interface = true;
    h.run_steps(2);
    assert!(h.get_by_label("Interface size").accesskit_node().value().is_some());
    press(&mut h, Key::Escape);
    assert!(!h.state().view.dialogs.interface, "Escape closes the interface settings");
    h.state_mut().view.dialogs.about = true;
    h.run_steps(2);
    press(&mut h, Key::Escape);
    assert!(!h.state().view.dialogs.about, "and the About window");
    press_with(&mut h, Modifiers::COMMAND, Key::K);
    assert!(h.state().view.palette.is_some(), "Mod+K opens the palette");
    press(&mut h, Key::Escape);
    assert!(h.state().view.palette.is_none(), "and Escape closes it");
}

#[test]
fn the_size_keys_step_the_interface_and_mod_0_goes_back_to_auto() {
    let mut h = harness(common::app(Theme::Dark));
    assert_eq!(h.ctx.zoom_factor(), 1.0);
    press_with(&mut h, Modifiers::COMMAND, Key::Equals);
    assert_eq!(h.state().prefs.ui_scale, Some(1.1));
    assert!((h.ctx.zoom_factor() - 1.1).abs() < 1e-4, "the interface grows a step");
    press_with(&mut h, Modifiers::COMMAND, Key::Minus);
    press_with(&mut h, Modifiers::COMMAND, Key::Minus);
    assert_eq!(h.state().prefs.ui_scale, Some(0.9));
    press_with(&mut h, Modifiers::COMMAND, Key::Num0);
    assert_eq!(h.state().prefs.ui_scale, None, "Mod+0 is Auto");
    assert!((h.ctx.zoom_factor() - 1.0).abs() < 1e-4, "which is 100% on a 1440-point screen");
}

/// A harness `width` points wide with the interface size on Auto.
fn auto_harness(width: f32) -> Harness<'static, BodoApp> {
    let app = app_with(Prefs { ui_scale: None, ..common::prefs(Theme::Dark) });
    let mut h = Harness::builder().with_size(vec2(width, 900.0)).with_step_dt(1.0 / 60.0).with_max_steps(120).build_ui_state(
        |ui, app: &mut BodoApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(4);
    h
}

#[test]
fn auto_sizes_the_interface_from_the_screens_width() {
    for (width, scale) in [(1440.0, 1.0), (1920.0, 1.35), (1280.0, 0.9)] {
        let h = auto_harness(width);
        assert!((h.state().ui_scale() - scale).abs() < 1e-4, "{width} points wide: {}", h.state().ui_scale());
        assert!((h.ctx.zoom_factor() - scale).abs() < 1e-4);
    }
}

#[test]
fn the_high_contrast_theme_and_the_color_blind_safe_colors_apply_from_their_commands() {
    let mut h = harness(common::app(Theme::Dark));
    h.state_mut().run("ui.set_theme", json!({ "theme": "High contrast" })).unwrap();
    h.run_steps(2);
    assert_eq!(h.state().prefs.theme, ThemePref::HighContrast);
    assert_eq!(Tokens::get(), Tokens::HIGH_CONTRAST);
    assert_eq!(h.ctx.global_style().visuals.panel_fill, Tokens::HIGH_CONTRAST.panel);
    assert_eq!(bodo_kit_ui::Palette::get(&h.ctx), bodo_ui::theme::palette_for(Look::HighContrast, false));
    h.state_mut().run("ui.set_color_safe", json!({ "on": true })).unwrap();
    h.run_steps(2);
    assert!(h.state().prefs.color_safe && bodo_ui::theme::color_safe());
    assert_eq!(bodo_kit_ui::Palette::get(&h.ctx), bodo_ui::theme::palette_for(Look::HighContrast, true));
}

#[test]
fn turkish_shows_in_the_interface_while_the_control_channel_stays_english() {
    let mut h = harness(app_with(Prefs { language: LanguagePref::Turkish, ..common::prefs(Theme::Dark) }));
    h.run_steps(2);
    assert!(h.query_by_label("Dosya").is_some(), "the File menu reads Dosya");
    assert!(h.query_by_label("Kanal rafı").is_some() || h.query_by_label_contains("Kanal rafı").is_some());
    common::element(&h, "Menu File");
    // An undo step the engine names shows by its command's title.
    h.state_mut().run("channel.set_volume", json!({ "channel": "Kick", "volume": 0.5 })).unwrap();
    assert_eq!(h.state().session.undo_labels().last(), Some("Set Kick volume"));
    assert_eq!(h.state().status().map(|(text, _)| text), Some("Kanal ses düzeyini ayarla"));
    h.state_mut().run("ui.set_language", json!({ "language": "en" })).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("File").is_some(), "and back in English");
}

/// Draws the app set up by `setup` with `prefs`, and compares it with snapshot `name`.
fn snapshot(name: &str, prefs: Prefs, setup: fn(&mut BodoApp)) {
    let theme = if prefs.theme == ThemePref::Light { Theme::Light } else { Theme::Dark };
    let mut app = app_with(prefs);
    setup(&mut app);
    let mut h = common::harness(app, theme, true);
    h.run_steps(6);
    h.snapshot_options(name, &common::snapshot_options(name));
}

fn with_beat(app: &mut BodoApp) {
    let script: serde_json::Value = serde_json::from_str(include_str!("../../bodo-engine/tests/fixtures/beat.json")).unwrap();
    let calls: Vec<bodo_engine::Call> = serde_json::from_value(script["calls"].clone()).unwrap();
    for call in calls {
        app.invoke(call).unwrap();
    }
    app.run("ui.select_pattern", json!({ "pattern": "Verse" })).unwrap();
    app.run("ui.open_piano_roll", json!({ "channel": "808" })).unwrap();
}

fn with_sampler(app: &mut BodoApp) {
    app.run("sampler.set", json!({ "channel": "Kick", "loopMode": "forward", "filter": { "kind": "lowPass", "cutoff": 1200 } })).unwrap();
    app.run("ui.open_channel_window", json!({ "channel": "Kick" })).unwrap();
}

fn high_contrast() -> Prefs {
    Prefs { theme: ThemePref::HighContrast, ..common::prefs(Theme::Dark) }
}

fn turkish() -> Prefs {
    Prefs { language: LanguagePref::Turkish, ..common::prefs(Theme::Dark) }
}

#[test]
fn the_main_window_in_high_contrast() {
    snapshot("main_high_contrast", high_contrast(), |_| {});
}

#[test]
fn a_beat_in_high_contrast() {
    snapshot("beat_high_contrast", high_contrast(), with_beat);
}

#[test]
fn the_main_window_in_turkish() {
    snapshot("main_turkish", turkish(), |_| {});
}

#[test]
fn a_beat_in_turkish() {
    snapshot("beat_turkish", turkish(), with_beat);
}

#[test]
fn the_sampler_in_turkish() {
    snapshot("sampler_turkish", turkish(), with_sampler);
}
