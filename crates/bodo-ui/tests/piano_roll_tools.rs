//! The piano roll's tools (project.md section 10.4), driven with real pointer and key events: the playlist's tools
//! on notes, the lanes, the note tools menu, the stamp, the chord readout, ghost notes and the tool keys. Each test
//! checks what the tool does to the notes, that a gesture is one undo step, and that view-only tools stay out of the
//! history.

#![allow(clippy::unwrap_used, clippy::panic, reason = "tests")]

mod common;

use bodo_engine::PlayMode;
use bodo_kit_core::Tick;
use bodo_model::{ChannelId, Note};
use bodo_ui::BodoApp;
use bodo_ui::windows::WindowKind;
use common::{click_at, drag, element, line};
use egui::{Key, Modifiers, PointerButton, Pos2, Rect, Theme, pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

/// Points per beat in these tests.
const BEAT: f32 = 48.0;
/// Points per key.
const KEY: f32 = 14.0;
/// The key at the top of the grid.
const TOP: f32 = 72.0;

/// A fresh project with the piano roll filling the workspace on the Kick channel, and `tool` chosen.
fn setup(tool: &str) -> (Harness<'static, BodoApp>, Rect) {
    let mut app = common::app(Theme::Dark);
    app.run("ui.open_piano_roll", json!({ "channel": "Kick" })).unwrap();
    let mut state = app.prefs.layout.state(WindowKind::PianoRoll);
    state.maximized = true;
    app.prefs.layout.windows.insert(WindowKind::PianoRoll, state);
    let view = &mut app.view.piano;
    (view.zoom, view.scroll, view.top_key, view.key_height) = (BEAT / 960.0, 0.0, TOP, KEY);
    app.run("ui.set_tool", json!({ "tool": tool, "window": "pianoRoll" })).unwrap();
    let mut harness = common::harness(app, Theme::Dark, false);
    harness.run_steps(4);
    let canvas = element(&harness, "Piano roll grid");
    (harness, canvas)
}

/// The middle of key `key`'s row at `beat` (counted from 1, fractions allowed).
fn at(canvas: Rect, beat: f32, key: u8) -> Pos2 {
    pos2(canvas.left() + (beat - 1.0) * BEAT, canvas.top() + (TOP - f32::from(key)) * KEY + KEY / 2.0)
}

fn channel(h: &Harness<'_, BodoApp>, name: &str) -> ChannelId {
    h.state().session.project().channels.values().find(|c| c.name == name).unwrap().id
}

/// The notes of `name` in the current pattern, in time order.
fn notes_of(h: &Harness<'_, BodoApp>, name: &str) -> Vec<Note> {
    let project = h.state().session.project();
    let pattern = &project.patterns[&h.state().session.editor().pattern.unwrap()];
    pattern.notes.get(&channel(h, name)).map(|seq| seq.notes().iter().cloned().collect()).unwrap_or_default()
}

fn kick(h: &Harness<'_, BodoApp>) -> Vec<Note> {
    notes_of(h, "Kick")
}

/// `(key, start, length)` of each Kick note.
fn shape(h: &Harness<'_, BodoApp>) -> Vec<(u8, i64, i64)> {
    kick(h).iter().map(|n| (n.key, n.start.0, n.len.0)).collect()
}

fn steps(h: &Harness<'_, BodoApp>) -> usize {
    h.state().session.undo_labels().count()
}

/// Adds Kick notes `(key, start, length)` through the session, as an agent would.
fn add(h: &mut Harness<'_, BodoApp>, notes: &[(u8, i64, i64)]) {
    let notes: Vec<_> = notes.iter().map(|(key, start, length)| json!({ "key": key, "start": start, "length": length })).collect();
    h.state_mut().run("note.add_many", json!({ "channel": "Kick", "notes": notes })).unwrap();
    h.state_mut().run("edit.deselect", json!({})).unwrap();
    h.run_steps(2);
}

fn right_click(h: &mut Harness<'_, BodoApp>, at: Pos2) {
    h.event(egui::Event::PointerMoved(at));
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        h.step();
    }
}

#[test]
fn draw_adds_moves_resizes_deletes_and_alt_turns_snap_off() {
    let (mut h, canvas) = setup("draw");
    let before = steps(&h);
    // A click adds a step-long note at the beat, the snap.
    click_at(&mut h, at(canvas, 1.3, 60));
    assert_eq!(shape(&h), [(60, 0, 240)]);
    assert_eq!(steps(&h), before + 1);
    // Dragging before release moves the new note: still one step.
    drag(&mut h, PointerButton::Primary, at(canvas, 3.2, 62), &line(at(canvas, 3.2, 62), at(canvas, 4.2, 64), 5), Modifiers::NONE);
    assert_eq!(shape(&h), [(60, 0, 240), (64, 2880, 240)]);
    assert_eq!(steps(&h), before + 2);
    // Dragging a note moves it in time and key, one undo step for the whole drag.
    drag(&mut h, PointerButton::Primary, at(canvas, 1.1, 60), &line(at(canvas, 1.1, 60), at(canvas, 2.1, 57), 6), Modifiers::NONE);
    assert_eq!(shape(&h), [(57, 960, 240), (64, 2880, 240)]);
    assert_eq!(steps(&h), before + 3);
    // Dragging a note's right edge resizes it, and new notes take that length.
    let edge = pos2(at(canvas, 2.0, 57).x + 240.0 * BEAT / 960.0 - 2.0, at(canvas, 2.0, 57).y);
    drag(&mut h, PointerButton::Primary, edge, &line(edge, at(canvas, 4.0, 57), 6), Modifiers::NONE);
    assert_eq!(shape(&h)[0], (57, 960, 1920));
    assert_eq!(h.state().view.piano.note_length, Tick(1920));
    assert_eq!(steps(&h), before + 4);
    // Right-click deletes.
    right_click(&mut h, at(canvas, 2.5, 57));
    assert_eq!(shape(&h), [(64, 2880, 240)]);
    // Alt turns snap off.
    drag(&mut h, PointerButton::Primary, at(canvas, 5.3, 50), &[], Modifiers::ALT);
    let start = kick(&h).iter().find(|n| n.key == 50).unwrap().start.0;
    assert!(start % 960 != 0, "unsnapped start {start}");
}

#[test]
fn paint_lays_notes_along_the_drag_and_right_drag_deletes_them() {
    let (mut h, canvas) = setup("paint");
    let before = steps(&h);
    drag(&mut h, PointerButton::Primary, at(canvas, 1.1, 60), &line(at(canvas, 1.1, 60), at(canvas, 2.9, 60), 12), Modifiers::NONE);
    let starts: Vec<i64> = kick(&h).iter().map(|n| n.start.0).collect();
    assert_eq!(starts, [0, 240, 480, 720, 960, 1200, 1440, 1680], "a step-long note each step from the beat");
    assert_eq!(steps(&h), before + 1, "a stroke is one undo step");
    drag(&mut h, PointerButton::Secondary, at(canvas, 1.1, 60), &line(at(canvas, 1.1, 60), at(canvas, 2.9, 60), 24), Modifiers::NONE);
    assert!(kick(&h).is_empty());
    assert_eq!(steps(&h), before + 2);
}

#[test]
fn delete_removes_the_notes_under_the_drag() {
    let (mut h, canvas) = setup("delete");
    add(&mut h, &[(60, 0, 240), (60, 960, 240), (62, 960, 240), (60, 2880, 240)]);
    let before = steps(&h);
    drag(&mut h, PointerButton::Primary, at(canvas, 1.1, 60), &line(at(canvas, 1.1, 60), at(canvas, 2.1, 60), 12), Modifiers::NONE);
    assert_eq!(shape(&h), [(62, 960, 240), (60, 2880, 240)]);
    assert_eq!(steps(&h), before + 1);
}

#[test]
fn mute_toggles_notes() {
    let (mut h, canvas) = setup("mute");
    add(&mut h, &[(60, 0, 480)]);
    click_at(&mut h, at(canvas, 1.2, 60));
    assert!(kick(&h)[0].muted);
    click_at(&mut h, at(canvas, 1.2, 60));
    assert!(!kick(&h)[0].muted);
}

#[test]
fn slice_cuts_the_notes_the_line_crosses_and_shift_slants_it() {
    let (mut h, canvas) = setup("slice");
    add(&mut h, &[(60, 0, 1920), (64, 0, 1920), (67, 0, 1920)]);
    let before = steps(&h);
    // Straight down at beat 2 through C4 and E4.
    drag(&mut h, PointerButton::Primary, at(canvas, 2.0, 64), &line(at(canvas, 2.0, 64), at(canvas, 2.0, 60), 4), Modifiers::NONE);
    assert_eq!(shape(&h), [(60, 0, 960), (64, 0, 960), (67, 0, 1920), (60, 960, 960), (64, 960, 960)]);
    assert_eq!(steps(&h), before + 1);
    // A slanted cut with Shift, snap off with Alt: from beat 1.5 on G4 to beat 1.9 on E4 cuts each where the line
    // crosses its row.
    let slant = Modifiers::SHIFT | Modifiers::ALT;
    drag(&mut h, PointerButton::Primary, at(canvas, 1.5, 67), &line(at(canvas, 1.5, 67), at(canvas, 1.9, 64), 4), slant);
    let on = |key: u8| -> Vec<(i64, i64)> { kick(&h).iter().filter(|n| n.key == key).map(|n| (n.start.0, n.len.0)).collect() };
    assert_eq!(on(67), [(0, 480), (480, 1440)]);
    assert_eq!(on(64), [(0, 864), (864, 96), (960, 960)]);
    assert_eq!(on(60), [(0, 960), (960, 960)], "C4 is below the line");
    assert_eq!(steps(&h), before + 2);
}

#[test]
fn select_boxes_notes_shift_toggles_and_the_history_stays_put() {
    let (mut h, canvas) = setup("select");
    add(&mut h, &[(60, 0, 240), (62, 960, 240), (64, 1920, 240)]);
    let before = steps(&h);
    drag(&mut h, PointerButton::Primary, at(canvas, 1.0, 63), &line(at(canvas, 1.0, 63), at(canvas, 2.5, 59), 5), Modifiers::NONE);
    assert_eq!(h.state().session.editor().notes.len(), 2);
    drag(&mut h, PointerButton::Primary, at(canvas, 3.0, 65), &line(at(canvas, 3.0, 65), at(canvas, 3.5, 63), 3), Modifiers::SHIFT);
    assert_eq!(h.state().session.editor().notes.len(), 3);
    drag(&mut h, PointerButton::Primary, at(canvas, 1.1, 60), &line(at(canvas, 1.1, 60), at(canvas, 1.15, 60), 2), Modifiers::SHIFT);
    assert_eq!(h.state().session.editor().notes.len(), 2, "Shift takes C4 out again");
    assert_eq!(steps(&h), before, "selection never enters the undo history");
}

#[test]
fn zoom_boxes_into_time_and_keys_and_changes_nothing_else() {
    let (mut h, canvas) = setup("zoom");
    let project = h.state().session.project().clone();
    drag(&mut h, PointerButton::Primary, at(canvas, 3.0, 66), &line(at(canvas, 3.0, 66), at(canvas, 5.0, 60), 4), Modifiers::NONE);
    let view = &h.state().view.piano;
    let beats = canvas.width() / view.zoom / 960.0;
    assert!((beats - 2.0).abs() < 0.05, "two beats fill the grid: {beats}");
    assert!((view.scroll - 1920.0).abs() < 1.0, "the view starts at beat 3: {}", view.scroll);
    assert!(view.key_height > KEY, "keys got taller: {}", view.key_height);
    assert!(std::sync::Arc::ptr_eq(h.state().session.project(), &project), "zooming is not an edit");
}

#[test]
fn playback_plays_the_pattern_from_the_pointer_while_held() {
    let (mut h, canvas) = setup("playback");
    let from = at(canvas, 3.1, 60);
    h.event(egui::Event::PointerMoved(from));
    h.step();
    h.event(egui::Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    let transport = *h.state().session.transport();
    assert!(transport.playing && transport.mode == PlayMode::Pattern);
    assert_eq!(transport.position, Tick(1920));
    h.event(egui::Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert!(!h.state().session.transport().playing, "releasing stops");
    assert!(kick(&h).is_empty(), "nothing was edited");
}

#[test]
fn a_lane_stroke_sets_each_note_it_crosses_in_one_undo_step() {
    let (mut h, canvas) = setup("draw");
    add(&mut h, &[(60, 0, 240), (62, 960, 240), (64, 1920, 240)]);
    let lane = element(&h, "Piano roll lane");
    let before = steps(&h);
    // A level stroke near the bottom of the lane, across all three stems.
    let y = lane.bottom() - 0.25 * lane.height();
    let from = pos2(canvas.left() - 0.0 + 2.0, y);
    drag(&mut h, PointerButton::Primary, from, &line(from, pos2(canvas.left() + 3.0 * BEAT, y), 8), Modifiers::NONE);
    let velocities: Vec<f32> = kick(&h).iter().map(|n| f32::from(n.velocity) / f32::from(u16::MAX)).collect();
    assert!(velocities.iter().all(|v| (v - 0.25).abs() < 0.08), "{velocities:?}");
    assert_eq!(steps(&h), before + 1, "a stroke is one undo step");
    // Pan centers its stems; a stroke at the top pans hard right.
    h.state_mut().view.piano.lane = bodo_ui::Lane::Pan;
    h.run_steps(2);
    let top = lane.top() + 0.5;
    drag(
        &mut h,
        PointerButton::Primary,
        pos2(canvas.left() + 1.0, top),
        &line(pos2(canvas.left() + 1.0, top), pos2(canvas.left() + BEAT + 2.0, top), 4),
        Modifiers::NONE,
    );
    let pans: Vec<f32> = kick(&h).iter().map(|n| n.pan).collect();
    assert!(pans[0] > 0.95 && pans[1] > 0.95 && pans[2].abs() < 1e-6, "{pans:?}");
}

#[test]
fn the_tools_menu_runs_note_tools_on_the_selection() {
    let (mut h, _) = setup("draw");
    add(&mut h, &[(60, 0, 240), (60, 960, 240), (64, 1920, 240)]);
    let menu = element(&h, "Piano roll tools");
    click_at(&mut h, menu.center());
    h.get_by_label("Legato").click();
    h.run_steps(3);
    assert_eq!(shape(&h), [(60, 0, 960), (60, 960, 960), (64, 1920, 240)], "legato with nothing selected acts on every note");
    assert_eq!(h.state().session.undo_labels().last(), Some("Legato"));
}

#[test]
fn the_stamp_lays_down_a_chord_and_the_readout_names_it() {
    let (mut h, canvas) = setup("draw");
    h.state_mut().view.piano.stamp = Some("m7");
    h.run_steps(2);
    let before = steps(&h);
    click_at(&mut h, at(canvas, 1.2, 57));
    let keys: Vec<u8> = kick(&h).iter().map(|n| n.key).collect();
    assert_eq!(keys, [57, 60, 64, 67], "A minor 7 from A3");
    assert_eq!(steps(&h), before + 1);
    h.run_steps(2);
    element(&h, "Chord Am7");
}

#[test]
fn double_clicking_a_ghost_note_edits_its_channel() {
    let (mut h, canvas) = setup("draw");
    h.state_mut().run("note.add_many", json!({ "channel": "Clap", "notes": [{ "key": 62, "start": 960, "length": 480 }] })).unwrap();
    h.run_steps(2);
    common::double_click_at(&mut h, at(canvas, 2.2, 62));
    assert_eq!(h.state().session.editor().channel, Some(channel(&h, "Clap")));
    assert!(kick(&h).is_empty(), "the double-click added nothing");
}

#[test]
fn arrow_keys_transpose_and_tool_keys_reach_the_editor_in_front() {
    let (mut h, canvas) = setup("draw");
    add(&mut h, &[(60, 0, 240)]);
    click_at(&mut h, at(canvas, 1.1, 60));
    h.key_press(Key::ArrowUp);
    h.run_steps(2);
    assert_eq!(kick(&h)[0].key, 61);
    h.key_press_modifiers(Modifiers::SHIFT, Key::ArrowDown);
    h.run_steps(2);
    assert_eq!(kick(&h)[0].key, 49);
    // The piano roll was used last, so B picks its paint tool and leaves the playlist's alone.
    h.key_press(Key::B);
    h.run_steps(2);
    assert_eq!(h.state().view.piano.tool, bodo_ui::Tool::Paint);
    assert_eq!(h.state().view.playlist.tool, bodo_ui::Tool::Draw);
}
