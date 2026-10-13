//! The playlist tools' input (project.md section 10.3).
//!
//! A press on the canvas starts a gesture for the current tool, moves update it, and the release finishes it. Every
//! change a gesture makes runs as commands of one gesture, so it is one undo step. Selection, zoom and scrubbing
//! never enter the history.

use bodo_engine::{Call, PlayMode};
use bodo_model::playlist::ClipSource;
use bodo_model::{ClipId, Project, TrackId};
use egui::{Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2};
use serde_json::json;

use super::{Canvas, Shown, Tool, Zone};
use crate::theme::{self, Tokens};
use crate::{BodoApp, Snap, control};

/// The most clips one frame of painting lays down, so a fast drag can't flood the playlist.
const PAINT_PER_FRAME: usize = 32;

/// A gesture in progress on the canvas.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Drag {
    /// The draw tool's new clip, placed on release.
    Place { track: TrackId, start: i64 },
    /// A clip being moved; `grab` is where it was held, in ticks from its start.
    Move { clip: ClipId, grab: f64, gesture: u64 },
    /// A clip's right edge being dragged.
    Resize { clip: ClipId, start: i64, gesture: u64 },
    /// Copies being painted; `next` is where the next one goes on its track.
    Paint { gesture: u64, next: Option<(TrackId, i64)> },
    /// Clips being deleted along the path.
    Erase { gesture: u64 },
    /// Clips being muted or unmuted along the path, each once.
    Mute { gesture: u64, done: Vec<ClipId> },
    /// A clip's content being slipped; `last` is the tick it was slipped to so far.
    Slip { clip: ClipId, gesture: u64, last: f64 },
    /// A cutting line from `from`.
    Slice { from: Pos2 },
    /// A selection box from `from`.
    Marquee { from: Pos2 },
    /// A zoom box from `from`.
    Zoom { from: Pos2 },
    /// The playback tool scrubbing; `last` is the position sent last.
    Scrub { last: i64 },
}

/// What the pointer did this frame.
#[derive(Clone, Copy, Debug)]
struct Pointer {
    at: Pos2,
    primary_pressed: bool,
    secondary_pressed: bool,
    down: bool,
    shift: bool,
    alt: bool,
}

/// Handles the current tool's input on the canvas and draws its overlays.
pub(super) fn input(app: &mut BodoApp, ui: &mut Ui, project: &Project, canvas: &Canvas) {
    let response = ui.interact(canvas.rect, ui.id().with("playlist-canvas"), Sense::click_and_drag());
    control::record(ui, &response, "Playlist canvas", "canvas", None);
    let tool = app.view.playlist.tool;
    if response.hovered() {
        app.view.hint = Some(tool.hint().to_owned());
    }
    let pointer = ui.input(|i| {
        i.pointer.latest_pos().map(|at| Pointer {
            at,
            primary_pressed: i.pointer.primary_pressed(),
            secondary_pressed: i.pointer.secondary_pressed(),
            down: i.pointer.any_down(),
            shift: i.modifiers.shift,
            alt: i.modifiers.alt,
        })
    });
    let Some(pointer) = pointer else { return };
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.view.playlist.drag = None;
        return;
    }
    let pressed_here = response.is_pointer_button_down_on() || response.hovered();
    if app.view.playlist.drag.is_none() && pressed_here && canvas.rect.contains(pointer.at) {
        if pointer.primary_pressed {
            begin(app, project, canvas, tool, pointer);
        } else if pointer.secondary_pressed {
            begin_secondary(app, project, canvas, tool, pointer);
        }
    }
    if app.view.playlist.drag.is_some() {
        if pointer.down {
            update(app, project, canvas, pointer);
        } else {
            finish(app, project, canvas, pointer);
        }
    }
    overlay(app, ui, project, canvas, pointer);
    // Cursors that say what a press would do.
    if response.hovered()
        && tool == Tool::Draw
        && let Some((_, Zone::Edge)) = canvas.clip_at(pointer.at)
    {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
}

fn snap(app: &BodoApp, pointer: Pointer) -> Snap {
    if pointer.alt { Snap::None } else { app.view.snap }
}

/// The pattern clips of `track` hold `pattern` at `start` already.
fn occupied(canvas: &Canvas, track: TrackId, start: i64) -> bool {
    canvas.shown.iter().any(|c| c.track == track && c.start == start)
}

fn begin(app: &mut BodoApp, project: &Project, canvas: &Canvas, tool: Tool, pointer: Pointer) {
    let at = canvas.scale.tick(pointer.at.x);
    let hit = canvas.clip_at(pointer.at);
    let drag = match tool {
        Tool::Draw => match hit {
            Some((clip, Zone::Edge)) => Some(Drag::Resize { clip: clip.id, start: clip.start, gesture: app.new_gesture() }),
            Some((clip, _)) => {
                select_clip(app, &clip);
                Some(Drag::Move { clip: clip.id, grab: at - clip.start as f64, gesture: app.new_gesture() })
            }
            None => match (canvas.row_at(pointer.at.y), app.session.editor().pattern) {
                (Some(row), Some(_)) => Some(Drag::Place { track: row.id, start: snap(app, pointer).floor(project, at).max(0) }),
                _ => None,
            },
        },
        Tool::Paint => Some(Drag::Paint { gesture: app.new_gesture(), next: None }),
        Tool::Delete => Some(Drag::Erase { gesture: app.new_gesture() }),
        Tool::Mute => Some(Drag::Mute { gesture: app.new_gesture(), done: Vec::new() }),
        Tool::Slip => hit.map(|(clip, _)| Drag::Slip { clip: clip.id, gesture: app.new_gesture(), last: at }),
        Tool::Slice => Some(Drag::Slice { from: pointer.at }),
        Tool::Select => Some(Drag::Marquee { from: pointer.at }),
        Tool::Zoom => Some(Drag::Zoom { from: pointer.at }),
        Tool::Playback => {
            let start = snap(app, pointer).floor(project, at).max(0);
            if app.session.transport().mode != PlayMode::Song {
                let _ = app.run("transport.set_mode", json!({ "mode": "song" }));
            }
            let _ = app.run("transport.set_position", json!({ "at": start }));
            if !app.session.transport().playing {
                let _ = app.run("transport.play", json!({}));
            }
            Some(Drag::Scrub { last: start })
        }
    };
    app.view.playlist.drag = drag;
    // Some tools act on the press itself.
    if matches!(tool, Tool::Paint | Tool::Delete | Tool::Mute) {
        update(app, project, canvas, pointer);
    }
}

fn begin_secondary(app: &mut BodoApp, project: &Project, canvas: &Canvas, tool: Tool, pointer: Pointer) {
    let hit = canvas.clip_at(pointer.at);
    if let Some((clip, Zone::Header)) = hit
        && !matches!(tool, Tool::Paint | Tool::Delete)
    {
        app.view.playlist.menu = Some((clip.id, pointer.at));
        return;
    }
    match tool {
        Tool::Draw => {
            if let Some((clip, _)) = hit {
                let _ = app.run("clip.remove", json!({ "clips": [clip.id] }));
            }
        }
        Tool::Paint | Tool::Delete => {
            app.view.playlist.drag = Some(Drag::Erase { gesture: app.new_gesture() });
            update(app, project, canvas, pointer);
        }
        Tool::Select => {
            let _ = app.run("edit.deselect", json!({}));
        }
        Tool::Zoom => zoom_around(app, canvas, pointer.at.x, 0.5),
        Tool::Mute | Tool::Slip | Tool::Slice | Tool::Playback => {}
    }
}

/// Selects `clip` and makes its pattern current, as clicking a clip does in FL.
fn select_clip(app: &mut BodoApp, clip: &Shown) {
    let _ = app.run("edit.select", json!({ "clips": [clip.id], "track": clip.track }));
    if let ClipSource::Pattern(pattern) = clip.source
        && app.session.editor().pattern != Some(pattern)
    {
        let _ = app.run("ui.select_pattern", json!({ "pattern": pattern }));
    }
}

fn run_in(app: &mut BodoApp, gesture: u64, id: &str, params: serde_json::Value) {
    let _ = app.invoke(Call::new(id, params).in_gesture(gesture));
}

fn update(app: &mut BodoApp, project: &Project, canvas: &Canvas, pointer: Pointer) {
    let Some(drag) = app.view.playlist.drag.clone() else { return };
    let at = canvas.scale.tick(pointer.at.x);
    let snap = snap(app, pointer);
    let hit = canvas.clip_at(pointer.at);
    let next = match drag {
        Drag::Place { .. } => match canvas.row_at(pointer.at.y) {
            Some(row) => Drag::Place { track: row.id, start: snap.floor(project, at).max(0) },
            None => drag,
        },
        Drag::Move { clip, grab, gesture } => {
            let start = snap.round(project, at - grab).max(0);
            let track = canvas.row_at(pointer.at.y).map(|r| r.id);
            let current = canvas.shown.iter().find(|c| c.id == clip);
            let moved = current.is_none_or(|c| c.start != start || track.is_some_and(|t| t != c.track));
            if moved {
                run_in(app, gesture, "clip.move", json!({ "clips": [clip], "start": start, "track": track }));
            }
            drag
        }
        Drag::Resize { clip, start, gesture } => {
            let end = snap.round(project, at).max(start + snap.ticks(project).max(1));
            run_in(app, gesture, "clip.resize", json!({ "clips": [clip], "length": end - start }));
            drag
        }
        Drag::Paint { gesture, next } => paint(app, project, canvas, pointer, gesture, next),
        Drag::Erase { gesture } => {
            if let Some((clip, _)) = hit {
                run_in(app, gesture, "clip.remove", json!({ "clips": [clip.id] }));
            }
            drag
        }
        Drag::Mute { gesture, mut done } => {
            if let Some((clip, _)) = hit
                && !done.contains(&clip.id)
            {
                run_in(app, gesture, "clip.mute", json!({ "clips": [clip.id] }));
                done.push(clip.id);
            }
            Drag::Mute { gesture, done }
        }
        Drag::Slip { clip, gesture, last } => {
            let grid = snap.ticks(project).max(1) as f64;
            let by = ((at - last) / grid).trunc() * grid;
            if by != 0.0 {
                run_in(app, gesture, "clip.slip", json!({ "clips": [clip], "by": by as i64 }));
                Drag::Slip { clip, gesture, last: last + by }
            } else {
                drag
            }
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
    app.view.playlist.drag = Some(next);
}

/// Lays down copies of the current pattern along the drag: a copy where the pointer is when it enters a track, then
/// one after another, each at the pattern's length, as the pointer moves on.
fn paint(app: &mut BodoApp, project: &Project, canvas: &Canvas, pointer: Pointer, gesture: u64, next: Option<(TrackId, i64)>) -> Drag {
    let (Some(row), Some(pattern)) = (canvas.row_at(pointer.at.y), app.session.editor().pattern) else { return Drag::Paint { gesture, next } };
    let length = bodo_engine::pattern_length(project, pattern).0.max(1);
    let at = canvas.scale.tick(pointer.at.x).max(0.0);
    let mut slot = match next {
        Some((track, slot)) if track == row.id => slot,
        _ => snap(app, pointer).floor(project, at).max(0),
    };
    let mut placed = 0;
    while (slot as f64) <= at && placed < PAINT_PER_FRAME {
        if !occupied(canvas, row.id, slot) {
            run_in(app, gesture, "clip.place", json!({ "track": row.id, "start": slot }));
        }
        slot += length;
        placed += 1;
    }
    Drag::Paint { gesture, next: Some((row.id, slot)) }
}

fn finish(app: &mut BodoApp, project: &Project, canvas: &Canvas, pointer: Pointer) {
    let Some(drag) = app.view.playlist.drag.take() else { return };
    match drag {
        Drag::Place { track, start } => {
            let _ = app.run("clip.place", json!({ "track": track, "start": start }));
        }
        Drag::Slice { from } => {
            let gesture = app.new_gesture();
            let (top, bottom) = (from.y.min(pointer.at.y), from.y.max(pointer.at.y));
            for row in canvas.rows.iter().filter(|r| r.top + r.height >= top && r.top <= bottom) {
                let middle = (row.top + row.height / 2.0).clamp(top, bottom);
                // Straight down by default; Shift follows the slanted line.
                let x = if pointer.shift && (pointer.at.y - from.y).abs() > 1.0 {
                    from.x + (pointer.at.x - from.x) * (middle - from.y) / (pointer.at.y - from.y)
                } else {
                    from.x
                };
                let at = snap(app, pointer).round(project, canvas.scale.tick(x)).max(0);
                let crosses = canvas
                    .shown
                    .iter()
                    .any(|c| c.track == row.id && c.rect.left() < canvas.scale.x(at as f64) && c.rect.right() > canvas.scale.x(at as f64));
                if crosses {
                    run_in(app, gesture, "clip.slice", json!({ "at": at, "tracks": [row.id] }));
                }
            }
        }
        Drag::Marquee { from } => {
            let rect = Rect::from_two_pos(from, pointer.at);
            let clips: Vec<ClipId> = canvas.shown.iter().filter(|c| c.rect.intersects(rect)).map(|c| c.id).collect();
            if clips.is_empty() && !pointer.shift {
                let _ = app.run("edit.deselect", json!({}));
            } else if !clips.is_empty() {
                let _ = app.run("edit.select", json!({ "clips": clips, "toggle": pointer.shift }));
            }
        }
        Drag::Zoom { from } => {
            let rect = Rect::from_two_pos(from, pointer.at);
            if rect.width() < 4.0 {
                zoom_around(app, canvas, pointer.at.x, 2.0);
            } else {
                let (start, end) = (canvas.scale.tick(rect.left()).max(0.0), canvas.scale.tick(rect.right()));
                let view = &mut app.view.playlist;
                view.zoom = (canvas.rect.width() / (end - start).max(1.0) as f32).clamp(4.0 / 3840.0, 2.0);
                view.scroll = start;
            }
        }
        Drag::Scrub { .. } => {
            let _ = app.run("transport.stop", json!({}));
        }
        Drag::Move { .. } | Drag::Resize { .. } | Drag::Paint { .. } | Drag::Erase { .. } | Drag::Mute { .. } | Drag::Slip { .. } => {}
    }
}

/// Zooms by `factor` keeping the tick under `x` in place.
fn zoom_around(app: &mut BodoApp, canvas: &Canvas, x: f32, factor: f32) {
    let anchor = canvas.scale.tick(x);
    let view = &mut app.view.playlist;
    view.zoom = (view.zoom * factor).clamp(4.0 / 3840.0, 2.0);
    view.scroll = (anchor - f64::from((x - canvas.rect.left()) / view.zoom)).max(0.0);
}

/// Draws the gesture in progress: the new clip, the cutting line, the selection or zoom box.
fn overlay(app: &BodoApp, ui: &Ui, project: &Project, canvas: &Canvas, pointer: Pointer) {
    let tk = Tokens::get();
    let painter = ui.painter().with_clip_rect(canvas.rect);
    match &app.view.playlist.drag {
        Some(Drag::Place { track, start }) => {
            let (Some(row), Some(pattern)) = (canvas.rows.iter().find(|r| r.id == *track), app.session.editor().pattern) else { return };
            let length = bodo_engine::pattern_length(project, pattern).0;
            let color = project.patterns.get(&pattern).map_or(tk.accent, |p| theme::color(p.color));
            let rect = Rect::from_min_max(
                pos2(canvas.scale.x(*start as f64), row.top + 1.0),
                pos2(canvas.scale.x((start + length) as f64), row.top + row.height - 1.0),
            );
            painter.rect(rect, 3.0, theme::faded(color, 0.35), Stroke::new(1.0, color), StrokeKind::Inside);
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
