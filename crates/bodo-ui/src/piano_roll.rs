//! The Piano Roll (project.md section 10.4): the notes of one channel in the current pattern, with the other channels'
//! notes shown faintly behind them.
//!
//! Click empty space to add a note, drag a note to move it, drag its right edge to change its length, right-click to
//! delete it. New notes take the length of the last note drawn or resized. Ctrl/Cmd+wheel zooms, Shift+wheel scrolls
//! sideways, the wheel scrolls keys.
//!
//! Drawing is culled: only visible keys are drawn, and notes come from the sequence's overlap query, O(log n + k).

use std::sync::Arc;

use bodo_engine::PlayMode;
use bodo_engine::params::{STEP, key_name};
use bodo_kit_core::Tick;
use bodo_model::{ChannelId, NoteId, Pattern, Project};
use egui::{Align2, FontId, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::json;

use crate::theme::{self, Tokens, size};
use crate::windows::{self, WindowKind};
use crate::{BodoApp, control};

/// The keyboard's width.
const KEYS: f32 = 46.0;
/// The ruler's height.
const RULER: f32 = 20.0;
/// How close to a note's right edge a drag resizes it.
const EDGE: f32 = 5.0;

#[derive(Clone, Copy, Debug, PartialEq)]
struct NoteDrag {
    note: NoteId,
    /// Where the note started when the drag began.
    start: i64,
    /// Ticks between the note's start and where it was grabbed.
    grab: f64,
    resize: bool,
    gesture: u64,
}

/// The piano roll's zoom, scroll and the length of new notes.
#[derive(Debug)]
pub struct PianoView {
    /// Points per tick.
    pub zoom: f32,
    /// The tick at the grid's left edge.
    pub scroll: f64,
    /// The key at the top edge, fractional while scrolling.
    pub top_key: f32,
    /// Points per key.
    pub key_height: f32,
    /// The length of new notes.
    pub note_length: Tick,
    drag: Option<NoteDrag>,
}

impl Default for PianoView {
    fn default() -> Self {
        Self { zoom: 160.0 / 3840.0 * 4.0, scroll: 0.0, top_key: 76.0, key_height: 14.0, note_length: STEP, drag: None }
    }
}

fn is_black(key: u8) -> bool {
    matches!(key % 12, 1 | 3 | 6 | 8 | 10)
}

/// Draws the Piano Roll window.
pub(crate) fn window(app: &mut BodoApp, ctx: &egui::Context, workspace: Rect, project: &Arc<Project>) {
    let mut layout = std::mem::take(&mut app.prefs.layout);
    let channel = app.session.editor().channel.filter(|c| project.channels.contains_key(c)).or_else(|| project.rack_order.front().copied());
    let pattern = app.session.editor().pattern.and_then(|p| project.patterns.get(&p)).cloned();
    let channel_name = channel.and_then(|c| project.channels.get(&c)).map(|c| c.name.clone()).unwrap_or_default();
    let title = match &pattern {
        Some(p) => format!("Piano roll - {} > {channel_name}", p.name),
        None => String::from("Piano roll"),
    };
    windows::show(
        ctx,
        &mut layout,
        WindowKind::PianoRoll,
        workspace,
        &title,
        app,
        |app, ui| header(app, ui, project, channel),
        |app, ui| match (&pattern, channel) {
            (Some(pattern), Some(channel)) => body(app, ui, project, pattern, channel),
            _ => windows::placeholder(ui, ui.max_rect(), "Make a pattern and a channel to write notes"),
        },
    );
    app.prefs.layout = layout;
}

fn header(app: &mut BodoApp, ui: &mut Ui, project: &Project, channel: Option<ChannelId>) {
    let name = channel.and_then(|c| project.channels.get(&c)).map(|c| c.name.clone()).unwrap_or_else(|| String::from("No channel"));
    let combo = egui::ComboBox::from_id_salt("bodo-piano-channel").width(120.0).selected_text(name).show_ui(ui, |ui| {
        for id in project.rack_order.iter() {
            if let Some(c) = project.channels.get(id)
                && ui.selectable_label(channel == Some(*id), &c.name).clicked()
            {
                let _ = app.run("edit.select", json!({ "channel": id }));
            }
        }
    });
    control::record(ui, &combo.response, "Piano roll channel", "combo", None);
    app.hint(&combo.response, "The channel whose notes the piano roll edits");
}

/// Converts between ticks, keys and points.
#[derive(Clone, Copy, Debug)]
struct Grid {
    rect: Rect,
    zoom: f32,
    scroll: f64,
    top_key: f32,
    key_height: f32,
}

impl Grid {
    fn x(&self, tick: f64) -> f32 {
        self.rect.left() + ((tick - self.scroll) as f32) * self.zoom
    }

    fn tick(&self, x: f32) -> f64 {
        self.scroll + f64::from((x - self.rect.left()) / self.zoom)
    }

    /// The top of key `key`'s row.
    fn y(&self, key: u8) -> f32 {
        self.rect.top() + (self.top_key - f32::from(key)) * self.key_height
    }

    fn key(&self, y: f32) -> Option<u8> {
        let key = (self.top_key - (y - self.rect.top()) / self.key_height).ceil();
        (0.0..=127.0).contains(&key).then_some(key as u8)
    }
}

fn body(app: &mut BodoApp, ui: &mut Ui, project: &Project, pattern: &Pattern, channel: ChannelId) {
    let tk = Tokens::get();
    let rect = ui.max_rect();
    let keys = Rect::from_min_max(pos2(rect.left(), rect.top() + RULER), pos2(rect.left() + KEYS, rect.bottom()));
    let ruler = Rect::from_min_max(pos2(keys.right(), rect.top()), pos2(rect.right(), rect.top() + RULER));
    let canvas = Rect::from_min_max(pos2(keys.right(), ruler.bottom()), rect.max);
    navigate(app, ui, canvas.union(ruler).union(keys));
    let view = &app.view.piano;
    let grid = Grid { rect: canvas, zoom: view.zoom, scroll: view.scroll, top_key: view.top_key, key_height: view.key_height };
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(canvas, 0.0, tk.panel);
    painter.rect_filled(ruler, 0.0, tk.chrome_dark);

    // Key rows and the keyboard.
    let low = grid.key(canvas.bottom()).unwrap_or(0);
    let high = grid.key(canvas.top()).unwrap_or(127);
    for key in low..=high {
        let y = grid.y(key);
        let row = Rect::from_min_size(pos2(canvas.left(), y), vec2(canvas.width(), grid.key_height));
        if is_black(key) {
            painter.with_clip_rect(canvas).rect_filled(row, 0.0, tk.row_black);
        }
        painter.with_clip_rect(canvas).hline(
            canvas.x_range(),
            y + grid.key_height,
            Stroke::new(1.0, if key % 12 == 0 { tk.grid_bar } else { tk.grid_step }),
        );
        let key_rect = Rect::from_min_size(pos2(keys.left(), y), vec2(keys.width(), grid.key_height));
        let clip = painter.with_clip_rect(keys);
        clip.rect_filled(key_rect.shrink2(vec2(0.0, 0.5)), 0.0, if is_black(key) { tk.key_black } else { tk.key_white });
        if key % 12 == 0 {
            clip.text(
                key_rect.right_center() - vec2(3.0, 0.0),
                Align2::RIGHT_CENTER,
                key_name(key),
                FontId::proportional(9.5),
                theme::text_on(tk.key_white),
            );
        }
    }

    // Time grid.
    let length = bodo_engine::pattern_length(project, pattern.id);
    let meter = project.meter.signature_at(Tick::ZERO);
    let (bar, beat, step) = (meter.bar_ticks() as f64, meter.beat_ticks() as f64, STEP.0 as f64);
    let (first, last) = (grid.tick(canvas.left()).max(0.0), grid.tick(canvas.right()));
    let grid_painter = painter.with_clip_rect(canvas.union(ruler));
    for (spacing, color) in [(step, tk.grid_step), (beat, tk.grid_beat), (bar, tk.grid_bar)] {
        if spacing * f64::from(grid.zoom) < 6.0 {
            continue;
        }
        let mut t = (first / spacing).floor() * spacing;
        while t <= last {
            grid_painter.vline(grid.x(t), canvas.y_range(), Stroke::new(1.0, color));
            if spacing == bar {
                grid_painter.text(
                    pos2(grid.x(t) + 3.0, ruler.center().y),
                    Align2::LEFT_CENTER,
                    format!("{}", (t / bar) as i64 + 1),
                    FontId::monospace(size::SMALL),
                    tk.text_dim,
                );
            }
            t += spacing;
        }
    }
    let end = grid.x(length.0 as f64);
    if end < canvas.right() {
        grid_painter.rect_filled(Rect::from_min_max(pos2(end.max(canvas.left()), canvas.top()), canvas.max), 0.0, theme::faded(tk.border, 0.5));
    }

    // Notes: other channels faintly, this channel in its color.
    let view_start = Tick(grid.tick(canvas.left()).floor() as i64);
    let view_end = Tick(grid.tick(canvas.right()).ceil() as i64 + 1);
    let note_painter = painter.with_clip_rect(canvas);
    let selected = app.session.editor().notes.clone();
    let mut hit: Option<(NoteId, bool, i64)> = None;
    let pointer = ui.input(|i| i.pointer.hover_pos()).filter(|p| canvas.contains(*p));
    for (owner, seq) in pattern.notes.iter() {
        let own = *owner == channel;
        let color = project.channels.get(owner).map_or(tk.text_faint, |c| theme::color(c.color));
        for note in seq.overlapping(view_start, view_end) {
            if note.key < low || note.key > high {
                continue;
            }
            let y = grid.y(note.key);
            let rect = Rect::from_min_max(
                pos2(grid.x(note.start.0 as f64), y + 1.0),
                pos2(grid.x(note.end().0 as f64).max(grid.x(note.start.0 as f64) + 3.0), y + grid.key_height - 1.0),
            );
            if !own {
                note_painter.rect_filled(rect, 2.0, theme::faded(color, 0.25));
                continue;
            }
            let velocity = f32::from(note.velocity) / f32::from(u16::MAX);
            let fill = theme::faded(theme::tint(color, 0.45 + 0.55 * velocity), if note.muted { 0.4 } else { 1.0 });
            note_painter.rect_filled(rect, 2.0, fill);
            if selected.contains(&note.id) {
                note_painter.rect_stroke(rect, 2.0, Stroke::new(1.5, tk.accent), StrokeKind::Inside);
            }
            control::record_rect(
                ui,
                ui.id().with(("note", note.id)),
                rect,
                &format!("Note {} at {}", key_name(note.key), bodo_engine::params::format_position(project, note.start)),
                "note",
                Some(selected.contains(&note.id)),
                true,
            );
            if let Some(p) = pointer
                && rect.contains(p)
            {
                hit = Some((note.id, p.x >= rect.right() - EDGE, note.start.0));
            }
        }
    }
    playhead(app, &painter, project, canvas.union(ruler), &grid);
    input(app, ui, project, canvas, &grid, channel, hit);
}

fn input(app: &mut BodoApp, ui: &mut Ui, project: &Project, canvas: Rect, grid: &Grid, channel: ChannelId, hit: Option<(NoteId, bool, i64)>) {
    let response = ui.interact(canvas, ui.id().with("piano-canvas"), Sense::click_and_drag());
    control::record(ui, &response, "Piano roll grid", "canvas", None);
    app.hint(&response, "Click to add a note, drag to move it, drag its end to resize it, right-click to delete");
    if hit.is_some_and(|(_, edge, _)| edge) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    let Some(pointer) = response.interact_pointer_pos().or_else(|| response.hover_pos()) else { return };
    let snap = app.view.snap;
    let at = grid.tick(pointer.x);
    if let Some(drag) = app.view.piano.drag {
        if response.dragged() {
            let params = if drag.resize {
                let end = snap.round(project, at).max(drag.start + snap.ticks(project).max(1));
                app.view.piano.note_length = Tick(end - drag.start);
                json!({ "notes": [drag.note.0], "length": end - drag.start })
            } else {
                let start = snap.round(project, at - drag.grab).max(0);
                let key = grid.key(pointer.y).unwrap_or(60);
                json!({ "notes": [drag.note.0], "start": start, "key": key })
            };
            let _ = app.invoke(bodo_engine::Call::new("note.set_props", params).in_gesture(drag.gesture));
        }
        if response.drag_stopped() {
            app.view.piano.drag = None;
        }
        return;
    }
    if response.drag_started_by(egui::PointerButton::Primary)
        && let Some((note, edge, start)) = hit
    {
        let gesture = app.new_gesture();
        app.view.piano.drag = Some(NoteDrag { note, start, grab: at - start as f64, resize: edge, gesture });
        let _ = app.run("edit.select", json!({ "notes": [note.0] }));
    } else if response.clicked() {
        match hit {
            Some((note, _, _)) => {
                let _ = app.run("edit.select", json!({ "notes": [note.0] }));
            }
            None => {
                if let Some(key) = grid.key(pointer.y) {
                    let start = snap.floor(project, at).max(0);
                    let length = app.view.piano.note_length.0.max(1);
                    let _ = app.run("note.add_many", json!({ "channel": channel, "notes": [{ "key": key, "start": start, "length": length }] }));
                }
            }
        }
    } else if response.secondary_clicked()
        && let Some((note, _, _)) = hit
    {
        let _ = app.run("note.remove", json!({ "notes": [note.0] }));
    }
}

fn navigate(app: &mut BodoApp, ui: &Ui, area: Rect) {
    if !ui.rect_contains_pointer(area) {
        return;
    }
    let (delta, modifiers, pointer) = ui.input(|i| (i.smooth_scroll_delta, i.modifiers, i.pointer.hover_pos()));
    if delta == egui::Vec2::ZERO {
        return;
    }
    let view = &mut app.view.piano;
    let left = area.left() + KEYS;
    if modifiers.command || modifiers.ctrl {
        let anchor = pointer.map_or(left, |p| p.x);
        let before = view.scroll + f64::from((anchor - left) / view.zoom);
        view.zoom = (view.zoom * (1.0 + delta.y * 0.004)).clamp(8.0 / 3840.0, 4.0);
        view.scroll = (before - f64::from((anchor - left) / view.zoom)).max(0.0);
    } else if modifiers.shift || delta.x != 0.0 {
        let dx = if delta.x != 0.0 { delta.x } else { delta.y };
        view.scroll = (view.scroll - f64::from(dx / view.zoom)).max(0.0);
    } else {
        view.top_key = (view.top_key + delta.y / view.key_height).clamp(12.0, 127.0);
    }
}

fn playhead(app: &BodoApp, painter: &egui::Painter, _project: &Project, area: Rect, grid: &Grid) {
    if app.session.transport().mode != PlayMode::Pattern || !app.session.transport().playing {
        return;
    }
    let x = grid.x(app.session.transport().position.0 as f64);
    if x >= area.left() && x <= area.right() {
        painter.with_clip_rect(area).vline(x, area.y_range(), Stroke::new(1.5, Tokens::get().playhead));
    }
}
