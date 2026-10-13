//! The Playlist (project.md section 10.3): the pattern list, track headers, the ruler and the arrangement.
//!
//! Every tool of the section's table is here, with FL's keys: Draw (P), Paint (B), Delete (D), Mute (T), Slip (S),
//! Slice (C), Select (E), Zoom (Z) and Playback (Y); [`tools`] handles their input. Ctrl/Cmd+wheel zooms
//! horizontally, Alt+wheel vertically, Shift+wheel scrolls sideways. The ruler moves the song position
//! (click or drag), sets the loop region (Shift+drag) and holds markers (right-click to add one, right-click a
//! marker to remove it). Dragging a track header's bottom edge resizes the track.
//!
//! Drawing is culled: only visible tracks are visited, and each track's clips are found by binary search on their
//! sorted starts, widened by the track's longest clip, which is cached per track snapshot.

mod tools;

use std::collections::HashMap;
use std::sync::Arc;

use bodo_engine::PlayMode;
use bodo_kit_core::Tick;
use bodo_kit_ui::{Icon, Led, icons};
use bodo_model::playlist::ClipSource;
use bodo_model::{ClipId, PlaylistTrack, Project, TrackId};
use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
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
/// The height of a clip's name strip.
const HEADER_STRIP: f32 = 13.0;

/// A playlist tool.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Tool {
    /// Place, move and resize clips.
    #[default]
    Draw,
    /// Lay down copies along a drag.
    Paint,
    /// Delete clips.
    Delete,
    /// Mute clips.
    Mute,
    /// Slide content inside clips.
    Slip,
    /// Cut clips along a line.
    Slice,
    /// Select clips with a marquee.
    Select,
    /// Zoom the view.
    Zoom,
    /// Scrub and audition.
    Playback,
}

impl Tool {
    /// Every tool, in the header's order.
    pub const ALL: [Self; 9] = [Self::Draw, Self::Paint, Self::Delete, Self::Mute, Self::Slip, Self::Slice, Self::Select, Self::Zoom, Self::Playback];

    /// Its id, such as `draw`.
    pub fn label(self) -> &'static str {
        match self {
            Self::Draw => "draw",
            Self::Paint => "paint",
            Self::Delete => "delete",
            Self::Mute => "mute",
            Self::Slip => "slip",
            Self::Slice => "slice",
            Self::Select => "select",
            Self::Zoom => "zoom",
            Self::Playback => "playback",
        }
    }

    /// Its key, as FL users expect.
    pub fn key(self) -> egui::Key {
        match self {
            Self::Draw => egui::Key::P,
            Self::Paint => egui::Key::B,
            Self::Delete => egui::Key::D,
            Self::Mute => egui::Key::T,
            Self::Slip => egui::Key::S,
            Self::Slice => egui::Key::C,
            Self::Select => egui::Key::E,
            Self::Zoom => egui::Key::Z,
            Self::Playback => egui::Key::Y,
        }
    }

    /// Its icon in the playlist's and the piano roll's headers.
    pub(crate) fn icon(self) -> Icon {
        match self {
            Self::Draw => icons::PENCIL,
            Self::Paint => icons::BRUSH,
            Self::Delete => icons::ERASER,
            Self::Mute => icons::VOLUME_OFF,
            Self::Slip => icons::ARROWS_HORIZONTAL,
            Self::Slice => icons::SCISSORS,
            Self::Select => icons::MARQUEE_2,
            Self::Zoom => icons::ZOOM_IN,
            Self::Playback => icons::PLAYER_PLAY,
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Self::Draw => "Draw (P): click to place the current pattern, drag to move or resize, right-click to delete; Alt turns snap off",
            Self::Paint => "Paint (B): drag to lay down copies of the current pattern; right-drag deletes",
            Self::Delete => "Delete (D): click or drag over clips to delete them",
            Self::Mute => "Mute (T): click or drag over clips to mute or unmute them",
            Self::Slip => "Slip (S): drag a clip sideways to slide what plays inside it",
            Self::Slice => "Slice (C): drag a line to cut every clip it crosses; Shift draws a slanted line",
            Self::Select => "Select (E): drag a box to select clips; Shift adds or removes",
            Self::Zoom => "Zoom (Z): drag a box to zoom to it; click to zoom in, right-click to zoom out",
            Self::Playback => "Playback (Y): hold to play from the pointer, drag to scrub",
        }
    }
}

/// Sets the playlist's tool by id.
pub(crate) fn set_tool(app: &mut BodoApp, id: &str) -> Result<Value, String> {
    let tool = Tool::ALL
        .into_iter()
        .find(|t| t.label().eq_ignore_ascii_case(id))
        .ok_or("tool must be draw, paint, delete, mute, slip, slice, select, zoom or playback")?;
    app.view.playlist.tool = tool;
    Ok(json!({ "tool": tool.label() }))
}

/// The playlist's tool, zoom, scroll and the gesture in progress.
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
    drag: Option<tools::Drag>,
    /// The clip whose menu is open, and where.
    menu: Option<(ClipId, Pos2)>,
    /// The longest clip of each track, by the track snapshot it was measured on.
    longest: HashMap<TrackId, (usize, i64)>,
    ruler_drag: Option<(f64, bool)>,
    height_drag: Option<(TrackId, u64)>,
    /// The vertical zoom: track heights are drawn this many times their size.
    pub row_scale: f32,
}

impl Default for PlaylistView {
    fn default() -> Self {
        Self {
            tool: Tool::Draw,
            zoom: 72.0 / 3840.0,
            scroll: 0.0,
            scroll_y: 0.0,
            drag: None,
            menu: None,
            longest: HashMap::new(),
            ruler_drag: None,
            height_drag: None,
            row_scale: 1.0,
        }
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
    windows::show(ctx, &mut layout, WindowKind::Playlist, workspace, &title, app, header, |app, ui| body(app, ui, project));
    app.prefs.layout = layout;
}

fn header(app: &mut BodoApp, ui: &mut Ui) {
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
pub(crate) struct Scale {
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

/// A track's row on screen.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Row {
    id: TrackId,
    top: f32,
    height: f32,
}

/// A clip drawn this frame, for hit testing.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Shown {
    id: ClipId,
    track: TrackId,
    rect: Rect,
    start: i64,
    source: ClipSource,
}

/// What lies under a point of the canvas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Zone {
    /// A clip's name strip.
    Header,
    /// A clip's body.
    Body,
    /// A clip's right edge, which resizes it.
    Edge,
}

/// The canvas as the tools see it: its place, scale, rows and the clips drawn this frame.
pub(crate) struct Canvas {
    rect: Rect,
    scale: Scale,
    rows: Vec<Row>,
    shown: Vec<Shown>,
}

impl Canvas {
    /// The track under height `y`.
    fn row_at(&self, y: f32) -> Option<Row> {
        let index = self.rows.partition_point(|r| r.top <= y);
        index.checked_sub(1).and_then(|i| self.rows.get(i)).filter(|r| y < r.top + r.height).copied()
    }

    /// The topmost clip under `p`, and which part of it.
    fn clip_at(&self, p: Pos2) -> Option<(Shown, Zone)> {
        self.shown.iter().rev().find(|c| c.rect.contains(p)).map(|c| {
            let zone = if p.x >= c.rect.right() - EDGE {
                Zone::Edge
            } else if p.y <= c.rect.top() + HEADER_STRIP {
                Zone::Header
            } else {
                Zone::Body
            };
            (*c, zone)
        })
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
    navigate(app, ui, canvas.union(ruler).union(headers));
    let scale = Scale { left: canvas.left(), zoom: app.view.playlist.zoom, scroll: app.view.playlist.scroll };
    grid(ui, project, canvas, ruler, scale);
    loop_region(app, ui, canvas.union(ruler), scale);
    let rows = track_rows(project, canvas.top() - app.view.playlist.scroll_y, app.view.playlist.row_scale);
    track_headers(app, ui, project, headers, &rows);
    let shown = clips(app, ui, project, canvas, scale, &rows);
    let canvas = Canvas { rect: canvas, scale, rows, shown };
    tools::input(app, ui, project, &canvas);
    clip_menu(app, ui.ctx(), project);
    ruler_input(app, ui, project, ruler, scale);
    markers(app, ui, project, ruler, scale);
    playhead(app, ui, canvas.rect.union(ruler), scale);
}

/// Each track's row on screen, in order, at vertical zoom `scale`.
fn track_rows(project: &Project, top: f32, scale: f32) -> Vec<Row> {
    let mut y = top;
    project
        .track_order
        .iter()
        .filter_map(|id| project.tracks.get(id))
        .map(|track| {
            let height = track.height.clamp(16.0, 400.0) * scale;
            let row = Row { id: track.id, top: y, height };
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
    if modifiers.alt {
        // Alt+wheel zooms vertically: the view only, track heights stay as saved.
        view.row_scale = (view.row_scale * (1.0 + delta.y * 0.004)).clamp(0.4, 4.0);
        return;
    }
    let left = area.left() + HEADERS;
    if modifiers.command || modifiers.ctrl {
        let anchor = pointer.map_or(left, |p| p.x);
        let before = view.scroll + f64::from((anchor - left) / view.zoom);
        view.zoom = (view.zoom * (1.0 + delta.y * 0.004)).clamp(4.0 / 3840.0, 2.0);
        view.scroll = (before - f64::from((anchor - left) / view.zoom)).max(0.0);
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
    // A grid level shows only when its lines are at least 6 points apart, and never more than 2000 lines.
    for (spacing, color) in [(step, tk.grid_step), (beat, tk.grid_beat)] {
        if spacing * f64::from(scale.zoom) < 6.0 || (last - first) / spacing > 2000.0 {
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
    let line_every = (1..).map(|n: u32| 2f64.powi(n as i32 - 1)).find(|n| (last - first) / (n * bar) <= 600.0).unwrap_or(1.0);
    let mut b = (first / bar / line_every).floor() * line_every;
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
        b += line_every;
    }
}

/// Shades the loop region.
fn loop_region(app: &BodoApp, ui: &Ui, area: Rect, scale: Scale) {
    let Some((start, end)) = app.session.transport().loop_region else { return };
    let tk = Tokens::get();
    let rect = Rect::from_x_y_ranges(scale.x(start.0 as f64)..=scale.x(end.0 as f64), area.y_range()).intersect(area);
    if rect.width() > 0.0 {
        ui.painter().with_clip_rect(area).rect_filled(rect, 0.0, theme::faded(tk.accent, 0.08));
        let ruler_band = Rect::from_x_y_ranges(rect.x_range(), area.top()..=area.top() + 4.0);
        ui.painter().rect_filled(ruler_band, 0.0, tk.accent);
    }
}

fn track_headers(app: &mut BodoApp, ui: &mut Ui, project: &Project, rect: Rect, rows: &[Row]) {
    let tk = Tokens::get();
    let painter = ui.painter().with_clip_rect(rect);
    for row in rows {
        if row.top > rect.bottom() || row.top + row.height < rect.top() {
            continue;
        }
        let Some(track) = project.tracks.get(&row.id) else { continue };
        let area = Rect::from_min_size(pos2(rect.left(), row.top), vec2(rect.width(), row.height));
        painter.hline(area.x_range(), area.bottom() - 0.5, Stroke::new(1.0, tk.border));
        painter.text(
            area.left_top() + vec2(8.0, 6.0),
            Align2::LEFT_TOP,
            &track.name,
            FontId::proportional(size::SMALL),
            if track.muted { tk.text_faint } else { tk.text },
        );
        if track.locked {
            icons::LOCK.paint(&painter, Rect::from_min_size(area.left_top() + vec2(8.0, 22.0), vec2(11.0, 11.0)), tk.text_faint);
        }
        let led_rect = Rect::from_min_size(pos2(area.right() - 22.0, area.top() + 6.0), vec2(14.0, 14.0));
        if led_rect.intersects(rect) {
            let mut on = !track.muted;
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(led_rect).id_salt(("track-led", row.id)));
            child.set_clip_rect(rect);
            let led = child.add(Led::new(&mut on).label(format!("{} on", track.name)));
            control::record(&child, &led, &format!("{} on", track.name), "led", Some(!track.muted));
            if led.changed() {
                let _ = app.run("playlist.set_track_props", json!({ "track": row.id, "muted": !on }));
            }
        }
        // The bottom edge resizes the track.
        let edge = Rect::from_min_max(pos2(area.left(), area.bottom() - 4.0), pos2(area.right(), area.bottom() + 2.0)).intersect(rect);
        let resize = ui.interact(edge, ui.id().with(("track-height", row.id)), Sense::drag()).on_hover_cursor(egui::CursorIcon::ResizeVertical);
        if resize.drag_started() {
            app.view.playlist.height_drag = Some((row.id, app.new_gesture()));
        }
        if resize.dragged()
            && let Some((id, gesture)) = app.view.playlist.height_drag
            && let Some(pointer) = resize.interact_pointer_pos()
        {
            let height = ((pointer.y - row.top) / app.view.playlist.row_scale).clamp(16.0, 400.0);
            let _ = app.invoke(bodo_engine::Call::new("playlist.set_track_props", json!({ "track": id, "height": height })).in_gesture(gesture));
        }
        if resize.drag_stopped() {
            app.view.playlist.height_drag = None;
        }
        let header = ui.interact(area.intersect(rect), ui.id().with(("track-header", row.id)), Sense::click());
        header.context_menu(|ui| {
            if ui.button(if track.locked { "Unlock" } else { "Lock" }).clicked() {
                let _ = app.run("playlist.set_track_props", json!({ "track": row.id, "locked": !track.locked }));
            }
            if ui.button("Insert track above").clicked() {
                let position = project.track_order.index_of(&row.id).map_or(1, |i| i + 1);
                let _ = app.run("playlist.add_track", json!({ "at": position }));
            }
            if ui.button("Delete track").clicked() {
                let _ = app.run("playlist.remove_track", json!({ "track": row.id }));
            }
        });
    }
}

/// The index of the first clip that starts at `tick` or later; clips are sorted by start.
///
/// Complexity: O(log n) in clips.
fn first_starting_after(clips: &imbl::Vector<bodo_model::Clip>, tick: f64) -> usize {
    clips.binary_search_by(|c| if (c.start.0 as f64) < tick { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater }).unwrap_or_else(|i| i)
}

/// Draws the visible clips and returns them for hit testing.
fn clips(app: &mut BodoApp, ui: &mut Ui, project: &Project, canvas: Rect, scale: Scale, rows: &[Row]) -> Vec<Shown> {
    let tk = Tokens::get();
    let painter = ui.painter().with_clip_rect(canvas);
    let (view_start, view_end) = (scale.tick(canvas.left()), scale.tick(canvas.right()));
    let selected = app.session.editor().clips.clone();
    let mut shown = Vec::new();
    for row in rows {
        if row.top > canvas.bottom() || row.top + row.height < canvas.top() {
            continue;
        }
        let Some(track) = project.tracks.get(&row.id) else { continue };
        painter.hline(canvas.x_range(), row.top + row.height - 0.5, Stroke::new(1.0, tk.border));
        let longest = app.view.playlist.longest(track);
        let from = first_starting_after(&track.clips, view_start - longest as f64);
        for clip in track.clips.iter().skip(from).take_while(|c| (c.start.0 as f64) < view_end) {
            if (clip.end().0 as f64) < view_start {
                continue;
            }
            let rect =
                Rect::from_min_max(pos2(scale.x(clip.start.0 as f64), row.top + 1.0), pos2(scale.x(clip.end().0 as f64), row.top + row.height - 1.0));
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
            let header = Rect::from_min_max(rect.min, pos2(rect.right(), (rect.top() + HEADER_STRIP).min(rect.bottom())));
            painter.rect_filled(header, egui::CornerRadius { nw: 3, ne: 3, sw: 0, se: 0 }, theme::faded(color, alpha));
            painter.with_clip_rect(header.intersect(canvas)).text(
                header.left_center() + vec2(4.0, 0.0),
                Align2::LEFT_CENTER,
                &name,
                FontId::proportional(10.5),
                theme::text_on(color),
            );
            if let Some(pattern) = pattern {
                let offset = match clip.offset {
                    bodo_model::playlist::ClipOffset::Ticks(t) => t.0,
                    bodo_model::playlist::ClipOffset::Frames(_) => 0,
                };
                preview(
                    &painter,
                    project,
                    pattern,
                    clip.start.0,
                    clip.len.0,
                    offset,
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
            shown.push(Shown { id: clip.id, track: row.id, rect, start: clip.start.0, source: clip.source });
        }
    }
    shown
}

/// A clip's picture of its pattern: a short line per note, repeated across the clip from its offset, keys scaled to
/// its height.
#[allow(clippy::too_many_arguments, reason = "drawing needs the clip's place in time and on screen")]
fn preview(
    painter: &egui::Painter,
    project: &Project,
    pattern: &bodo_model::Pattern,
    start: i64,
    len: i64,
    offset: i64,
    rect: Rect,
    scale: Scale,
    alpha: f32,
) {
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
    let origin = start - offset.rem_euclid(length);
    let repeats = ((len + offset.rem_euclid(length) + length - 1) / length).min(17);
    let color = theme::faded(tk.text, 0.75 * alpha);
    for repeat in 0..repeats {
        let base = origin + repeat * length;
        for (channel_index, seq) in pattern.notes.values().enumerate() {
            for note in seq.notes() {
                let t = base + note.start.0;
                if t >= start + len {
                    break;
                }
                if t + note.len.0 <= start {
                    continue;
                }
                let x0 = scale.x(t.max(start) as f64);
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

/// The menu of a clip, opened by right-clicking its name strip.
fn clip_menu(app: &mut BodoApp, ctx: &egui::Context, project: &Project) {
    let Some((clip, at)) = app.view.playlist.menu else { return };
    let Some(source) = project.tracks.values().flat_map(|t| t.clips.iter()).find(|c| c.id == clip).map(|c| c.source) else {
        app.view.playlist.menu = None;
        return;
    };
    let mut chosen: Option<(&str, Value)> = None;
    let area = egui::Area::new(egui::Id::new("bodo-clip-menu")).order(egui::Order::Foreground).fixed_pos(at).show(ctx, |ui| {
        egui::Frame::menu(ui.style()).show(ui, |ui| {
            ui.set_min_width(160.0);
            if let ClipSource::Pattern(pattern) = source {
                if ui.button("Make unique").clicked() {
                    chosen = Some(("clip.make_unique", json!({ "clip": clip })));
                }
                if ui.button("Split by channel").clicked() {
                    chosen = Some(("pattern.split_by_channel", json!({ "pattern": pattern })));
                }
            }
            if ui.button("Mute or unmute").clicked() {
                chosen = Some(("clip.mute", json!({ "clips": [clip] })));
            }
            ui.menu_button("Color", |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.set_max_width(140.0);
                    for hue in bodo_engine::palette::HUES {
                        let (rect, swatch) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::click());
                        ui.painter().rect_filled(rect, 3.0, theme::color(hue));
                        if swatch.clicked() {
                            chosen = Some(("clip.set_color", json!({ "clips": [clip], "color": hue.to_string() })));
                        }
                    }
                });
                if ui.button("Default").clicked() {
                    chosen = Some(("clip.set_color", json!({ "clips": [clip], "color": null })));
                }
            });
            ui.separator();
            if ui.button("Delete").clicked() {
                chosen = Some(("clip.remove", json!({ "clips": [clip] })));
            }
        });
    });
    // A press outside the menu and its submenus closes it.
    let pressed_at = ctx.input(|i| if i.pointer.any_pressed() { i.pointer.interact_pos() } else { None });
    let outside =
        pressed_at.is_some_and(|p| !area.response.rect.contains(p) && ctx.layer_id_at(p).is_none_or(|l| l.order != egui::Order::Foreground));
    let dismissed = ctx.input(|i| i.key_pressed(egui::Key::Escape)) || outside;
    if let Some((id, params)) = chosen {
        let _ = app.run(id, params);
        app.view.playlist.menu = None;
    } else if dismissed {
        app.view.playlist.menu = None;
    }
}

fn ruler_input(app: &mut BodoApp, ui: &mut Ui, project: &Project, ruler: Rect, scale: Scale) {
    let response = ui.interact(ruler, ui.id().with("playlist-ruler"), Sense::click_and_drag());
    control::record(ui, &response, "Playlist ruler", "ruler", None);
    app.hint(&response, "The ruler: click or drag to move the song position, Shift+drag to set the loop, right-click to add a marker");
    let Some(pointer) = response.interact_pointer_pos().or_else(|| response.hover_pos()) else { return };
    let at = app.view.snap.floor(project, scale.tick(pointer.x)).max(0);
    let shift = ui.input(|i| i.modifiers.shift);
    if response.drag_started() && shift {
        // The loop starts where the press began, not where the drag was recognized.
        let origin = ui.input(|i| i.pointer.press_origin()).unwrap_or(pointer);
        let start = app.view.snap.round(project, scale.tick(origin.x)).max(0);
        app.view.playlist.ruler_drag = Some((start as f64, true));
    }
    if let Some((start, true)) = app.view.playlist.ruler_drag {
        if response.dragged() || response.drag_stopped() {
            let (a, b) = (start as i64, app.view.snap.round(project, scale.tick(pointer.x)).max(0));
            let (from, to) = (a.min(b), a.max(b));
            if to > from {
                let _ = app.run("transport.set_loop", json!({ "start": from, "end": to }));
            }
        }
        if response.drag_stopped() {
            app.view.playlist.ruler_drag = None;
        }
        return;
    }
    if response.secondary_clicked() {
        let _ = app.run("project.add_marker", json!({ "at": at }));
        return;
    }
    if response.clicked() || response.dragged() {
        if app.session.transport().mode != PlayMode::Song {
            let _ = app.run("transport.set_mode", json!({ "mode": "song" }));
        }
        let _ = app.run("transport.set_position", json!({ "at": at }));
    }
}

/// Markers on the ruler: a flag and a name; right-click one to remove it.
fn markers(app: &mut BodoApp, ui: &mut Ui, project: &Project, ruler: Rect, scale: Scale) {
    let tk = Tokens::get();
    let painter = ui.painter().with_clip_rect(ruler);
    for (index, marker) in project.markers.iter().enumerate() {
        let x = scale.x(marker.at.0 as f64);
        if x < ruler.left() - 120.0 || x > ruler.right() {
            continue;
        }
        let color = marker.color.map_or(tk.warn, theme::color);
        painter.vline(x, ruler.y_range(), Stroke::new(1.5, color));
        let galley = painter.layout_no_wrap(marker.name.clone(), FontId::proportional(10.5), tk.text);
        let flag = Rect::from_min_size(pos2(x, ruler.top() + 2.0), vec2(galley.size().x + 8.0, ruler.height() - 4.0));
        painter.rect_filled(flag, 2.0, theme::tint(color, 0.45));
        painter.galley(flag.left_center() + vec2(4.0, -galley.size().y / 2.0), galley, tk.text);
        let response = ui.interact(flag, ui.id().with(("marker", index)), Sense::click());
        control::record(ui, &response, &format!("Marker {}", marker.name), "marker", None);
        app.hint(&response, format!("Marker {}: click to go there, right-click to remove it", marker.name));
        if response.clicked() {
            let _ = app.run("transport.set_position", json!({ "at": marker.at.0 }));
        }
        if response.secondary_clicked() {
            let _ = app.run("project.remove_marker", json!({ "marker": index + 1 }));
        }
    }
}

fn playhead(app: &BodoApp, ui: &Ui, area: Rect, scale: Scale) {
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
    let tip = vec![pos2(x - 5.0, area.top()), pos2(x + 5.0, area.top()), pos2(x, area.top() + 7.0)];
    painter.add(egui::Shape::convex_polygon(tip, tk.playhead, Stroke::NONE));
}
