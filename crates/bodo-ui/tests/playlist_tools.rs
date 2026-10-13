//! Every row of the Playlist tool table (project.md section 10.3), driven with real pointer events: Draw, Paint,
//! Delete, Mute, Slip, Slice, Select, Zoom and Playback. Each test checks what the tool does to the project, that a
//! gesture is one undo step, and that view-only tools stay out of the history.

#![allow(clippy::unwrap_used, clippy::panic, reason = "tests")]

mod common;

use bodo_engine::{PlayMode, inspect};
use bodo_kit_core::Tick;
use bodo_model::Clip;
use bodo_model::playlist::{ClipOffset, ClipSource};
use bodo_ui::BodoApp;
use common::{click_at, drag, element, line};
use egui::{Modifiers, PointerButton, Pos2, Rect, Theme, pos2};
use egui_kittest::Harness;
use serde_json::json;

/// Points per bar in these tests: the whole 8-bar beat fits the canvas.
const BAR: f32 = 48.0;
/// The height of a track.
const TRACK: f32 = 48.0;

/// The beat in a harness, zoomed out, with `tool` chosen.
fn setup(tool: &str) -> (Harness<'static, BodoApp>, Rect) {
    let mut app = common::beat_app();
    app.view.playlist.zoom = BAR / 3840.0;
    app.run("ui.set_tool", json!({ "tool": tool })).unwrap();
    let mut harness = common::harness(app, Theme::Dark, false);
    harness.run_steps(4);
    let canvas = element(&harness, "Playlist canvas");
    (harness, canvas)
}

/// The point at `bar` (counted from 1, fractions allowed) on track `track` (from 1), in the middle of the track.
fn at(canvas: Rect, bar: f32, track: usize) -> Pos2 {
    pos2(canvas.left() + (bar - 1.0) * BAR, canvas.top() + (track as f32 - 0.5) * TRACK)
}

fn clips(harness: &Harness<'_, BodoApp>, track: usize) -> Vec<Clip> {
    let project = harness.state().session.project().clone();
    let id = project.track_order[track - 1];
    project.tracks[&id].clips.iter().cloned().collect()
}

fn steps(harness: &Harness<'_, BodoApp>) -> usize {
    harness.state().session.undo_labels().count()
}

#[test]
fn draw_places_moves_deletes_and_alt_turns_snap_off() {
    let (mut h, canvas) = setup("draw");
    let before = steps(&h);
    // Click places the current pattern (Outro, one bar) at the snapped position.
    click_at(&mut h, at(canvas, 2.3, 3));
    assert_eq!(clips(&h, 3).iter().map(|c| c.start.0).collect::<Vec<_>>(), [3840 + 960]);
    // Dragging before release moves the new clip: one undo step for the whole gesture.
    drag(&mut h, PointerButton::Primary, at(canvas, 4.1, 4), &line(at(canvas, 4.1, 4), at(canvas, 6.1, 5), 6), Modifiers::NONE);
    assert!(clips(&h, 4).is_empty());
    assert_eq!(clips(&h, 5).iter().map(|c| c.start.0).collect::<Vec<_>>(), [5 * 3840]);
    assert_eq!(steps(&h), before + 2);
    // Dragging a clip moves it, to another track too.
    drag(&mut h, PointerButton::Primary, at(canvas, 6.2, 5), &line(at(canvas, 6.2, 5), at(canvas, 3.2, 6), 6), Modifiers::NONE);
    assert_eq!(clips(&h, 6).iter().map(|c| c.start.0).collect::<Vec<_>>(), [2 * 3840]);
    // Right-click deletes.
    h.event(egui::Event::PointerMoved(at(canvas, 3.5, 6)));
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at(canvas, 3.5, 6), button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        h.step();
    }
    assert!(clips(&h, 6).is_empty());
    // Alt turns snap off: the clip lands where the pointer is, not on the beat.
    drag(&mut h, PointerButton::Primary, at(canvas, 2.3, 7), &[], Modifiers::ALT);
    let start = clips(&h, 7)[0].start.0;
    assert!(start % 960 != 0, "unsnapped start {start}");
}

#[test]
fn paint_lays_copies_along_the_drag_and_right_drag_deletes_them() {
    let (mut h, canvas) = setup("paint");
    let before = steps(&h);
    drag(&mut h, PointerButton::Primary, at(canvas, 1.2, 4), &line(at(canvas, 1.2, 4), at(canvas, 4.5, 4), 12), Modifiers::NONE);
    assert_eq!(clips(&h, 4).iter().map(|c| c.start.0).collect::<Vec<_>>(), [0, 3840, 7680, 11_520]);
    assert_eq!(steps(&h), before + 1, "a stroke is one undo step");
    drag(&mut h, PointerButton::Secondary, at(canvas, 1.5, 4), &line(at(canvas, 1.5, 4), at(canvas, 4.5, 4), 12), Modifiers::NONE);
    assert!(clips(&h, 4).is_empty());
    assert_eq!(steps(&h), before + 2);
}

#[test]
fn delete_removes_the_clips_under_the_drag() {
    let (mut h, canvas) = setup("delete");
    let before = steps(&h);
    drag(&mut h, PointerButton::Primary, at(canvas, 1.5, 1), &line(at(canvas, 1.5, 1), at(canvas, 7.5, 1), 24), Modifiers::NONE);
    let left: Vec<i64> = clips(&h, 1).iter().map(|c| c.start.0).collect();
    assert_eq!(left, [7 * 3840], "Intro, Verse and Fill are gone; the Outro in bar 8 stays");
    assert_eq!(steps(&h), before + 1);
}

#[test]
fn mute_toggles_clips_without_touching_the_track() {
    let (mut h, canvas) = setup("mute");
    click_at(&mut h, at(canvas, 1.5, 1));
    assert!(clips(&h, 1)[0].muted);
    assert!(!h.state().session.project().tracks.values().any(|t| t.muted), "track mute is separate");
    click_at(&mut h, at(canvas, 1.5, 1));
    assert!(!clips(&h, 1)[0].muted);
}

#[test]
fn slip_slides_content_but_not_the_clip() {
    let (mut h, canvas) = setup("slip");
    let original = clips(&h, 1)[0].clone();
    drag(&mut h, PointerButton::Primary, at(canvas, 1.2, 1), &line(at(canvas, 1.2, 1), at(canvas, 1.2 + 0.25, 1), 4), Modifiers::NONE);
    let slipped = clips(&h, 1)[0].clone();
    assert_eq!((slipped.start, slipped.len), (original.start, original.len));
    assert_eq!(slipped.offset, ClipOffset::Ticks(Tick(960)), "a quarter bar, one beat");
}

#[test]
fn slice_cuts_every_clip_the_line_crosses_and_shift_slants_it() {
    let (mut h, canvas) = setup("slice");
    h.state_mut().run("clip.place", json!({ "track": 3, "pattern": "Verse", "start": 0, "length": "4 bars" })).unwrap();
    h.state_mut().run("clip.place", json!({ "track": 4, "pattern": "Verse", "start": 0, "length": "4 bars" })).unwrap();
    let before = steps(&h);
    // A straight cut down at bar 5 crosses the Verse on track 1 only.
    drag(&mut h, PointerButton::Primary, at(canvas, 5.0, 1), &line(at(canvas, 5.0, 1), at(canvas, 5.0, 2), 4), Modifiers::NONE);
    let track1 = clips(&h, 1);
    let verse: Vec<&Clip> = track1.iter().filter(|c| c.start.0 >= 7680 && c.start.0 < 23_040).collect();
    assert_eq!(verse.iter().map(|c| (c.start.0, c.len.0)).collect::<Vec<_>>(), [(7680, 7680), (15_360, 7680)]);
    assert_eq!(verse[0].source, verse[1].source, "both halves play the same pattern");
    assert_eq!(steps(&h), before + 1);
    // A slanted cut with Shift, from bar 2 on track 3 to bar 4 on track 4.
    drag(&mut h, PointerButton::Primary, at(canvas, 2.0, 3), &line(at(canvas, 2.0, 3), at(canvas, 4.0, 4), 6), Modifiers::SHIFT);
    assert_eq!(clips(&h, 3).iter().map(|c| c.start.0).collect::<Vec<_>>(), [0, 3840]);
    assert_eq!(clips(&h, 4).iter().map(|c| c.start.0).collect::<Vec<_>>(), [0, 3 * 3840]);
    assert!(matches!(clips(&h, 4)[1].source, ClipSource::Pattern(_)));
}

#[test]
fn select_boxes_clips_shift_toggles_and_the_history_stays_put() {
    let (mut h, canvas) = setup("select");
    let before = steps(&h);
    let top = at(canvas, 1.05, 1) - egui::vec2(0.0, 20.0);
    drag(&mut h, PointerButton::Primary, top, &line(top, at(canvas, 6.5, 1), 6), Modifiers::NONE);
    assert_eq!(h.state().session.editor().clips.len(), 2, "the Intro and the Verse");
    drag(&mut h, PointerButton::Primary, at(canvas, 7.5, 1), &line(at(canvas, 7.5, 1), at(canvas, 8.5, 2), 4), Modifiers::SHIFT);
    assert_eq!(h.state().session.editor().clips.len(), 4, "Shift adds the Fill and the Outro");
    drag(&mut h, PointerButton::Primary, at(canvas, 1.5, 1), &line(at(canvas, 1.5, 1), at(canvas, 1.6, 1), 2), Modifiers::SHIFT);
    assert_eq!(h.state().session.editor().clips.len(), 3, "Shift takes the Intro out again");
    assert_eq!(steps(&h), before, "selection never enters the undo history");
}

#[test]
fn zoom_boxes_into_the_view_and_changes_nothing_else() {
    let (mut h, canvas) = setup("zoom");
    let project = h.state().session.project().clone();
    drag(&mut h, PointerButton::Primary, at(canvas, 3.0, 1), &line(at(canvas, 3.0, 1), at(canvas, 5.0, 2), 4), Modifiers::NONE);
    let view = &h.state().view.playlist;
    let bars_shown = canvas.width() / view.zoom / 3840.0;
    assert!((bars_shown - 2.0).abs() < 0.05, "two bars fill the canvas: {bars_shown}");
    assert!((view.scroll - 7680.0).abs() < 1.0, "the view starts at bar 3: {}", view.scroll);
    assert!(std::sync::Arc::ptr_eq(h.state().session.project(), &project), "zooming is not an edit");
}

#[test]
fn playback_plays_from_the_pointer_while_held() {
    let (mut h, canvas) = setup("playback");
    let from = at(canvas, 3.1, 1);
    h.event(egui::Event::PointerMoved(from));
    h.step();
    h.event(egui::Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    let transport = *h.state().session.transport();
    assert!(transport.playing && transport.mode == PlayMode::Song);
    assert_eq!(transport.position, Tick(7680), "from bar 3");
    // Dragging scrubs.
    h.event(egui::Event::PointerMoved(at(canvas, 5.1, 1)));
    h.step();
    assert_eq!(h.state().session.transport().position, Tick(15_360));
    h.event(egui::Event::PointerButton { pos: at(canvas, 5.1, 1), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.step();
    h.step();
    assert!(!h.state().session.transport().playing, "releasing stops");
    let text = inspect::project(&h.state().session).text;
    assert!(text.contains("Drums: Intro at 1:1:0"), "nothing was edited: {text}");
}

#[test]
fn shift_dragging_the_ruler_sets_the_song_loop() {
    let (mut h, canvas) = setup("draw");
    let ruler = element(&h, "Playlist ruler");
    let y = ruler.center().y;
    let (from, to) = (pos2(canvas.left() + 2.0 * BAR + 1.0, y), pos2(canvas.left() + 4.0 * BAR + 1.0, y));
    let before = steps(&h);
    drag(&mut h, PointerButton::Primary, from, &line(from, to, 6), Modifiers::SHIFT);
    assert_eq!(h.state().session.transport().loop_region, Some((Tick(7680), Tick(15_360))), "bars 3 to 5");
    assert_eq!(steps(&h), before, "the loop is transport state, not an edit");
}

#[test]
fn a_clips_menu_opens_from_its_name_strip_and_makes_it_unique() {
    use egui_kittest::kittest::Queryable;
    let (mut h, canvas) = setup("draw");
    let verse = clips(&h, 1)[1].clone();
    let strip = pos2(at(canvas, 4.0, 1).x, canvas.top() + 4.0);
    h.event(egui::Event::PointerMoved(strip));
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: strip, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        h.step();
    }
    h.step();
    h.get_by_label("Make unique").click();
    h.run_steps(3);
    let after = clips(&h, 1)[1].clone();
    assert_eq!((after.id, after.start), (verse.id, verse.start), "the right-click didn't delete the clip");
    assert_ne!(after.source, verse.source, "the clip plays a copy of its pattern now");
}

#[test]
fn cmd_x_cuts_the_time_selection_on_every_track() {
    let (mut h, _) = setup("draw");
    h.state_mut().view.playlist.time_selection = Some((7680, 15_360));
    h.state_mut().view.editor = bodo_ui::windows::WindowKind::Playlist;
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::X);
    h.run_steps(2);
    let starts: Vec<i64> = clips(&h, 1).iter().map(|c| c.start.0).collect();
    assert!(starts.iter().all(|s| !(7680..15_360).contains(s)), "nothing starts inside the cut: {starts:?}");
    assert!(starts.contains(&15_360), "the Verse goes on after the cut: {starts:?}");
    assert!(h.state().session.editor().clipboard.is_some(), "the cut clips are on the clipboard");
}
