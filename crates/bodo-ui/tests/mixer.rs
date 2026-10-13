//! The mixer window (project.md section 9.7), driven with real pointer and key events: strips and their switches,
//! group editing, the route control, meters and clip lights, the insert panel with its slots, EQ, outputs and timing,
//! effect editors, the Browser's plugins, the routing graph, the top bar's loop record, oscilloscope and latency, and
//! the export dialog. Snapshots show the mixer wide with the insert panel, compact, as a graph, an effect editor and
//! the export dialog, in both themes.

#![allow(clippy::unwrap_used, clippy::panic, reason = "tests")]

mod common;

use std::collections::HashMap;

use bodo_mix::MeterReading;
use bodo_model::MixerInsert;
use bodo_model::mixer::{PdcBypass, RouteKey, RouteKind, RouteSource};
use bodo_ui::BodoApp;
use bodo_ui::audio::Readings;
use bodo_ui::windows::WindowKind;
use common::{click, click_at, double_click_at, drag, element, line};
use egui::{Modifiers, PointerButton, Pos2, Theme, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

/// The app with the mixer filling the workspace and the other windows closed, after `setup` ran on it.
fn app_with(theme: Theme, setup: impl FnOnce(&mut BodoApp)) -> BodoApp {
    let mut app = common::app(theme);
    for kind in [WindowKind::ChannelRack, WindowKind::Playlist] {
        let mut state = app.prefs.layout.state(kind);
        state.open = false;
        app.prefs.layout.windows.insert(kind, state);
    }
    let mut state = app.prefs.layout.state(WindowKind::Mixer);
    (state.open, state.maximized) = (true, true);
    app.prefs.layout.windows.insert(WindowKind::Mixer, state);
    setup(&mut app);
    app
}

fn harness(setup: impl FnOnce(&mut BodoApp)) -> Harness<'static, BodoApp> {
    let mut h = common::harness(app_with(Theme::Dark, setup), Theme::Dark, false);
    h.run_steps(4);
    h
}

fn insert(h: &Harness<'_, BodoApp>, number: usize) -> MixerInsert {
    let project = h.state().session.project();
    if number == 0 { (*project.mixer.master).clone() } else { (*project.mixer.inserts[&project.mixer.order[number - 1]]).clone() }
}

fn steps(h: &Harness<'_, BodoApp>) -> usize {
    h.state().session.undo_labels().count()
}

fn right_click(h: &mut Harness<'_, BodoApp>, at: Pos2) {
    h.event(egui::Event::PointerMoved(at));
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        h.step();
    }
    h.step();
}

/// A click with `modifiers` held.
fn click_with(h: &mut Harness<'_, BodoApp>, at: Pos2, modifiers: Modifiers) {
    h.event_modifiers(egui::Event::PointerMoved(at), modifiers);
    h.step();
    for pressed in [true, false] {
        h.event_modifiers(egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers }, modifiers);
        h.step();
    }
    h.step();
}

/// Drags the fader labeled `label` by `dy` points.
fn drag_fader(h: &mut Harness<'_, BodoApp>, label: &str, dy: f32, modifiers: Modifiers) {
    let fader = element(h, label);
    let from = pos2(fader.center().x, fader.top() + fader.height() * 0.3);
    drag(h, PointerButton::Primary, from, &line(from, from + vec2(0.0, dy), 6), modifiers);
}

/// Clicks the widget labeled `label` with the pointer, as a person would, so popups see the press.
fn click_labeled(h: &mut Harness<'_, BodoApp>, label: &str) {
    let at = h.get_by_label(label).rect().center();
    click_at(h, at);
}

/// Clicks the widget whose label contains `text`, such as a submenu button with its arrow.
fn click_containing(h: &mut Harness<'_, BodoApp>, text: &str) {
    let at = h.get_by_label_contains(text).rect().center();
    click_at(h, at);
}

#[test]
fn the_strip_switches_mute_solo_flip_and_swap() {
    let mut h = harness(|_| {});
    click(&mut h, "Insert 2 on");
    assert!(insert(&h, 2).muted);
    click(&mut h, "Insert 2 solo");
    assert!(insert(&h, 2).solo);
    // Cmd/Ctrl click solos one alone.
    let at = element(&h, "Insert 3 solo").center();
    click_with(&mut h, at, Modifiers::COMMAND);
    assert!(insert(&h, 3).solo && !insert(&h, 2).solo, "exclusive solo");
    click(&mut h, "Insert 2 polarity");
    assert!(insert(&h, 2).polarity_invert);
    click(&mut h, "Insert 2 swap");
    assert!(insert(&h, 2).swap_lr);
}

#[test]
fn dragging_a_fader_of_a_group_moves_the_group_in_one_undo_step() {
    let mut h = harness(|_| {});
    let strip = |h: &Harness<'_, BodoApp>, n: usize| {
        let rect = element(h, &format!("Insert {n} strip"));
        pos2(rect.center().x, rect.top() + 16.0)
    };
    let at = strip(&h, 1);
    click_at(&mut h, at);
    let at = strip(&h, 2);
    click_with(&mut h, at, Modifiers::SHIFT);
    assert_eq!(h.state().session.editor().inserts.len(), 2);
    let before = steps(&h);
    drag_fader(&mut h, "Insert 1 volume", 40.0, Modifiers::NONE);
    let (one, two) = (insert(&h, 1).volume_db, insert(&h, 2).volume_db);
    assert!(one < -1.0 && (one - two).abs() < 1e-3, "both faders moved by the same amount: {one} {two}");
    assert!(insert(&h, 3).volume_db.abs() < 1e-6, "a strip outside the group stays");
    assert_eq!(steps(&h), before + 1, "one undo step");
    // Double-clicking a fader puts it back at 0 dB.
    let at = element(&h, "Insert 3 volume").center();
    double_click_at(&mut h, at);
    drag_fader(&mut h, "Insert 3 volume", 30.0, Modifiers::NONE);
    let at = element(&h, "Insert 3 volume").center();
    double_click_at(&mut h, at);
    assert!(insert(&h, 3).volume_db.abs() < 1e-6, "{}", insert(&h, 3).volume_db);
}

#[test]
fn alt_drags_the_selected_faders_along_with_another() {
    let mut h = harness(|_| {});
    let rect = element(&h, "Insert 3 strip");
    click_at(&mut h, pos2(rect.center().x, rect.top() + 16.0));
    drag_fader(&mut h, "Insert 4 volume", 40.0, Modifiers::ALT);
    let (three, four) = (insert(&h, 3).volume_db, insert(&h, 4).volume_db);
    assert!(four < -1.0 && (three - four).abs() < 1e-3, "{three} {four}");
}

#[test]
fn the_route_control_routes_sets_the_level_and_makes_sidechains() {
    let mut h = harness(|_| {});
    let rect = element(&h, "Insert 1 strip");
    click_at(&mut h, pos2(rect.center().x, rect.top() + 16.0));
    let route = |h: &Harness<'_, BodoApp>| {
        let project = h.state().session.project();
        let key = RouteKey { from: RouteSource::Insert(project.mixer.order[0]), to: project.mixer.order[1] };
        project.mixer.routes.get(&key).copied()
    };
    click(&mut h, "Insert 2 route");
    assert_eq!(route(&h).map(|r| (r.kind, r.send_level)), Some((RouteKind::Audio, 1.0)));
    let before = steps(&h);
    let at = element(&h, "Insert 2 route").center();
    drag(&mut h, PointerButton::Primary, at, &line(at, at + vec2(0.0, 30.0), 6), Modifiers::NONE);
    let level = route(&h).unwrap().send_level;
    assert!(level < 0.9, "dragging down lowers the send: {level}");
    assert_eq!(steps(&h), before + 1, "one undo step");
    click(&mut h, "Insert 2 route");
    assert!(route(&h).is_none(), "a click takes the route away");
    let at = element(&h, "Insert 2 route").center();
    click_with(&mut h, at, Modifiers::COMMAND);
    assert_eq!(route(&h).map(|r| r.kind), Some(RouteKind::Sidechain));
    // On the master, the control is whether the selected insert feeds it.
    click(&mut h, "Master route");
    assert!(!insert(&h, 1).to_master);
}

#[test]
fn meters_show_the_engine_and_a_click_clears_the_clip_light() {
    let mut h = harness(|app| {
        let id = app.session.project().mixer.order[0];
        let reading = MeterReading { peak: [0.8, 1.2], clipped: true, correlation: 1.0 };
        app.audio.set_readings(Readings { meters: HashMap::from([(id, reading)]), ..Readings::default() });
    });
    let clip = bodo_ui::control::elements(&h.ctx).into_iter().find(|e| e.label == "Insert 1 clip").unwrap();
    assert_eq!(clip.selected, Some(true));
    click(&mut h, "Insert 1 clip");
    h.run_steps(2);
    let clip = bodo_ui::control::elements(&h.ctx).into_iter().find(|e| e.label == "Insert 1 clip").unwrap();
    assert_eq!(clip.selected, Some(false));
}

#[test]
fn slots_load_switch_move_and_open_their_editor() {
    let mut h = harness(|_| {});
    click(&mut h, "Insert 1 slot 1");
    // The Delay category's heading comes first, then the effect.
    let at = h.get_all_by_label("Delay").last().map(|n| n.rect().center()).unwrap();
    click_at(&mut h, at);
    h.run_steps(3);
    let uid = |h: &Harness<'_, BodoApp>, slot: usize| insert(h, 1).slots[slot].plugin.as_ref().map(|p| p.uid.clone());
    assert_eq!(uid(&h, 0).as_deref(), Some("bodo.delay"));
    click(&mut h, "Insert 1 slot 1 on");
    assert!(!insert(&h, 1).slots[0].enabled);
    // Dragging slot 1 onto slot 4 moves it there.
    let from = element(&h, "Insert 1 slot 1").center() + vec2(30.0, 0.0);
    let to = element(&h, "Insert 1 slot 4").center() + vec2(30.0, 0.0);
    drag(&mut h, PointerButton::Primary, from, &line(from, to, 8), Modifiers::NONE);
    assert_eq!((uid(&h, 0), uid(&h, 3).as_deref()), (None, Some("bodo.delay")));
    // The context menu moves it up again.
    let at = element(&h, "Insert 1 slot 4").center() + vec2(30.0, 0.0);
    right_click(&mut h, at);
    click_labeled(&mut h, "Move up");
    h.run_steps(3);
    assert_eq!(uid(&h, 2).as_deref(), Some("bodo.delay"));
    // A double-click opens its editor.
    let at = element(&h, "Insert 1 slot 3").center() + vec2(30.0, 0.0);
    double_click_at(&mut h, at);
    let instance = insert(&h, 1).slots[2].plugin.as_ref().unwrap().instance;
    assert!(h.state().view.effect_editors.is_open(instance));
}

#[test]
fn the_effect_editor_sets_parameters_bypass_and_makes_automation() {
    let mut h = harness(|app| {
        app.run("fx.load", json!({ "insert": 1, "slot": 1, "effect": "Filter" })).unwrap();
        app.run("edit.select", json!({ "insert": 1 })).unwrap();
    });
    let at = element(&h, "Insert 1 slot 1").center() + vec2(30.0, 0.0);
    double_click_at(&mut h, at);
    h.run_steps(2);
    let info = bodo_mix::fx::find("bodo.filter").unwrap();
    let spec = info.params.iter().find(|p| matches!(p.kind, bodo_mix::fx::ParamKind::Logarithmic | bodo_mix::fx::ParamKind::Linear)).unwrap();
    let label = format!("Filter {}", spec.name);
    let before = steps(&h);
    let knob = element(&h, &label).center();
    drag(&mut h, PointerButton::Primary, knob, &line(knob, knob + vec2(0.0, -40.0), 6), Modifiers::NONE);
    let value = insert(&h, 1).slots[0].param(spec.id).unwrap();
    assert!(value != spec.default, "the knob changed {}: {value}", spec.name);
    assert_eq!(steps(&h), before + 1, "a drag is one undo step");
    click(&mut h, "Insert 1 slot 1 editor on");
    assert!(!insert(&h, 1).slots[0].enabled);
    let at = element(&h, &label).center();
    right_click(&mut h, at);
    click_labeled(&mut h, "Create automation clip");
    h.run_steps(3);
    let made = h.state().session.project().channels.values().any(|c| matches!(c.kind, bodo_model::channel::ChannelKind::Automation(_)));
    assert!(made, "an automation channel for the parameter");
}

#[test]
fn the_panel_sets_the_eq_outputs_track_latency_and_pdc_bypass() {
    let mut h = harness(|app| {
        app.run("edit.select", json!({ "insert": 2 })).unwrap();
    });
    let before = steps(&h);
    let knob = element(&h, "Insert 2 EQ Mid gain").center();
    drag(&mut h, PointerButton::Primary, knob, &line(knob, knob + vec2(0.0, -30.0), 6), Modifiers::NONE);
    assert!(insert(&h, 2).eq.mid.gain_db > 1.0, "{}", insert(&h, 2).eq.mid.gain_db);
    assert_eq!(steps(&h), before + 1);
    click(&mut h, "Panel to master");
    assert!(!insert(&h, 2).to_master);
    click(&mut h, "Panel direct out");
    click_labeled(&mut h, "Out 3-4");
    h.run_steps(3);
    assert_eq!(insert(&h, 2).output.map(|p| p.channels), Some(vec![2, 3]));
    click(&mut h, "Panel PDC bypass");
    click_labeled(&mut h, "While recording");
    h.run_steps(3);
    assert_eq!(insert(&h, 2).pdc_bypass, PdcBypass::WhileRecording);
    let at = element(&h, "Panel track latency").center();
    drag(&mut h, PointerButton::Primary, at, &line(at, at + vec2(30.0, 0.0), 4), Modifiers::NONE);
    assert!(insert(&h, 2).latency_offset.0 > 0, "{:?}", insert(&h, 2).latency_offset);
}

#[test]
fn the_strip_menu_sets_an_icon_and_automates_the_fader() {
    let mut h = harness(|_| {});
    let rect = element(&h, "Insert 2 strip");
    right_click(&mut h, pos2(rect.center().x, rect.top() + 16.0));
    click_containing(&mut h, "Icon");
    h.run_steps(2);
    click_labeled(&mut h, "drum");
    h.run_steps(3);
    assert_eq!(insert(&h, 2).icon.as_deref(), Some("drum"));
    let at = element(&h, "Insert 2 volume").center();
    right_click(&mut h, at);
    click_labeled(&mut h, "Create automation clip");
    h.run_steps(3);
    assert!(h.state().session.project().channels.values().any(|c| c.name.contains("Insert 2")), "an automation channel named after the fader");
}

#[test]
fn effects_from_the_browser_load_by_drag_and_double_click() {
    let mut h = harness(|app| {
        app.run("edit.select", json!({ "insert": 1 })).unwrap();
    });
    click(&mut h, "Browser tab Plugins");
    h.run_steps(2);
    click(&mut h, "Folder Plugin database");
    h.run_steps(2);
    let from = element(&h, "Plugin Delay").center();
    let to = element(&h, "Insert 2 strip").center();
    drag(&mut h, PointerButton::Primary, from, &line(from, to, 10), Modifiers::NONE);
    assert_eq!(insert(&h, 2).slots[0].plugin.as_ref().map(|p| p.uid.as_str()), Some("bodo.delay"));
    let at = element(&h, "Plugin Filter").center();
    double_click_at(&mut h, at);
    assert_eq!(insert(&h, 1).slots[0].plugin.as_ref().map(|p| p.uid.as_str()), Some("bodo.filter"));
}

#[test]
fn the_routing_graph_selects_inserts_and_the_sizes_change_the_strips() {
    let mut h = harness(|app| {
        app.run("mixer.add_route", json!({ "from": "insert:1", "to": "insert:2" })).unwrap();
    });
    click(&mut h, "Routing graph");
    h.run_steps(2);
    click(&mut h, "Graph Insert 2");
    let second = h.state().session.project().mixer.order[1];
    assert_eq!(h.state().session.editor().insert, Some(second));
    click(&mut h, "Routing graph");
    click(&mut h, "Mixer size Compact");
    assert_eq!(h.state().view.mixer.size, bodo_ui::mixer::StripSize::Compact);
    let compact = element(&h, "Insert 1 strip").width();
    click(&mut h, "Mixer size Extra compact");
    assert!(element(&h, "Insert 1 strip").width() < compact);
}

#[test]
fn the_top_bar_has_loop_record_the_oscilloscope_and_latency() {
    let mut h = harness(|app| {
        let scope: Vec<f32> = (0..1024).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
        app.audio.set_readings(Readings { scope, latency: Some(480), ..Readings::default() });
    });
    click(&mut h, "Loop record");
    assert!(h.state().session.transport().loop_record);
    element(&h, "Oscilloscope");
    h.get_by_label_contains("10.0 ms");
}

#[test]
fn the_export_dialog_writes_the_mix_and_the_stems() {
    let dir = std::env::temp_dir().join(format!("bodo-ui-export-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut h = common::harness(common::beat_app(), Theme::Dark, false);
    h.run_steps(3);
    click_labeled(&mut h, "File");
    h.run_steps(2);
    click_labeled(&mut h, "Export mix...");
    h.run_steps(2);
    let dialog = h.state_mut().view.dialogs.export.as_mut().unwrap();
    dialog.folder = dir.display().to_string();
    dialog.name = String::from("Beat");
    h.run_steps(2);
    click(&mut h, "Export run");
    assert!(dir.join("Beat.wav").is_file());
    // The export ran inside the last frame; let the dialog settle before the next click.
    h.run_steps(3);
    click(&mut h, "Export Stems and the mix");
    click(&mut h, "Export format FLAC");
    click(&mut h, "Export run");
    let said = h.state().view.dialogs.export.as_ref().and_then(|d| d.message.clone());
    assert!(dir.join("Beat stems").join("Mix.flac").is_file(), "{said:?}");
    let stems = std::fs::read_dir(dir.join("Beat stems")).unwrap().count();
    assert!(stems > 2, "{stems} files");
    let _ = std::fs::remove_dir_all(dir);
}

/// What a snapshot shows.
#[derive(Clone, Copy)]
enum Scene {
    Wide,
    Compact,
    Graph,
    Editor,
    Export,
}

fn scene(app: &mut BodoApp, scene: Scene) {
    let (first, second, fourth) = {
        let order = &app.session.project().mixer.order;
        (order[0], order[1], order[3])
    };
    app.run("fx.load", json!({ "insert": 1, "slot": 1, "effect": "Filter" })).unwrap();
    app.run("fx.load", json!({ "insert": 1, "slot": 2, "effect": "Delay" })).unwrap();
    app.run("fx.load", json!({ "insert": 2, "slot": 1, "effect": "Ducker" })).unwrap();
    app.run("fx.set_sidechain", json!({ "insert": 2, "slot": 1, "from": "insert:1" })).unwrap();
    app.run("mixer.add_route", json!({ "from": "insert:1", "to": "insert:4", "level": 0.6 })).unwrap();
    app.run("mixer.set_eq", json!({ "insert": 1, "band": "mid", "gainDb": 4.0, "frequency": 900, "q": 1.2 })).unwrap();
    app.run("mixer.set_eq", json!({ "insert": 1, "band": "low", "gainDb": -3.0 })).unwrap();
    app.run("mixer.set_icon", json!({ "insert": 1, "icon": "drum" })).unwrap();
    app.run("mixer.solo", json!({ "insert": 3, "on": true })).unwrap();
    app.run("edit.select", json!({ "insert": 1 })).unwrap();
    let reading = |l: f32, r: f32, clipped: bool| MeterReading { peak: [l, r], clipped, correlation: 1.0 };
    let meters = HashMap::from([(first, reading(0.7, 0.6, false)), (second, reading(0.3, 0.35, false)), (fourth, reading(1.1, 0.9, true))]);
    app.audio.set_readings(Readings { meters, ..Readings::default() });
    match scene {
        Scene::Wide => {}
        Scene::Compact => app.view.mixer.size = bodo_ui::mixer::StripSize::Compact,
        Scene::Graph => app.view.mixer.graph = true,
        Scene::Editor => bodo_ui::effect_editor::open(app, None, first, 0),
        Scene::Export => {
            app.run("ui.export", json!({ "what": "stems" })).unwrap();
            // A folder that reads the same on every machine.
            if let Some(dialog) = app.view.dialogs.export.as_mut() {
                (dialog.folder, dialog.name) = (String::from("/Users/me/Music/Bodo/Rendered"), String::from("Beat"));
            }
        }
    }
}

fn snapshot(name: &str, theme: Theme, which: Scene) {
    let app = app_with(theme, |app| scene(app, which));
    let mut h = common::harness(app, theme, true);
    h.run_steps(8);
    h.snapshot_options(name, &common::snapshot_options(name));
}

#[test]
fn the_mixer_wide_dark() {
    snapshot("mixer_wide_dark", Theme::Dark, Scene::Wide);
}

#[test]
fn the_mixer_wide_light() {
    snapshot("mixer_wide_light", Theme::Light, Scene::Wide);
}

#[test]
fn the_mixer_compact_dark() {
    snapshot("mixer_compact_dark", Theme::Dark, Scene::Compact);
}

#[test]
fn the_mixer_compact_light() {
    snapshot("mixer_compact_light", Theme::Light, Scene::Compact);
}

#[test]
fn the_routing_graph_dark() {
    snapshot("mixer_graph_dark", Theme::Dark, Scene::Graph);
}

#[test]
fn the_routing_graph_light() {
    snapshot("mixer_graph_light", Theme::Light, Scene::Graph);
}

#[test]
fn an_effect_editor_dark() {
    snapshot("effect_editor_dark", Theme::Dark, Scene::Editor);
}

#[test]
fn an_effect_editor_light() {
    snapshot("effect_editor_light", Theme::Light, Scene::Editor);
}

#[test]
fn the_export_dialog_dark() {
    snapshot("export_dark", Theme::Dark, Scene::Export);
}

#[test]
fn the_export_dialog_light() {
    snapshot("export_light", Theme::Light, Scene::Export);
}
