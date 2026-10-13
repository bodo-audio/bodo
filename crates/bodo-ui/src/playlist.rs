//! The Playlist (project.md section 10.3): the pattern list, track headers, the ruler and the arrangement.
//!
//! M2 has three tools. Draw (P): click empty space to place the current pattern, drag a clip to move it, drag its
//! right edge to resize it, right-click to delete it; clicking a clip makes its pattern current. Delete (D): click or
//! drag over clips. Mute (T): click clips. Ctrl/Cmd+wheel zooms, Shift+wheel scrolls sideways.
//!
//! Drawing is culled: only visible tracks are visited, and each track's clips are found by binary search on their
//! sorted starts, widened by the track's longest clip, which is cached per track snapshot.

use std::collections::HashMap;
use std::sync::Arc;

use bodo_engine::PlayMode;
use bodo_kit_core::Tick;
use bodo_kit_ui::{Icon, Led, icons};
use bodo_model::playlist::ClipSource;
use bodo_model::{ClipId, PlaylistTrack, Project, TrackId};
use egui::{Align2, Color32, FontId, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::{Value, json};

use crate::theme::{self, Tokens, size};
use crate::windows::{self, WindowKind, icon_button};
use crate::{BodoApp, control};

/// The pattern list's width.
const PATTERNS: f32 = 112.0;
/// The track headers' width.
const HEADERS: f32 = 108.0;
/// The ruler's height.
const RULER: f32 = 22.0;
/// How close to a clip's right edge a drag resizes it, in points.
const EDGE: f32 = 6.0;

/// A playlist tool.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Tool {
    /// Place, move and resize clips.
    #[default]
    Draw,
    /// Delete clips.
    Delete,
    /// Mute clips.
    Mute,
}

impl Tool {
    /// Every tool, in the header's order.
    pub const ALL: [Self; 3] = [Self::Draw, Self::Delete, Self::Mute];

    /// Its id, such as `draw`.
    pub fn label(self) -> &'static str {
        match self {
            Self::Draw => "draw",
            Self::Delete => "delete",
            Self::Mute => "mute",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Self::Draw => icons::PENCIL,
            Self::Delete => icons::ERASER,
            Self::Mute => icons::VOLUME_OFF,
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Self::Draw => "Draw (P): click to place the current pattern, drag to move or resize, right-click to delete",
            Self::Delete => "Delete (D): click or drag over clips to delete them",
            Self::Mute => "Mute (T): click clips to mute or unmute them",
        }
    }
}

/// Sets the playlist's tool by id.
pub(crate) fn set_tool(app: &mut BodoApp, id: &str) -> Result<Value, String> {
    let tool = Tool::ALL.into_iter().find(|t| t.label().eq_ignore_ascii_case(id)).ok_or("tool must be draw, delete or mute")?;
    app.view.playlist.tool = tool;
    Ok(json!({ "tool": tool.label() }))
}

/// A clip being dragged.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ClipDrag {
    clip: ClipId,
    /// Ticks between the clip's start and where it was grabbed.
    grab: f64,
    resize: bool,
    gesture: u64,
    moved: bool,
}

/// The playlist's tool, zoom and scroll.
#[derive(Debug)]
pub struct PlaylistView {
    /// The tool.
    pub tool: Tool,
    /// Points per tick.
    pub zoom: f32,
    /// The tick at the canvas's left edge.
    pub scroll: f64,
    /// Points scrolled down.
    pub scroll_y: f32,
    drag: Option<ClipDrag>,
    /// The longest clip of each track, by the track snapshot it was measured on.
    longest: HashMap<TrackId, (usize, i64)>,
}

impl Default for PlaylistView {
    fn default() -> Self {
        Self { tool: Tool::Draw, zoom: 72.0 / 3840.0, scroll: 0.0, scroll_y: 0.0, drag: None, longest: HashMap::new() }
    }
}

impl PlaylistView {
    /// The longest clip on `track`, measured again only when the track changed.
    fn longest(&mut self, track: &Arc<PlaylistTrack>) -> i64 {
        let key = Arc::as_ptr(track) as usize;
        match self.longest.get(&track.id) {
            Some((ptr, longest)) if *ptr == key => *longest,
            _ => {
                let longest = track.clips.iter().map(|c| c.len.0).max().unwrap_or(0);
                self.longest.insert(track.id, (key, longest));
                longest
            }
        }
    }
}

/// Draws the Playlist window.
pub(crate) fn window(app: &mut BodoApp, ctx: &egui::Context, workspace: Rect, project: &Arc<Project>) {
    let mut layout = std::mem::take(&mut app.prefs.layout);
    let pattern = app.session.editor().pattern.and_then(|p| project.patterns.get(&p)).map(|p| p.name.clone());
    let title = format!("Playlist - Arrangement{}", pattern.map(|p| format!(" > {p}")).unwrap_or_default());
    windows::show(ctx, &mut layout, WindowKind::Playlist, workspace, &title, app, tools, |app, ui| body(app, ui, project));
    app.prefs.layout = layout;
}

fn tools(app: &mut BodoApp, ui: &mut Ui) {
    for tool in Tool::ALL {
        let button = icon_button(ui, tool.icon(), &format!("Playlist tool {}", tool.label()), app.view.playlist.tool == tool);
        app.hint(&button, tool.hint());
        if button.clicked() {
            app.view.playlist.tool = tool;
        }
    }
}

/// Converts between ticks and points on the canvas.
#[derive(Clone, Copy, Debug)]
struct Scale {
    left: f32,
    zoom: f32,
    scroll: f64,
}

impl Scale {
    fn x(&self, tick: f64) -> f32 {
        self.left + ((tick - self.scroll) as f32) * self.zoom
    }

    fn tick(&self, x: f32) -> f64 {
        self.scroll + f64::from((x - self.left) / self.zoom)
    }
}

fn body(app: &mut BodoApp, ui: &mut Ui, project: &Arc<Project>) {
    let tk = Tokens::get();
    let rect = ui.max_rect();
    let patterns = Rect::from_min_max(rect.min, pos2(rect.left() + PATTERNS, rect.bottom()));
    let headers = Rect::from_min_max(pos2(patterns.right(), rect.top() + RULER), pos2(patterns.right() + HEADERS, rect.bottom()));
    let ruler = Rect::from_min_max(pos2(headers.right(), rect.top()), pos2(rect.right(), rect.top() + RULER));
    let canvas = Rect::from_min_max(pos2(headers.right(), ruler.bottom()), rect.max);
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(patterns, 0.0, tk.chrome);
    painter.rect_filled(headers, 0.0, tk.chrome);
    painter.rect_filled(ruler, 0.0, tk.chrome_dark);
    painter.rect_filled(canvas, 0.0, tk.panel);

    pattern_list(app, ui, project, patterns);
    navigate(app, ui, canvas.union(ruler));
    let scale = Scale { left: canvas.left(), zoom: app.view.playlist.zoom, scroll: app.view.playlist.scroll };
    grid(ui, project, canvas, ruler, scale);
    let rows = track_rows(project, canvas.top() - app.view.playlist.scroll_y);
    track_headers(app, ui, project, headers, &rows);
    clips(app, ui, project, canvas, scale, &rows);
    ruler_input(app, ui, project, ruler, scale);
    playhead(app, ui, project, canvas.union(ruler), scale);
}

/// Each track's id, top and height on screen, in order.
fn track_rows(project: &Project, top: f32) -> Vec<(TrackId, f32, f32)> {
    let mut y = top;
    project
        .track_order
        .iter()
        .filter_map(|id| project.tracks.get(id))
        .map(|track| {
            let height = track.height.clamp(24.0, 200.0);
            let row = (track.id, y, height);
            y += height;
            row
        })
        .collect()
}

fn pattern_list(app: &mut BodoApp, ui: &mut Ui, project: &Project, rect: Rect) {
    let tk = Tokens::get();
    let mut list = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(4.0, 4.0))).layout(egui::Layout::top_down(egui::Align::Min)));
    list.set_clip_rect(rect);
    egui::ScrollArea::vertical().id_salt("bodo-pattern-list").auto_shrink([false, false]).show(&mut list, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        for pattern in project.patterns.values() {
            let current = app.session.editor().pattern == Some(pattern.id);
            let (row, response) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::click());
            let fill = if current {
                tk.accent_soft
            } else if response.hovered() {
                tk.control_hover
            } else {
                Color32::TRANSPARENT
            };
            ui.painter().rect_filled(row, 4.0, fill);
            ui.painter().rect_filled(Rect::from_min_size(row.min + vec2(3.0, 5.0), vec2(4.0, 10.0)), 1.0, theme::color(pattern.color));
            let color = if current { tk.accent } else { tk.text };
            ui.painter().with_clip_rect(row).text(
                row.left_center() + vec2(12.0, 0.0),
                Align2::LEFT_CENTER,
                &pattern.name,
                FontId::proportional(size::SMALL),
                color,
            );
            control::record(ui, &response, &format!("Pattern {}", pattern.name), "pattern", Some(current));
            app.hint(&response, format!("{}: click to make it current; the draw tool places it", pattern.name));
            if response.clicked() {
                let _ = app.run("ui.select_pattern", json!({ "pattern": pattern.id }));
            }
        }
        let add = icon_button(ui, icons::PLUS, "New pattern", false);
        if add.clicked() {
            let _ = app.run("pattern.new", json!({}));
        }
    });
}

/// Scrolling and zooming with the wheel over the canvas.
fn navigate(app: &mut BodoApp, ui: &Ui, area: Rect) {
    if !ui.rect_contains_pointer(area) {
        return;
    }
    let (delta, modifiers, pointer) = ui.input(|i| (i.smooth_scroll_delta, i.modifiers, i.pointer.hover_pos()));
    if delta == egui::Vec2::ZERO {
        return;
    }
    let view = &mut app.view.playlist;
    if modifiers.command || modifiers.ctrl {
        let anchor = pointer.map_or(area.left(), |p| p.x);
        let before = view.scroll + f64::from((anchor - area.left()) / view.zoom);
        view.zoom = (view.zoom * (1.0 + delta.y * 0.004)).clamp(4.0 / 3840.0, 2.0);
        view.scroll = (before - f64::from((anchor - area.left()) / view.zoom)).max(0.0);
    } else if modifiers.shift || delta.x != 0.0 {
        let dx = if delta.x != 0.0 { delta.x } else { delta.y };
        view.scroll = (view.scroll - f64::from(dx / view.zoom)).max(0.0);
    } else {
        view.scroll_y = (view.scroll_y - delta.y).max(0.0);
    }
}

fn grid(ui: &Ui, project: &Project, canvas: Rect, ruler: Rect, scale: Scale) {
    let tk = Tokens::get();
    let painter = ui.painter().with_clip_rect(canvas.union(ruler));
    let meter = project.meter.signature_at(Tick::ZERO);
    let (bar, beat, step) = (meter.bar_ticks() as f64, meter.beat_ticks() as f64, bodo_engine::params::STEP.0 as f64);
    let first = scale.tick(canvas.left()).max(0.0);
    let last = scale.tick(canvas.right());
    // A grid level shows only when its lines are at least 6 points apart.
    for (spacing, color) in [(step, tk.grid_step), (beat, tk.grid_beat)] {
        if spacing * f64::from(scale.zoom) < 6.0 {
            continue;
        }
        let mut t = (first / spacing).floor() * spacing;
        while t <= last {
            if t % bar != 0.0 {
                painter.vline(scale.x(t), canvas.y_range(), Stroke::new(1.0, color));
            }
            t += spacing;
        }
    }
    let label_every = (1..).map(|n: u32| 2f64.powi(n as i32 - 1)).find(|n| n * bar * f64::from(scale.zoom) >= 34.0).unwrap_or(1.0);
    let mut b = (first / bar).floor();
    while b * bar <= last {
        let x = scale.x(b * bar);
        painter.vline(x, canvas.y_range(), Stroke::new(1.0, tk.grid_bar));
        if (b % label_every) == 0.0 {
            painter.text(
                pos2(x + 3.0, ruler.center().y),
                Align2::LEFT_CENTER,
                format!("{}", b as i64 + 1),
                FontId::monospace(size::SMALL),
                tk.text_dim,
            );
            painter.vline(x, ruler.y_range(), Stroke::new(1.0, tk.grid_bar));
        }
        b += 1.0;
    }
}

fn track_headers(app: &mut BodoApp, ui: &mut Ui, project: &Project, rect: Rect, rows: &[(TrackId, f32, f32)]) {
    let tk = Tokens::get();
    let painter = ui.painter().with_clip_rect(rect);
    for &(id, top, height) in rows {
        if top > rect.bottom() || top + height < rect.top() {
            continue;
        }
        let Some(track) = project.tracks.get(&id) else { continue };
        let row = Rect::from_min_size(pos2(rect.left(), top), vec2(rect.width(), height));
        painter.hline(row.x_range(), row.bottom() - 0.5, Stroke::new(1.0, tk.border));
        painter.text(
            row.left_top() + vec2(8.0, 6.0),
            Align2::LEFT_TOP,
            &track.name,
            FontId::proportional(size::SMALL),
            if track.muted { tk.text_faint } else { tk.text },
        );
        let led_rect = Rect::from_min_size(pos2(row.right() - 22.0, row.top() + 6.0), vec2(14.0, 14.0));
        if led_rect.intersects(rect) {
            let mut on = !track.muted;
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(led_rect).id_salt(("track-led", id)));
            child.set_clip_rect(rect);
            let led = child.add(Led::new(&mut on).label(format!("{} on", track.name)));
            control::record(&child, &led, &format!("{} on", track.name), "led", Some(!track.muted));
            if led.changed() {
                let _ = app.run("playlist.set_track_props", json!({ "track": id, "muted": !on }));
            }
        }
    }
}

/// The index of the first clip that starts at `tick` or later; clips are sorted by start.
///
/// Complexity: O(log n) in clips.
fn first_starting_after(clips: &imbl::Vector<bodo_model::Clip>, tick: f64) -> usize {
    clips.binary_search_by(|c| if (c.start.0 as f64) < tick { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater }).unwrap_or_else(|i| i)
}

/// The track row at height `y`.
fn row_at(rows: &[(TrackId, f32, f32)], y: f32) -> Option<TrackId> {
    let index = rows.partition_point(|&(_, top, _)| top <= y);
    index.checked_sub(1).and_then(|i| rows.get(i)).filter(|&&(_, top, height)| y < top + height).map(|&(id, _, _)| id)
}

#[allow(clippy::too_many_lines, reason = "the canvas draws clips and handles every tool's input in one pass")]
fn clips(app: &mut BodoApp, ui: &mut Ui, project: &Project, canvas: Rect, scale: Scale, rows: &[(TrackId, f32, f32)]) {
    let tk = Tokens::get();
    let response = ui.interact(canvas, ui.id().with("playlist-canvas"), Sense::click_and_drag());
    control::record(ui, &response, "Playlist canvas", "canvas", None);
    if response.hovered() {
        app.view.hint = Some(app.view.playlist.tool.hint().to_owned());
    }
    let painter = ui.painter().with_clip_rect(canvas);
    let pointer = response.interact_pointer_pos().or_else(|| response.hover_pos());
    let (view_start, view_end) = (scale.tick(canvas.left()), scale.tick(canvas.right()));
    let mut under_pointer: Option<(ClipId, TrackId, bool)> = None;
    let selected = app.session.editor().clips.clone();

    for &(track_id, top, height) in rows {
        if top > canvas.bottom() || top + height < canvas.top() {
            continue;
        }
        let Some(track) = project.tracks.get(&track_id) else { continue };
        painter.hline(canvas.x_range(), top + height - 0.5, Stroke::new(1.0, tk.border));
        let longest = app.view.playlist.longest(track);
        let from = first_starting_after(&track.clips, view_start - longest as f64);
        for clip in track.clips.iter().skip(from).take_while(|c| (c.start.0 as f64) < view_end) {
            if (clip.end().0 as f64) < view_start {
                continue;
            }
            let rect = Rect::from_min_max(pos2(scale.x(clip.start.0 as f64), top + 1.0), pos2(scale.x(clip.end().0 as f64), top + height - 1.0));
            let (name, color, pattern) = match clip.source {
                ClipSource::Pattern(p) => {
                    project.patterns.get(&p).map_or((String::from("?"), tk.text_faint, None), |p| (p.name.clone(), theme::color(p.color), Some(p)))
                }
                ClipSource::Audio(c) | ClipSource::Automation(c) => {
                    project.channels.get(&c).map_or((String::from("?"), tk.text_faint, None), |c| (c.name.clone(), theme::color(c.color), None))
                }
            };
            let color = clip.color.map_or(color, theme::color);
            let alpha = if clip.muted || track.muted { 0.4 } else { 1.0 };
            painter.rect_filled(rect, 3.0, theme::faded(theme::tint(color, 0.35), alpha));
            let header = Rect::from_min_max(rect.min, pos2(rect.right(), (rect.top() + 13.0).min(rect.bottom())));
            painter.rect_filled(header, egui::CornerRadius { nw: 3, ne: 3, sw: 0, se: 0 }, theme::faded(color, alpha));
            painter.with_clip_rect(header.intersect(canvas)).text(
                header.left_center() + vec2(4.0, 0.0),
                Align2::LEFT_CENTER,
                &name,
                FontId::proportional(10.5),
                theme::text_on(color),
            );
            if let Some(pattern) = pattern {
                preview(
                    &painter,
                    project,
                    pattern,
                    clip.start.0,
                    clip.len.0,
                    Rect::from_min_max(pos2(rect.left(), header.bottom() + 2.0), rect.max - vec2(0.0, 2.0)),
                    scale,
                    alpha,
                );
            }
            if selected.contains(&clip.id) {
                painter.rect_stroke(rect, 3.0, Stroke::new(1.5, tk.accent), StrokeKind::Inside);
            }
            control::record_rect(
                ui,
                ui.id().with(("clip", clip.id)),
                rect,
                &format!("Clip {name} at {}", bodo_engine::params::format_position(project, clip.start)),
                "clip",
                Some(selected.contains(&clip.id)),
                true,
            );
            if let Some(p) = pointer
                && rect.contains(p)
            {
                under_pointer = Some((clip.id, track_id, p.x >= rect.right() - EDGE));
            }
        }
    }

    let Some(pointer) = pointer else { return };
    let tool = app.view.playlist.tool;
    let snap = app.view.snap;
    let at = scale.tick(pointer.x);

    // Dragging a clip in progress.
    if let Some(mut drag) = app.view.playlist.drag {
        if response.dragged() {
            let clip = project.tracks.values().find_map(|t| t.clips.iter().find(|c| c.id == drag.clip).cloned());
            if let Some(clip) = clip {
                let call = if drag.resize {
                    let end = snap.round(project, at).max(clip.start.0 + snap.ticks(project).max(1));
                    (end - clip.start.0 != clip.len.0).then(|| ("clip.resize", json!({ "clips": [clip.id], "length": end - clip.start.0 })))
                } else {
                    let start = snap.round(project, at - drag.grab).max(0);
                    let track = row_at(rows, pointer.y);
                    let on_track = track.is_some_and(|t| project.tracks.get(&t).is_some_and(|t| t.clips.iter().any(|c| c.id == clip.id)));
                    (start != clip.start.0 || !on_track).then(|| ("clip.move", json!({ "clips": [clip.id], "start": start, "track": track })))
                };
                if let Some((id, params)) = call {
                    drag.moved = true;
                    let _ = app.invoke(bodo_engine::Call::new(id, params).in_gesture(drag.gesture));
                }
            }
            app.view.playlist.drag = Some(drag);
        }
        if response.drag_stopped() {
            app.view.playlist.drag = None;
        }
        return;
    }

    match tool {
        Tool::Draw => {
            if response.drag_started_by(egui::PointerButton::Primary)
                && let Some((clip, track, edge)) = under_pointer
            {
                let start = project.tracks.get(&track).and_then(|t| t.clips.iter().find(|c| c.id == clip)).map_or(0, |c| c.start.0);
                let gesture = app.new_gesture();
                app.view.playlist.drag = Some(ClipDrag { clip, grab: at - start as f64, resize: edge, gesture, moved: false });
                let _ = app.run("edit.select", json!({ "clips": [clip] }));
            } else if response.clicked() {
                match under_pointer {
                    Some((clip, track, _)) => {
                        let _ = app.run("edit.select", json!({ "clips": [clip], "track": track }));
                        let pattern = project.tracks.get(&track).and_then(|t| t.clips.iter().find(|c| c.id == clip)).and_then(|c| match c.source {
                            ClipSource::Pattern(p) => Some(p),
                            _ => None,
                        });
                        if let Some(pattern) = pattern {
                            let _ = app.run("ui.select_pattern", json!({ "pattern": pattern }));
                        }
                    }
                    None => {
                        if let Some(track) = row_at(rows, pointer.y)
                            && app.session.editor().pattern.is_some()
                        {
                            let start = snap.floor(project, at).max(0);
                            let _ = app.run("clip.place", json!({ "track": track, "start": start }));
                        }
                    }
                }
            } else if response.secondary_clicked()
                && let Some((clip, _, _)) = under_pointer
            {
                let _ = app.run("clip.remove", json!({ "clips": [clip] }));
            }
            if under_pointer.is_some_and(|(_, _, edge)| edge) {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
        }
        Tool::Delete => {
            if (response.clicked() || response.dragged() || response.secondary_clicked())
                && let Some((clip, _, _)) = under_pointer
            {
                let _ = app.run("clip.remove", json!({ "clips": [clip] }));
            }
        }
        Tool::Mute => {
            if response.clicked()
                && let Some((clip, _, _)) = under_pointer
            {
                let _ = app.run("clip.mute", json!({ "clips": [clip] }));
            }
        }
    }
}

/// A clip's picture of its pattern: a short line per note, repeated across the clip, keys scaled to its height.
#[allow(clippy::too_many_arguments, reason = "drawing needs the clip's place in time and on screen")]
fn preview(painter: &egui::Painter, project: &Project, pattern: &bodo_model::Pattern, start: i64, len: i64, rect: Rect, scale: Scale, alpha: f32) {
    if rect.height() < 4.0 {
        return;
    }
    let tk = Tokens::get();
    let length = bodo_engine::pattern_length(project, pattern.id).0.max(1);
    let (mut low, mut high) = (127u8, 0u8);
    let mut count = 0;
    for seq in pattern.notes.values() {
        for note in seq.notes() {
            low = low.min(note.key);
            high = high.max(note.key);
            count += 1;
        }
    }
    // Large patterns are left as plain clips; the preview stays cheap.
    if count == 0 || count > 512 {
        return;
    }
    let span = f32::from(high.saturating_sub(low).max(1));
    let repeats = ((len + length - 1) / length).min(16);
    let color = theme::faded(tk.text, 0.75 * alpha);
    for repeat in 0..repeats {
        let base = start + repeat * length;
        for (channel_index, seq) in pattern.notes.values().enumerate() {
            for note in seq.notes() {
                let t = base + note.start.0;
                if t >= start + len {
                    break;
                }
                let x0 = scale.x(t as f64);
                let x1 = scale.x((t + note.len.0).min(start + len) as f64).max(x0 + 1.5);
                let y = if high == low {
                    rect.top() + rect.height() * (0.2 + 0.6 * (channel_index % 4) as f32 / 4.0)
                } else {
                    rect.bottom() - 2.0 - (f32::from(note.key - low) / span) * (rect.height() - 4.0)
                };
                painter.line_segment([pos2(x0, y), pos2(x1, y)], Stroke::new(1.5, color));
            }
        }
    }
}

fn ruler_input(app: &mut BodoApp, ui: &mut Ui, project: &Project, ruler: Rect, scale: Scale) {
    let response = ui.interact(ruler, ui.id().with("playlist-ruler"), Sense::click_and_drag());
    control::record(ui, &response, "Playlist ruler", "ruler", None);
    app.hint(&response, "The ruler: click or drag to move the song position");
    if (response.clicked() || response.dragged())
        && let Some(p) = response.interact_pointer_pos()
    {
        let at = app.view.snap.floor(project, scale.tick(p.x)).max(0);
        if app.session.transport().mode != PlayMode::Song {
            let _ = app.run("transport.set_mode", json!({ "mode": "song" }));
        }
        let _ = app.run("transport.set_position", json!({ "at": at }));
    }
}

fn playhead(app: &BodoApp, ui: &Ui, _project: &Project, area: Rect, scale: Scale) {
    if app.session.transport().mode != PlayMode::Song {
        return;
    }
    let tk = Tokens::get();
    let x = scale.x(app.session.transport().position.0 as f64);
    if x < area.left() || x > area.right() {
        return;
    }
    let painter = ui.painter().with_clip_rect(area);
    painter.vline(x, area.y_range(), Stroke::new(1.5, tk.playhead));
    let tip = [pos2(x - 5.0, area.top()), pos2(x + 5.0, area.top()), pos2(x, area.top() + 7.0)];
    painter.add(egui::Shape::convex_polygon(tip.to_vec(), tk.playhead, Stroke::NONE));
}
