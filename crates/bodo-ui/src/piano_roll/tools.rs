//! The piano roll tools' input (project.md section 10.4).
//!
//! As in the playlist, a press starts a gesture for the current tool, moves update it, and the release finishes it.
//! Every change a gesture makes runs as commands of one gesture, so it is one undo step. Selection, zoom and scrubbing
//! never enter the history.

use bodo_engine::{Call, PlayMode};
use bodo_kit_music::PitchClass;
use bodo_model::{NoteId, Project};
use egui::{Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2};
use serde_json::json;

use super::{Canvas, Tool, Zone, tool_hint};
use crate::theme::{self, Tokens};
use crate::windows::WindowKind;
use crate::{BodoApp, Snap, control};

/// The most notes one frame of painting lays down, so a fast drag can't flood the pattern.
const PAINT_PER_FRAME: usize = 32;

/// A gesture in progress on the grid.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Drag {
    /// Draw's new note, or the stamp's chord, added on release.
    Place { key: u8, start: i64 },
    /// Notes being moved together; the grabbed one started at `start` on `key`, and `grab` is where it was held, in
    /// ticks from its start. `applied` is the move sent so far; `limits` keep every note on the grid.
    Move { notes: Vec<u32>, start: i64, key: u8, grab: f64, applied: (i64, i32), limits: Limits, gesture: u64 },
    /// A note's right edge being dragged.
    Resize { note: NoteId, start: i64, gesture: u64 },
    /// Notes being painted; `next` is the key and tick of the next one.
    Paint { gesture: u64, next: Option<(u8, i64)> },
    /// Notes being deleted along the path.
    Erase { gesture: u64 },
    /// Notes being muted or unmuted along the path, each once.
    Mute { gesture: u64, done: Vec<NoteId> },
    /// A cutting line from `from`.
    Slice { from: Pos2 },
    /// A selection box from `from`.
    Marquee { from: Pos2 },
    /// A zoom box from `from`.
    Zoom { from: Pos2 },
    /// The playback tool scrubbing; `last` is the position sent last.
    Scrub { last: i64 },
}

/// How far notes being moved may go: back to tick 0 and within the 128 keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Limits {
    earliest: i64,
    lowest: u8,
    highest: u8,
}

/// What the pointer did this frame.
#[derive(Clone, Copy, Debug)]
struct Pointer {
    at: Pos2,
    primary_pressed: bool,
    secondary_pressed: bool,
    double_clicked: bool,
    down: bool,
    shift: bool,
    alt: bool,
}

/// Handles the current tool's input on the grid and draws its overlays.
pub(super) fn input(app: &mut BodoApp, ui: &mut Ui, project: &Project, canvas: &Canvas) {
    let response = ui.interact(canvas.rect, ui.id().with("piano-canvas"), Sense::click_and_drag());
    control::record(ui, &response, "Piano roll grid", "canvas", None);
    let tool = app.view.piano.tool;
    if response.hovered() {
        app.view.hint = Some(tool_hint(tool).to_owned());
    }
    let pointer = ui.input(|i| {
        i.pointer.latest_pos().map(|at| Pointer {
            at,
            primary_pressed: i.pointer.primary_pressed(),
            secondary_pressed: i.pointer.secondary_pressed(),
            double_clicked: i.pointer.button_double_clicked(egui::PointerButton::Primary),
            down: i.pointer.any_down(),
            shift: i.modifiers.shift,
            alt: i.modifiers.alt,
        })
    });
    let Some(pointer) = pointer else { return };
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.view.piano.drag = None;
        return;
    }
    let pressed_here = response.is_pointer_button_down_on() || response.hovered();
    // A double-click on a note opens its properties. Its second press may have begun a move, which is dropped.
    if pointer.double_clicked
        && tool == Tool::Draw
        && pressed_here
        && let Some((shown, _)) = canvas.note_at(pointer.at)
    {
        app.view.piano.drag = None;
        app.view.piano.properties = Some(shown.note.id);
        app.view.piano.editing = None;
        return;
    }
    if app.view.piano.drag.is_none() && pressed_here && canvas.rect.contains(pointer.at) {
        if pointer.primary_pressed || pointer.secondary_pressed {
            app.view.editor = WindowKind::PianoRoll;
        }
        if pointer.double_clicked && tool == Tool::Draw && canvas.note_at(pointer.at).is_none() {
            // A double-click on a ghost note edits its channel instead.
            if let Some((channel, _)) = canvas.ghosts.iter().rev().find(|(_, r)| r.contains(pointer.at)) {
                let _ = app.run("edit.select", json!({ "channel": channel }));
                return;
            }
        }
        if pointer.primary_pressed {
            begin(app, project, canvas, tool, pointer);
        } else if pointer.secondary_pressed {
            begin_secondary(app, project, canvas, tool, pointer);
        }
    }
    if app.view.piano.drag.is_some() {
        if pointer.down {
            update(app, project, canvas, pointer);
        } else {
            finish(app, project, canvas, pointer);
        }
    }
    overlay(app, ui, project, canvas, pointer);
    if response.hovered()
        && tool == Tool::Draw
        && let Some((_, Zone::Edge)) = canvas.note_at(pointer.at)
    {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
}

fn snap(app: &BodoApp, pointer: Pointer) -> Snap {
    if pointer.alt { Snap::None } else { app.view.snap }
}

fn run_in(app: &mut BodoApp, gesture: u64, id: &str, params: serde_json::Value) {
    let _ = app.invoke(Call::new(id, params).in_gesture(gesture));
}

fn begin(app: &mut BodoApp, project: &Project, canvas: &Canvas, tool: Tool, pointer: Pointer) {
    let at = canvas.grid.tick(pointer.at.x);
    let hit = canvas.note_at(pointer.at).map(|(s, zone)| (s.note.clone(), zone));
    let drag = match tool {
        Tool::Draw => match hit {
            Some((note, Zone::Edge)) => {
                let _ = app.run("edit.select", json!({ "notes": [note.id.0] }));
                Some(Drag::Resize { note: note.id, start: note.start.0, gesture: app.new_gesture() })
            }
            Some((note, Zone::Body)) => {
                // Clicking a note picks up its length for the next ones, and holding it moves the selection with it.
                app.view.piano.note_length = note.len;
                let selected = &app.session.editor().notes;
                let notes: Vec<u32> = if selected.contains(&note.id) {
                    canvas.shown.iter().filter(|s| selected.contains(&s.note.id)).map(|s| s.note.id.0).collect()
                } else {
                    let _ = app.run("edit.select", json!({ "notes": [note.id.0] }));
                    vec![note.id.0]
                };
                let limits = limits(canvas, &notes);
                Some(Drag::Move {
                    notes,
                    start: note.start.0,
                    key: note.key,
                    grab: at - note.start.0 as f64,
                    applied: (0, 0),
                    limits,
                    gesture: app.new_gesture(),
                })
            }
            None => canvas.grid.key(pointer.at.y).map(|key| Drag::Place { key, start: snap(app, pointer).floor(project, at).max(0) }),
        },
        Tool::Paint => Some(Drag::Paint { gesture: app.new_gesture(), next: None }),
        Tool::Delete => Some(Drag::Erase { gesture: app.new_gesture() }),
        Tool::Mute => Some(Drag::Mute { gesture: app.new_gesture(), done: Vec::new() }),
        Tool::Slice => Some(Drag::Slice { from: pointer.at }),
        Tool::Select => Some(Drag::Marquee { from: pointer.at }),
        Tool::Zoom => Some(Drag::Zoom { from: pointer.at }),
        Tool::Playback => {
            let start = snap(app, pointer).floor(project, at).max(0);
            if app.session.transport().mode != PlayMode::Pattern {
                let _ = app.run("transport.set_mode", json!({ "mode": "pattern" }));
            }
            let _ = app.run("transport.set_position", json!({ "at": start }));
            if !app.session.transport().playing {
                let _ = app.run("transport.play", json!({}));
            }
            Some(Drag::Scrub { last: start })
        }
        Tool::Slip => None,
    };
    if let Some(Drag::Place { key, .. }) = drag {
        app.audio.play_note(canvas.channel, key, 0.8, true);
    }
    app.view.piano.drag = drag;
    // Some tools act on the press itself.
    if matches!(tool, Tool::Paint | Tool::Delete | Tool::Mute) {
        update(app, project, canvas, pointer);
    }
}

/// How far the notes `ids` can move.
fn limits(canvas: &Canvas, ids: &[u32]) -> Limits {
    let moving = canvas.shown.iter().filter(|s| ids.contains(&s.note.id.0)).map(|s| &s.note);
    let mut limits = Limits { earliest: i64::MAX, lowest: 127, highest: 0 };
    for note in moving {
        limits.earliest = limits.earliest.min(note.start.0);
        limits.lowest = limits.lowest.min(note.key);
        limits.highest = limits.highest.max(note.key);
    }
    if limits.earliest == i64::MAX {
        limits.earliest = 0;
    }
    limits
}

fn begin_secondary(app: &mut BodoApp, project: &Project, canvas: &Canvas, tool: Tool, pointer: Pointer) {
    match tool {
        Tool::Draw => {
            if let Some((shown, _)) = canvas.note_at(pointer.at) {
                let _ = app.run("note.remove", json!({ "notes": [shown.note.id.0] }));
            }
        }
        Tool::Paint | Tool::Delete => {
            app.view.piano.drag = Some(Drag::Erase { gesture: app.new_gesture() });
            update(app, project, canvas, pointer);
        }
        Tool::Select => {
            let _ = app.run("edit.deselect", json!({}));
        }
        Tool::Zoom => zoom_around(app, canvas, pointer.at.x, 0.5),
        Tool::Mute | Tool::Slip | Tool::Slice | Tool::Playback => {}
    }
}

fn update(app: &mut BodoApp, project: &Project, canvas: &Canvas, pointer: Pointer) {
    let Some(drag) = app.view.piano.drag.clone() else { return };
    let at = canvas.grid.tick(pointer.at.x);
    let snap = snap(app, pointer);
    let hit = canvas.note_at(pointer.at).map(|(s, _)| s.note.clone());
    let next = match drag {
        Drag::Place { key: before, .. } => {
            let key = canvas.grid.key(pointer.at.y).unwrap_or(before);
            if key != before {
                app.audio.play_note(canvas.channel, before, 0.0, false);
                app.audio.play_note(canvas.channel, key, 0.8, true);
            }
            Drag::Place { key, start: snap.floor(project, at).max(0) }
        }
        Drag::Move { notes, start, key, grab, applied, limits, gesture } => {
            let ticks = (snap.round(project, at - grab).max(0) - start).max(-limits.earliest);
            let keys = canvas.grid.key(pointer.at.y).map_or(applied.1, |k| i32::from(k) - i32::from(key));
            let keys = keys.clamp(-i32::from(limits.lowest), 127 - i32::from(limits.highest));
            if (ticks, keys) != applied {
                run_in(app, gesture, "note.move", json!({ "notes": notes, "ticks": ticks - applied.0, "keys": keys - applied.1 }));
            }
            Drag::Move { notes, start, key, grab, applied: (ticks, keys), limits, gesture }
        }
        Drag::Resize { note, start, gesture } => {
            let end = snap.round(project, at).max(start + snap.ticks(project).max(1));
            app.view.piano.note_length = bodo_kit_core::Tick(end - start);
            run_in(app, gesture, "note.set_props", json!({ "notes": [note.0], "length": end - start }));
            drag
        }
        Drag::Paint { gesture, next } => paint(app, project, canvas, pointer, gesture, next),
        Drag::Erase { gesture } => {
            if let Some(note) = hit {
                run_in(app, gesture, "note.remove", json!({ "notes": [note.id.0] }));
            }
            drag
        }
        Drag::Mute { gesture, mut done } => {
            if let Some(note) = hit
                && !done.contains(&note.id)
            {
                run_in(app, gesture, "note.set_props", json!({ "notes": [note.id.0], "muted": !note.muted }));
                done.push(note.id);
            }
            Drag::Mute { gesture, done }
        }
        Drag::Scrub { last } => {
            let position = snap.floor(project, at).max(0);
            if position != last {
                let _ = app.run("transport.set_position", json!({ "at": position }));
            }
            Drag::Scrub { last: position }
        }
        Drag::Slice { .. } | Drag::Marquee { .. } | Drag::Zoom { .. } => drag,
    };
    app.view.piano.drag = Some(next);
}

/// Lays down notes of the current length along the drag, on the key under the pointer: one where the pointer is when
/// it reaches a key, then one after another as it moves on.
fn paint(app: &mut BodoApp, project: &Project, canvas: &Canvas, pointer: Pointer, gesture: u64, next: Option<(u8, i64)>) -> Drag {
    let Some(key) = canvas.grid.key(pointer.at.y) else { return Drag::Paint { gesture, next } };
    let length = app.view.piano.note_length.0.max(1);
    let at = canvas.grid.tick(pointer.at.x).max(0.0);
    let mut slot = match next {
        Some((k, slot)) if k == key => slot,
        _ => snap(app, pointer).floor(project, at).max(0),
    };
    let mut placed = 0;
    while (slot as f64) <= at && placed < PAINT_PER_FRAME {
        let taken = canvas.shown.iter().any(|s| s.note.key == key && s.note.start.0 == slot);
        if !taken {
            let notes = json!([{ "key": key, "start": slot, "length": length }]);
            run_in(app, gesture, "note.add_many", json!({ "channel": canvas.channel, "notes": notes }));
        }
        slot += length;
        placed += 1;
    }
    Drag::Paint { gesture, next: Some((key, slot)) }
}

fn finish(app: &mut BodoApp, project: &Project, canvas: &Canvas, pointer: Pointer) {
    let Some(drag) = app.view.piano.drag.take() else { return };
    match drag {
        Drag::Place { key, start } => {
            app.audio.play_note(canvas.channel, key, 0.0, false);
            let length = app.view.piano.note_length.0.max(1);
            match app.view.piano.stamp {
                Some(quality) => {
                    let chord = format!("{}{quality}", PitchClass::of(key).name());
                    let octave = i32::from(key / 12) - 1;
                    let params = json!({ "channel": canvas.channel, "chord": chord, "octave": octave, "start": start, "length": length });
                    let _ = app.run("note.stamp", params);
                }
                None => {
                    let notes = json!([{ "key": key, "start": start, "length": length }]);
                    let _ = app.run("note.add_many", json!({ "channel": canvas.channel, "notes": notes }));
                }
            }
        }
        Drag::Slice { from } => slice(app, project, canvas, from, pointer),
        Drag::Marquee { from } => {
            let rect = Rect::from_two_pos(from, pointer.at);
            let notes: Vec<u32> = canvas.shown.iter().filter(|s| s.rect.intersects(rect)).map(|s| s.note.id.0).collect();
            if notes.is_empty() && !pointer.shift {
                let _ = app.run("edit.deselect", json!({}));
            } else if !notes.is_empty() {
                let _ = app.run("edit.select", json!({ "notes": notes, "toggle": pointer.shift }));
            }
        }
        Drag::Zoom { from } => {
            let rect = Rect::from_two_pos(from, pointer.at);
            if rect.width() < 4.0 {
                zoom_around(app, canvas, pointer.at.x, 2.0);
            } else {
                let grid = canvas.grid;
                let (start, end) = (grid.tick(rect.left()).max(0.0), grid.tick(rect.right()));
                let keys = (rect.height() / grid.key_height).max(1.0);
                let view = &mut app.view.piano;
                view.zoom = (canvas.rect.width() / (end - start).max(1.0) as f32).clamp(8.0 / 3840.0, 4.0);
                view.scroll = start;
                view.key_height = (canvas.rect.height() / keys).clamp(6.0, 40.0);
                view.top_key = (grid.top_key - (rect.top() - canvas.rect.top()) / grid.key_height).clamp(12.0, 127.0);
            }
        }
        Drag::Scrub { .. } => {
            let _ = app.run("transport.stop", json!({}));
        }
        Drag::Move { .. } | Drag::Resize { .. } | Drag::Paint { .. } | Drag::Erase { .. } | Drag::Mute { .. } => {}
    }
}

/// Cuts every note the line from `from` to the pointer crosses: straight down from `from`, or along the slanted line
/// with Shift. One gesture, so one undo step.
fn slice(app: &mut BodoApp, project: &Project, canvas: &Canvas, from: Pos2, pointer: Pointer) {
    let gesture = app.new_gesture();
    let (top, bottom) = (from.y.min(pointer.at.y), from.y.max(pointer.at.y));
    let mut cuts: Vec<(i64, Vec<u32>)> = Vec::new();
    for shown in canvas.shown.iter().filter(|s| s.rect.bottom() >= top && s.rect.top() <= bottom) {
        let middle = shown.rect.center().y.clamp(top, bottom);
        let x = if pointer.shift && (pointer.at.y - from.y).abs() > 1.0 {
            from.x + (pointer.at.x - from.x) * (middle - from.y) / (pointer.at.y - from.y)
        } else {
            from.x
        };
        let at = snap(app, pointer).round(project, canvas.grid.tick(x)).max(0);
        if shown.note.start.0 < at && at < shown.note.end().0 {
            match cuts.iter_mut().find(|(t, _)| *t == at) {
                Some((_, ids)) => ids.push(shown.note.id.0),
                None => cuts.push((at, vec![shown.note.id.0])),
            }
        }
    }
    for (at, notes) in cuts {
        run_in(app, gesture, "note.slice", json!({ "notes": notes, "at": at }));
    }
}

/// Zooms by `factor` keeping the tick under `x` in place.
fn zoom_around(app: &mut BodoApp, canvas: &Canvas, x: f32, factor: f32) {
    let anchor = canvas.grid.tick(x);
    let view = &mut app.view.piano;
    view.zoom = (view.zoom * factor).clamp(8.0 / 3840.0, 4.0);
    view.scroll = (anchor - f64::from((x - canvas.rect.left()) / view.zoom)).max(0.0);
}

/// Draws the gesture in progress: the new note or chord, the cutting line, the selection or zoom box.
fn overlay(app: &BodoApp, ui: &Ui, project: &Project, canvas: &Canvas, pointer: Pointer) {
    let tk = Tokens::get();
    let painter = ui.painter().with_clip_rect(canvas.rect);
    match &app.view.piano.drag {
        Some(Drag::Place { key, start }) => {
            let length = app.view.piano.note_length.0.max(1);
            let color = project.channels.get(&canvas.channel).map_or(tk.accent, |c| theme::color(c.color));
            let keys = match app.view.piano.stamp {
                Some(quality) => format!("{}{quality}", PitchClass::of(*key).name())
                    .parse::<bodo_kit_music::Chord>()
                    .map(|c| c.keys(i32::from(key / 12) - 1))
                    .unwrap_or_else(|_| vec![*key]),
                None => vec![*key],
            };
            for k in keys {
                let y = canvas.grid.y(k);
                let rect = Rect::from_min_max(
                    pos2(canvas.grid.x(*start as f64), y + 1.0),
                    pos2(canvas.grid.x((start + length) as f64), y + canvas.grid.key_height - 1.0),
                );
                painter.rect(rect, 2.0, theme::faded(color, 0.45), Stroke::new(1.0, color), StrokeKind::Inside);
            }
        }
        Some(Drag::Slice { from }) => {
            let to = if pointer.shift { pointer.at } else { pos2(from.x, pointer.at.y) };
            painter.line_segment([*from, to], Stroke::new(1.5, tk.error));
        }
        Some(Drag::Marquee { from }) | Some(Drag::Zoom { from }) => {
            let rect = Rect::from_two_pos(*from, pointer.at);
            painter.rect(rect, 0.0, theme::faded(tk.marquee, 0.15), Stroke::new(1.0, tk.marquee), StrokeKind::Inside);
        }
        _ => {}
    }
}
