//! The widgets driven with real pointer and keyboard events through `egui_kittest`, the way a person uses them.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, reason = "tests")]

use bodo_kit_ui::{Fader, Knob, Led, StepRow, fader};
use egui::{Event, Id, Key, Modifiers, PointerButton, Pos2, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

/// Frames of a sixtieth of a second, so two clicks can make a double-click (kittest's default is a quarter second).
const FRAME: f32 = 1.0 / 60.0;

/// Two rows of 16 steps sharing one paint id, as in a channel rack.
struct Rack {
    rows: Vec<(String, Vec<bool>)>,
}

impl Rack {
    fn new() -> Self {
        Self { rows: vec![("Kick".into(), vec![false; 16]), ("Snare".into(), vec![false; 16])] }
    }

    fn on(&self, row: usize) -> Vec<usize> {
        self.rows[row].1.iter().enumerate().filter(|(_, on)| **on).map(|(i, _)| i).collect()
    }
}

fn rack(state: Rack) -> Harness<'static, Rack> {
    Harness::builder().with_size(vec2(400.0, 120.0)).build_ui_state(
        |ui, rack: &mut Rack| {
            for (label, steps) in &mut rack.rows {
                let edits = StepRow::new(label.as_str(), steps).paint_id(Id::new("rack")).show(ui).edits;
                for (index, on) in edits {
                    steps[index] = on;
                }
            }
        },
        state,
    )
}

fn center(harness: &Harness<'_, Rack>, label: &str) -> Pos2 {
    harness.get_by_label(label).rect().center()
}

/// Presses `button` at `from`, moves through `path` one frame per point, and releases at the last point.
fn drag<S>(harness: &mut Harness<'_, S>, button: PointerButton, from: Pos2, path: &[Pos2]) {
    harness.hover_at(from);
    harness.event(Event::PointerButton { pos: from, button, pressed: true, modifiers: Modifiers::NONE });
    for &point in path {
        harness.hover_at(point);
    }
    let end = path.last().copied().unwrap_or(from);
    harness.event(Event::PointerButton { pos: end, button, pressed: false, modifiers: Modifiers::NONE });
    harness.remove_cursor();
    harness.run();
}

/// Drags a knob or fader: presses at `from` and first moves sideways past egui's drag threshold, a move egui spends
/// on deciding that the press is a drag and these widgets ignore anyway. Then it moves through `path` one frame per
/// point and releases there, with `modifiers` held throughout.
fn drag_widget<S>(harness: &mut Harness<'_, S>, from: Pos2, path: &[Pos2], modifiers: Modifiers) {
    let button = PointerButton::Primary;
    harness.event(Event::ModifiersChanged(modifiers));
    harness.hover_at(from);
    harness.event(Event::PointerButton { pos: from, button, pressed: true, modifiers });
    harness.hover_at(from + vec2(8.0, 0.0));
    for &point in path {
        harness.hover_at(point);
    }
    let end = path.last().copied().unwrap_or(from);
    harness.event(Event::PointerButton { pos: end, button, pressed: false, modifiers });
    harness.event(Event::ModifiersChanged(Modifiers::NONE));
    harness.remove_cursor();
    harness.run();
}

#[test]
fn clicking_a_step_toggles_it() {
    let mut h = rack(Rack::new());
    h.run();
    h.get_by_label("Kick step 5").click();
    h.run();
    assert_eq!(h.state().on(0), [4]);
    h.get_by_label("Kick step 5").click();
    h.run();
    assert!(h.state().on(0).is_empty(), "a second click turns it off");
}

#[test]
fn dragging_paints_a_run_and_right_dragging_clears_it() {
    let mut h = rack(Rack::new());
    h.run();
    let from = center(&h, "Kick step 2");
    let path: Vec<Pos2> = (3..=6).map(|n| center(&h, &format!("Kick step {n}"))).collect();
    drag(&mut h, PointerButton::Primary, from, &path);
    assert_eq!(h.state().on(0), [1, 2, 3, 4, 5]);

    // Pressing a playing step paints steps off.
    let from = center(&h, "Kick step 6");
    let path = [center(&h, "Kick step 5"), center(&h, "Kick step 4")];
    drag(&mut h, PointerButton::Primary, from, &path);
    assert_eq!(h.state().on(0), [1, 2]);

    // The secondary button only ever clears.
    let from = center(&h, "Kick step 1");
    let path = [center(&h, "Kick step 2"), center(&h, "Kick step 3")];
    drag(&mut h, PointerButton::Secondary, from, &path);
    assert!(h.state().on(0).is_empty());
}

#[test]
fn painting_continues_across_rows_that_share_a_paint_id() {
    let mut h = rack(Rack::new());
    h.run();
    let from = center(&h, "Kick step 1");
    let path = [center(&h, "Snare step 1"), center(&h, "Snare step 2"), center(&h, "Snare step 3")];
    drag(&mut h, PointerButton::Primary, from, &path);
    assert_eq!(h.state().on(0), [0]);
    assert_eq!(h.state().on(1), [0, 1, 2]);
}

#[test]
fn keyboard_and_screen_readers_toggle_steps() {
    let mut h = rack(Rack::new());
    h.run();
    h.get_by_label("Snare step 9").click_accesskit();
    h.run();
    assert_eq!(h.state().on(1), [8]);
}

/// A knob and what its responses reported.
#[derive(Default)]
struct KnobState {
    value: f32,
    changed: usize,
    started: usize,
    stopped: usize,
}

fn knob(value: f32) -> Harness<'static, KnobState> {
    Harness::builder().with_size(vec2(100.0, 100.0)).with_step_dt(FRAME).build_ui_state(
        |ui, state: &mut KnobState| {
            ui.add_space(30.0);
            let response = ui.add(Knob::new(&mut state.value, 0.0..=1.0).default(0.5).label("Cutoff").diameter(32.0));
            state.changed += usize::from(response.changed());
            state.started += usize::from(response.drag_started());
            state.stopped += usize::from(response.drag_stopped());
        },
        KnobState { value, ..KnobState::default() },
    )
}

#[test]
fn dragging_a_knob_up_turns_it_up_as_one_gesture() {
    let mut h = knob(0.2);
    h.run();
    let from = h.get_by_label("Cutoff").rect().center();
    let path: Vec<Pos2> = (1..=10).map(|k| from - vec2(0.0, 10.0 * k as f32)).collect();
    drag_widget(&mut h, from, &path, Modifiers::NONE);
    let state = h.state();
    // 100 points of a 200-point sweep is half the range.
    assert!((state.value - 0.7).abs() < 1e-5, "{}", state.value);
    assert!(state.changed >= 10, "{}", state.changed);
    assert_eq!((state.started, state.stopped), (1, 1), "one gesture");
}

#[test]
fn shift_drags_a_knob_finely() {
    let mut h = knob(0.2);
    h.run();
    let from = h.get_by_label("Cutoff").rect().center();
    let path: Vec<Pos2> = (1..=10).map(|k| from - vec2(0.0, 10.0 * k as f32)).collect();
    drag_widget(&mut h, from, &path, Modifiers::SHIFT);
    let value = h.state().value;
    assert!((value - 0.25).abs() < 1e-5, "a tenth as far: {value}");
}

#[test]
fn double_clicking_a_knob_resets_it_and_arrow_keys_step_it() {
    let mut h = knob(0.9);
    h.run();
    let node = h.get_by_label("Cutoff");
    node.click();
    node.click();
    h.run();
    assert_eq!(h.state().value, 0.5);
    h.get_by_label("Cutoff").focus();
    h.run();
    h.key_press(Key::ArrowUp);
    h.key_press(Key::ArrowUp);
    h.run();
    assert!((h.state().value - 0.52).abs() < 1e-5, "{}", h.state().value);
    h.key_press(Key::ArrowLeft);
    h.run();
    assert!((h.state().value - 0.51).abs() < 1e-5, "{}", h.state().value);
}

#[test]
fn the_knob_reports_its_value_to_screen_readers() {
    let mut h = knob(0.25);
    h.run();
    let value = h.get_by_label("Cutoff").accesskit_node().numeric_value();
    assert_eq!(value, Some(0.25));
}

fn fader_harness(db: f32) -> Harness<'static, f32> {
    Harness::builder().with_size(vec2(60.0, 200.0)).with_step_dt(FRAME).build_ui_state(
        |ui, db: &mut f32| {
            ui.add(Fader::new(db).label("Volume").ticks(true));
        },
        db,
    )
}

#[test]
fn a_fader_snaps_to_zero_inside_the_detent_and_resets_on_double_click() {
    let mut h = fader_harness(-12.0);
    h.run();
    let rect = h.get_by_label("Volume").rect();
    // The handle is 14 points tall, so its center travels the height less 14 points.
    let travel = rect.height() - 14.0;
    let y_of = |db: f32| rect.bottom() - 7.0 - fader::db_to_position(db) * travel;
    let from = pos2(rect.center().x, y_of(-12.0));
    let target = y_of(0.0) - fader::DETENT * 0.5 * travel;
    let path: Vec<Pos2> = (1..=8).map(|k| pos2(from.x, from.y + (target - from.y) * k as f32 / 8.0)).collect();
    drag_widget(&mut h, from, &path, Modifiers::NONE);
    assert_eq!(*h.state(), 0.0, "close to 0 dB is exactly 0 dB");

    // Past the detent it moves freely again.
    let from = pos2(rect.center().x, y_of(0.0));
    drag_widget(&mut h, from, &[pos2(from.x, from.y + 20.0)], Modifiers::NONE);
    let db = *h.state();
    assert!(db < -1.0, "{db}");

    h.get_by_label("Volume").click();
    h.get_by_label("Volume").click();
    h.run();
    assert_eq!(*h.state(), 0.0, "double-click resets to 0 dB");
}

#[test]
fn an_led_toggles_on_click() {
    let mut h = Harness::builder().with_size(vec2(40.0, 40.0)).build_ui_state(
        |ui, state: &mut (bool, usize)| {
            let response = ui.add(Led::new(&mut state.0).label("Mute"));
            state.1 += usize::from(response.changed());
        },
        (true, 0),
    );
    h.run();
    h.get_by_label("Mute").click();
    h.run();
    assert_eq!(*h.state(), (false, 1));
}

#[test]
fn page_keys_home_end_and_delete_drive_a_focused_knob() {
    let mut h = knob(0.5);
    h.run();
    h.get_by_label("Cutoff").focus();
    h.run();
    h.key_press(Key::PageUp);
    h.run();
    assert!((h.state().value - 0.6).abs() < 1e-5, "ten steps: {}", h.state().value);
    h.key_press(Key::End);
    h.run();
    assert_eq!(h.state().value, 1.0);
    h.key_press(Key::Home);
    h.run();
    assert_eq!(h.state().value, 0.0);
    h.key_press(Key::Delete);
    h.run();
    assert_eq!(h.state().value, 0.5, "Delete puts the default back");
}

#[test]
fn screen_readers_hear_values_with_their_units() {
    let mut h = knob(0.25);
    h.run();
    assert_eq!(h.get_by_label("Cutoff").accesskit_node().value(), Some("0.25".into()), "the knob's text as it shows it");
    let mut f = fader_harness(-6.0);
    f.run();
    assert_eq!(f.get_by_label("Volume").accesskit_node().value(), Some("-6.0 dB".into()));
}

#[test]
fn a_focused_fader_steps_with_the_keys_and_resets_with_delete() {
    let mut h = fader_harness(-12.0);
    h.run();
    h.get_by_label("Volume").focus();
    h.run();
    h.key_press(Key::PageUp);
    h.run();
    assert!(*h.state() > -12.0, "{}", h.state());
    h.key_press(Key::Delete);
    h.run();
    assert_eq!(*h.state(), 0.0);
}

#[test]
fn arrow_keys_move_between_steps_and_space_toggles() {
    let mut h = rack(Rack::new());
    h.run();
    h.get_by_label("Kick step 1").focus();
    h.run();
    h.key_press(Key::ArrowRight);
    h.run();
    h.key_press(Key::ArrowRight);
    h.run();
    h.key_press(Key::Space);
    h.run();
    assert_eq!(h.state().on(0), [2], "the third step");
}
