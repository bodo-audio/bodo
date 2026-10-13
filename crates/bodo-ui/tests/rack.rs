//! The Channel Rack's header and menus (project.md section 10.2), driven with real pointer and key events: the options
//! menu, groups and the group filter, the group swing knob, reordering by dragging, colors, MIDI inputs, compact rows
//! and the graph editor. Two snapshots show the header with a group in the dark and light themes.

#![allow(clippy::unwrap_used, clippy::panic, reason = "tests")]

mod common;

use bodo_model::{ChannelId, Note};
use bodo_ui::{BodoApp, GroupFilter};
use common::{click, click_at, drag, element, line};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Theme, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

fn harness() -> Harness<'static, BodoApp> {
    let mut h = common::harness(common::app(Theme::Dark), Theme::Dark, false);
    h.run_steps(4);
    h
}

fn channel(h: &Harness<'_, BodoApp>, name: &str) -> ChannelId {
    h.state().session.project().channels.values().find(|c| c.name == name).unwrap().id
}

fn names(h: &Harness<'_, BodoApp>) -> Vec<String> {
    let project = h.state().session.project();
    project.rack_order.iter().map(|c| project.channels[c].name.clone()).collect()
}

fn step_text(h: &Harness<'_, BodoApp>, name: &str) -> String {
    let project = h.state().session.project();
    let pattern = &project.patterns[&h.state().session.editor().pattern.unwrap()];
    bodo_engine::pattern_step_text(project, pattern, channel(h, name))
}

fn steps(h: &Harness<'_, BodoApp>) -> usize {
    h.state().session.undo_labels().count()
}

fn right_click(h: &mut Harness<'_, BodoApp>, at: Pos2) {
    h.event(Event::PointerMoved(at));
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        h.step();
    }
    h.step();
}

/// Clicks the channel name buttons `names`, with Shift after the first, so they are all selected.
fn select(h: &mut Harness<'_, BodoApp>, names: &[&str]) {
    for (i, name) in names.iter().enumerate() {
        let at = element(h, name).center();
        let modifiers = if i == 0 { Modifiers::NONE } else { Modifiers::SHIFT };
        h.event(Event::PointerMoved(at));
        h.step();
        for pressed in [true, false] {
            h.event_modifiers(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers }, modifiers);
            h.step();
        }
        h.run_steps(20);
    }
}

#[test]
fn the_options_menu_fills_and_clears_steps_in_one_step_each() {
    let mut h = harness();
    let before = steps(&h);
    click(&mut h, "Channel rack options");
    h.get_by_label("Fill each 4 steps").click();
    h.run_steps(3);
    for name in ["Kick", "Clap", "HiHat", "Snare"] {
        assert_eq!(step_text(&h, name), "x...x...x...x...", "{name}");
    }
    assert_eq!(steps(&h), before + 1, "filling every channel is one undo step");
    select(&mut h, &["HiHat"]);
    click(&mut h, "Channel rack options");
    h.get_by_label("Clear steps").click();
    h.run_steps(3);
    assert_eq!(step_text(&h, "HiHat"), "................", "only the selected channel");
    assert_eq!(step_text(&h, "Kick"), "x...x...x...x...");
}

#[test]
fn a_new_group_filters_the_rack_and_its_swing_knob_sets_every_channel_in_it() {
    let mut h = harness();
    select(&mut h, &["Kick", "Snare"]);
    click(&mut h, "Channel rack options");
    h.get_by_label_contains("Move to group").hover();
    h.run_steps(3);
    h.get_by_label("New group...").click();
    h.run_steps(3);
    for _ in 0..8 {
        h.key_press(Key::Backspace);
    }
    h.event(Event::Text(String::from("Drums")));
    h.key_press(Key::Enter);
    h.run_steps(3);
    let groups: Vec<(String, Option<String>)> = h.state().session.project().channels.values().map(|c| (c.name.clone(), c.group.clone())).collect();
    for (name, group) in &groups {
        let expected = if name == "Kick" || name == "Snare" { Some("Drums") } else { None };
        assert_eq!(group.as_deref(), expected, "{name}");
    }
    assert_eq!(h.state().view.rack.filter, GroupFilter::Group(String::from("Drums")));
    let shown: Vec<String> = bodo_ui::control::elements(&h.ctx).iter().filter(|e| e.role == "channel").map(|e| e.label.clone()).collect();
    assert_eq!(shown, ["Kick", "Snare"], "the filter shows the group");

    let before = steps(&h);
    let knob = element(&h, "Channel rack swing").center();
    drag(&mut h, PointerButton::Primary, knob, &line(knob, knob - vec2(0.0, 60.0), 8), Modifiers::NONE);
    let project = h.state().session.project();
    let swings: Vec<(String, f32)> = project.channels.values().map(|c| (c.name.clone(), c.swing)).collect();
    let kick = swings.iter().find(|(n, _)| n == "Kick").unwrap().1;
    assert!(kick > 0.05, "{swings:?}");
    for (name, swing) in &swings {
        let expected = if name == "Kick" || name == "Snare" { kick } else { 0.0 };
        assert_eq!(*swing, expected, "{name}");
    }
    assert_eq!(steps(&h), before + 1, "a swing drag is one undo step");
}

#[test]
fn dragging_a_name_button_moves_the_channel() {
    let mut h = harness();
    let snare = element(&h, "Snare").center();
    let kick = element(&h, "Kick");
    let target = pos2(snare.x, kick.top() + 3.0);
    drag(&mut h, PointerButton::Primary, snare, &line(snare, target, 10), Modifiers::NONE);
    assert_eq!(names(&h), ["Snare", "Kick", "Clap", "HiHat"]);
    let clap = element(&h, "Clap");
    let snare = element(&h, "Snare").center();
    drag(&mut h, PointerButton::Primary, snare, &line(snare, pos2(snare.x, clap.bottom() - 3.0), 10), Modifiers::NONE);
    assert_eq!(names(&h), ["Kick", "Clap", "Snare", "HiHat"], "dropped on the lower half of a row, it lands after it");
}

#[test]
fn the_channel_menu_sets_the_color_and_the_midi_input() {
    let mut h = harness();
    h.state_mut().view.rack.midi_ports = Some(vec![String::from("Test Keys")]);
    let clap = element(&h, "Clap").center();
    right_click(&mut h, clap);
    h.get_by_label_contains("Color").hover();
    h.run_steps(3);
    let hue = bodo_engine::palette::HUES[3];
    click(&mut h, &format!("Color {hue}"));
    h.run_steps(2);
    assert_eq!(h.state().session.project().channels[&channel(&h, "Clap")].color, hue);
    let clap = element(&h, "Clap").center();
    right_click(&mut h, clap);
    h.get_by_label_contains("MIDI input").hover();
    h.run_steps(3);
    h.get_by_label_contains("Test Keys").hover();
    h.run_steps(3);
    h.get_by_label("MIDI channel 2").click();
    h.run_steps(3);
    let binding = h.state().session.project().channels[&channel(&h, "Clap")].midi_in.clone().unwrap();
    assert_eq!((binding.port.as_str(), binding.channel), ("Test Keys", Some(1)), "MIDI channel 2 is channel index 1");
}

#[test]
fn compact_rows_are_smaller() {
    let mut h = harness();
    let tall = element(&h, "Kick").height();
    click(&mut h, "Compact rows");
    h.run_steps(2);
    assert!(element(&h, "Kick").height() < tall);
    assert!(h.state().view.rack.compact);
}

#[test]
fn the_graph_editor_sets_each_steps_velocity_and_pitch_with_a_stroke() {
    let mut h = harness();
    common::steps(&mut h, "Kick", &[1, 5, 9]);
    select(&mut h, &["Kick"]);
    click(&mut h, "Graph editor");
    h.run_steps(3);
    let area = element(&h, "Graph editor");
    let first = element(&h, "Kick step 1 velocity");
    let last = element(&h, "Kick step 9 velocity");
    let before = steps(&h);
    let y = area.bottom() - 0.25 * area.height();
    drag(&mut h, PointerButton::Primary, pos2(first.center().x, y), &line(pos2(first.center().x, y), pos2(last.center().x, y), 10), Modifiers::NONE);
    let notes = kick_notes(&h);
    assert_eq!(notes.len(), 3);
    assert!(notes.iter().all(|n| (f32::from(n.velocity) / f32::from(u16::MAX) - 0.25).abs() < 0.04), "{notes:?}");
    assert_eq!(steps(&h), before + 1, "a stroke is one undo step");

    h.state_mut().view.rack.graph_param = bodo_ui::StepParam::Pitch;
    h.run_steps(2);
    let top = area.top() + 1.0;
    drag(
        &mut h,
        PointerButton::Primary,
        pos2(first.center().x, top),
        &line(pos2(first.center().x, top), pos2(first.center().x + 2.0, top), 2),
        Modifiers::NONE,
    );
    let keys: Vec<u8> = kick_notes(&h).iter().map(|n| n.key).collect();
    assert_eq!(keys[0], 60 + 12, "the top of the pitch graph is an octave up from the root key: {keys:?}");
}

fn kick_notes(h: &Harness<'_, BodoApp>) -> Vec<Note> {
    let project = h.state().session.project();
    let pattern = &project.patterns[&h.state().session.editor().pattern.unwrap()];
    pattern.notes.get(&channel(h, "Kick")).map(|seq| seq.notes().iter().cloned().collect()).unwrap_or_default()
}

fn options(name: &str) -> egui_kittest::SnapshotOptions {
    let os_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots").join(std::env::consts::OS);
    let options = egui_kittest::SnapshotOptions::new()
        .threshold(egui_kittest::OsThreshold::new(1.5).windows(2.0))
        .max_failed_pixels(egui_kittest::OsThreshold::new(400).windows(2000));
    if os_dir.join(format!("{name}.png")).exists() { options.output_path(os_dir) } else { options }
}

/// The rack filtered to a Drums group with swing and the graph editor open, in `theme`.
fn snapshot(name: &str, theme: Theme) {
    let mut app = common::app(theme);
    app.run("channel.set_group", serde_json::json!({ "channels": ["Kick", "Snare"], "group": "Drums" })).unwrap();
    app.run("channel.set_swing", serde_json::json!({ "channels": ["Kick", "Snare"], "swing": 0.4 })).unwrap();
    app.run("pattern.set_steps", serde_json::json!({ "channel": "Kick", "steps": "x...x...x...x..." })).unwrap();
    app.run("edit.select", serde_json::json!({ "channel": "Kick" })).unwrap();
    app.view.rack.filter = GroupFilter::Group(String::from("Drums"));
    app.view.rack.graph = true;
    let mut harness = common::harness(app, theme, true);
    harness.run_steps(6);
    harness.snapshot_options(name, &options(name));
}

#[test]
fn the_rack_with_a_group_dark() {
    snapshot("rack_group_dark", Theme::Dark);
}

#[test]
fn the_rack_with_a_group_light() {
    snapshot("rack_group_light", Theme::Light);
}

#[test]
fn clicking_the_group_filter_shows_every_channel_again() {
    let mut h = harness();
    h.state_mut().run("channel.set_group", serde_json::json!({ "channels": ["Kick"], "group": "Drums" })).unwrap();
    h.state_mut().view.rack.filter = GroupFilter::Group(String::from("Drums"));
    h.run_steps(2);
    click(&mut h, "Channel rack group");
    h.run_steps(2);
    h.get_by_label("All channels").click();
    h.run_steps(2);
    assert_eq!(h.state().view.rack.filter, GroupFilter::All);
    let filter = element(&h, "Channel rack group").center();
    click_at(&mut h, filter);
    h.run_steps(2);
    h.get_by_label("Unsorted").click();
    h.run_steps(2);
    let shown: Vec<String> = bodo_ui::control::elements(&h.ctx).iter().filter(|e| e.role == "channel").map(|e| e.label.clone()).collect();
    assert_eq!(shown, ["Clap", "HiHat", "Snare"]);
}
