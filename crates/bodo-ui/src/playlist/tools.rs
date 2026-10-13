//! The playlist tools' input (project.md section 10.3).
//!
//! A press on the canvas starts a gesture for the current tool, moves update it, and the release finishes it. Every
//! change a gesture makes runs as commands of one gesture, so it is one undo step. Selection, zoom and scrubbing
//! never enter the history.
//!
//! The draw tool also works the handles on audio and automation clips: dragging a fade handle sets the fade, dragging
//! an automation point moves it, a double-click on an automation clip adds a point and a right-click on a point
//! removes it. A sound dropped from the browser becomes an audio clip where it lands.

use bodo_engine::{Call, PlayMode};
use bodo_model::playlist::ClipSource;
use bodo_model::{ChannelId, ClipId, Project, TrackId};
use egui::{Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2};
use serde_json::json;

use super::media::{self, Handle};
use super::{Canvas, Shown, Source, Tool, Zone, content_rect, current_source, source_length};
use crate::browser::SoundPayload;
use crate::theme::{self, Tokens};
use crate::{BodoApp, Snap, control};

/// The most clips one frame of painting lays down, so a fast drag can't flood the playlist.
const PAINT_PER_FRAME: usize = 32;

/// A gesture in progress on the canvas.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Drag {
    /// The draw tool's new clip of `source`, placed on release.
    Place { track: TrackId, start: i64, source: Source },
    /// A clip being moved; `grab` is where it was held, in ticks from its start.
    Move { clip: ClipId, grab: f64, gesture: u64 },
    /// A clip's right edge being dragged.
    Resize { clip: ClipId, start: i64, gesture: u64 },
    /// An audio clip's fade being dragged: the clip's start and end, and whether it is the fade-out.
    Fade { clip: ClipId, start: i64, end: i64, out: bool, gesture: u64 },
    /// A point of automation channel `channel` being dragged: its index, the tick its lane counts from, and the
    /// clip's content area, which maps heights to values.
    Point { channel: ChannelId, index: usize, origin: i64, content: Rect, gesture: u64 },
    /// The bend of the curve after point `index` of automation channel `channel` being dragged: where the press was,
    /// the tension the curve had then, whether it is an S-curve, whether the segment rises, and the clip's content
    /// height.
    Tension { channel: ChannelId, index: usize, from_y: f32, start: f32, scurve: bool, rising: bool, height: f32, gesture: u64 },
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
    double_clicked: bool,
    down: bool,
    shift: bool,
    alt: bool,
}

/// Handles the current tool's input on the canvas and draws its overlays.
pub(super) fn input(app: &mut BodoApp, ui: &mut Ui, project: &Project, canvas: &Canvas) {
    let response = ui.interact(canvas.rect, ui.id().with("playlist-canvas"), Sense::click_and_drag());
    control::record(ui, &response, "Playlist canvas", "canvas", None);
    drop_sound(app, ui, project, canvas, &response);
    let tool = app.view.playlist.tool;
    if response.hovered() {
        app.view.hint = Some(tool.hint().to_owned());
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
        app.view.playlist.drag = None;
        return;
    }
    let pressed_here = response.is_pointer_button_down_on() || response.hovered();
    if app.view.playlist.drag.is_none() && pressed_here && canvas.rect.contains(pointer.at) {
        if pointer.primary_pressed {
            let presses = &mut app.view.playlist.presses;
            *presses = [presses[1], canvas.clip_at(pointer.at).map(|(clip, _)| clip.id)];
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
    if pointer.double_clicked && tool == Tool::Draw && canvas.rect.contains(pointer.at) {
        add_point(app, project, canvas, pointer);
    }
    let ghost = match app.view.playlist.drag {
        Some(Drag::Place { track, start, source }) => {
            let length = source_length(app, ui.ctx(), project, source, start);
            canvas.rows.iter().find(|r| r.id == track).map(|row| {
                let rect = Rect::from_min_max(
                    pos2(canvas.scale.x(start as f64), row.top + 1.0),
                    pos2(canvas.scale.x((start + length) as f64), row.top + row.height - 1.0),
                );
                (rect, source.look(project).map(|(_, color)| color))
            })
        }
        _ => None,
    };
    overlay(app, ui, canvas, pointer, ghost);
    // Cursors that say what a press would do.
    if response.hovered() && tool == Tool::Draw {
        match canvas.clip_at(pointer.at) {
            Some((_, Zone::Edge | Zone::Handle(Handle::FadeIn | Handle::FadeOut))) => ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal),
            Some((_, Zone::Handle(Handle::Point(_)))) => ui.ctx().set_cursor_icon(egui::CursorIcon::Grab),
            Some((_, Zone::Handle(Handle::Tension(_)))) => ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical),
            _ => {}
        }
    }
}

/// A sound dragged from the browser and dropped on a track becomes an audio clip there, and the clip source.
fn drop_sound(app: &mut BodoApp, ui: &Ui, project: &Project, canvas: &Canvas, response: &egui::Response) {
    let Some(at) = ui.input(|i| i.pointer.latest_pos()) else { return };
    let (Some(row), true) = (canvas.row_at(at.y), canvas.rect.contains(at)) else { return };
    let start = app.view.snap.floor(project, canvas.scale.tick(at.x)).max(0);
    if response.dnd_hover_payload::<SoundPayload>().is_some() {
        let tk = Tokens::get();
        let band = Rect::from_min_max(pos2(canvas.scale.x(start as f64), row.top + 1.0), pos2(canvas.rect.right(), row.top + row.height - 1.0));
        let painter = ui.painter().with_clip_rect(canvas.rect);
        painter.rect(band, 3.0, theme::faded(tk.accent, 0.12), Stroke::new(1.0, tk.accent), StrokeKind::Inside);
    }
    if let Some(payload) = response.dnd_release_payload::<SoundPayload>() {
        let placed = app.run("clip.place_sound", json!({ "track": row.id, "sample": payload.0, "start": start }));
        if let Some(channel) = placed.ok().and_then(|r| serde_json::from_value::<ChannelId>(r["channel"].clone()).ok()) {
            app.view.playlist.source = Some(channel);
        }
    }
}

fn snap(app: &BodoApp, pointer: Pointer) -> Snap {
    if pointer.alt { Snap::None } else { app.view.snap }
}

/// The clips of `track` hold a clip at `start` already.
fn occupied(canvas: &Canvas, track: TrackId, start: i64) -> bool {
    canvas.shown.iter().any(|c| c.track == track && c.start == start)
}

/// The length of the first clip starting at `start` on `track`.
///
/// Complexity: O(log n) in the track's clips.
fn clip_length_at(project: &Project, track: TrackId, start: i64) -> Option<i64> {
    let clips = &project.tracks.get(&track)?.clips;
    clips.get(super::first_starting_after(clips, start as f64)).filter(|c| c.start.0 == start).map(|c| c.len.0)
}

/// The clip `shown` stands for, found by binary search on its track's starts.
///
/// Complexity: O(log n + k) in the track's clips and clips starting at the same tick.
fn clip_of<'p>(project: &'p Project, shown: &Shown) -> Option<&'p bodo_model::Clip> {
    let clips = &project.tracks.get(&shown.track)?.clips;
    let from = super::first_starting_after(clips, shown.start as f64);
    clips.iter().skip(from).take_while(|c| c.start.0 == shown.start).find(|c| c.id == shown.id)
}

fn begin(app: &mut BodoApp, project: &Project, canvas: &Canvas, tool: Tool, pointer: Pointer) {
    let at = canvas.scale.tick(pointer.at.x);
    let hit = canvas.clip_at(pointer.at);
    let drag = match tool {
        Tool::Draw => match hit {
            Some((clip, Zone::Handle(handle))) => begin_handle(app, project, &clip, handle, pointer.at.y),
            Some((clip, Zone::Edge)) => Some(Drag::Resize { clip: clip.id, start: clip.start, gesture: app.new_gesture() }),
            Some((clip, _)) => {
                select_clip(app, &clip);
                Some(Drag::Move { clip: clip.id, grab: at - clip.start as f64, gesture: app.new_gesture() })
            }
            None => match (canvas.row_at(pointer.at.y), current_source(app, project)) {
                (Some(row), Some(source)) => Some(Drag::Place { track: row.id, start: snap(app, pointer).floor(project, at).max(0), source }),
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

/// Starts dragging a fade, automation point or curve tension handle of `shown`, pressed at height `y`.
fn begin_handle(app: &mut BodoApp, project: &Project, shown: &Shown, handle: Handle, y: f32) -> Option<Drag> {
    let clip = clip_of(project, shown)?;
    let gesture = app.new_gesture();
    let header = Rect::from_min_max(shown.rect.min, pos2(shown.rect.right(), (shown.rect.top() + super::HEADER_STRIP).min(shown.rect.bottom())));
    let content = content_rect(shown.rect, header);
    match (handle, clip.source) {
        (Handle::FadeIn | Handle::FadeOut, _) => {
            Some(Drag::Fade { clip: clip.id, start: clip.start.0, end: clip.end().0, out: handle == Handle::FadeOut, gesture })
        }
        (Handle::Point(index), ClipSource::Automation(channel)) => {
            Some(Drag::Point { channel, index, origin: media::lane_origin(clip), content, gesture })
        }
        (Handle::Tension(index), ClipSource::Automation(channel)) => {
            let lane = media::lane(project, channel)?;
            let (from, to) = (lane.points.get(index)?, lane.points.get(index + 1)?);
            let (start, scurve) = media::tension(from.curve);
            let rising = to.value > from.value;
            Some(Drag::Tension { channel, index, from_y: y, start, scurve, rising, height: content.height().max(8.0), gesture })
        }
        (Handle::Point(_) | Handle::Tension(_), _) => None,
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
        Tool::Draw => match hit {
            // Right-clicking an automation point removes it, not its clip.
            Some((clip, Zone::Handle(Handle::Point(index)))) => {
                if let ClipSource::Automation(channel) = clip.source {
                    let _ = app.run("automation.remove_point", json!({ "channel": channel, "index": index }));
                }
            }
            Some((clip, _)) => {
                let _ = app.run("clip.remove", json!({ "clips": [clip.id] }));
            }
            None => {}
        },
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

/// Selects `clip` and makes its source current, as clicking a clip does in FL.
fn select_clip(app: &mut BodoApp, clip: &Shown) {
    let _ = app.run("edit.select", json!({ "clips": [clip.id], "track": clip.track }));
    match clip.source {
        ClipSource::Pattern(pattern) => {
            app.view.playlist.source = None;
            if app.session.editor().pattern != Some(pattern) {
                let _ = app.run("ui.select_pattern", json!({ "pattern": pattern }));
            }
        }
        ClipSource::Audio(channel) | ClipSource::Automation(channel) => app.view.playlist.source = Some(channel),
    }
}

/// A double-click on an automation clip, off its points, adds a point there. Both presses must land on the clip, so a
/// double-click that first placed the clip doesn't also add a point to it.
fn add_point(app: &mut BodoApp, project: &Project, canvas: &Canvas, pointer: Pointer) {
    let Some((shown, Zone::Body | Zone::Header)) = canvas.clip_at(pointer.at) else { return };
    if app.view.playlist.presses != [Some(shown.id); 2] {
        return;
    }
    let (ClipSource::Automation(channel), Some(clip)) = (shown.source, clip_of(project, &shown)) else { return };
    let header = Rect::from_min_max(shown.rect.min, pos2(shown.rect.right(), (shown.rect.top() + super::HEADER_STRIP).min(shown.rect.bottom())));
    let content = content_rect(shown.rect, header);
    if !content.contains(pointer.at) {
        return;
    }
    let tick = snap(app, pointer).round(project, canvas.scale.tick(pointer.at.x)).clamp(clip.start.0, clip.end().0);
    let at = (tick - media::lane_origin(clip)).max(0);
    let _ = app.run("automation.add_point", json!({ "channel": channel, "at": at, "value": media::y_value(content, pointer.at.y) }));
}

fn run_in(app: &mut BodoApp, gesture: u64, id: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
    app.invoke(Call::new(id, params).in_gesture(gesture))
}

fn update(app: &mut BodoApp, project: &Project, canvas: &Canvas, pointer: Pointer) {
    let Some(drag) = app.view.playlist.drag.clone() else { return };
    let at = canvas.scale.tick(pointer.at.x);
    let snap = snap(app, pointer);
    let hit = canvas.clip_at(pointer.at);
    let next = match drag {
        Drag::Place { source, .. } => match canvas.row_at(pointer.at.y) {
            Some(row) => Drag::Place { track: row.id, start: snap.floor(project, at).max(0), source },
            None => drag,
        },
        Drag::Move { clip, grab, gesture } => {
            let start = snap.round(project, at - grab).max(0);
            let track = canvas.row_at(pointer.at.y).map(|r| r.id);
            let current = canvas.shown.iter().find(|c| c.id == clip);
            let moved = current.is_none_or(|c| c.start != start || track.is_some_and(|t| t != c.track));
            if moved {
                let _ = run_in(app, gesture, "clip.move", json!({ "clips": [clip], "start": start, "track": track }));
            }
            drag
        }
        Drag::Resize { clip, start, gesture } => {
            let end = snap.round(project, at).max(start + snap.ticks(project).max(1));
            let _ = run_in(app, gesture, "clip.resize", json!({ "clips": [clip], "length": end - start }));
            drag
        }
        Drag::Fade { clip, start, end, out, gesture } => {
            let tick = snap.round(project, at).clamp(start, end);
            let params = if out { json!({ "clips": [clip], "fadeOut": end - tick }) } else { json!({ "clips": [clip], "fadeIn": tick - start }) };
            let _ = run_in(app, gesture, "clip.set_fades", params);
            drag
        }
        Drag::Point { channel, index, origin, content, gesture } => {
            move_point(app, project, pointer, snap.round(project, at) - origin, (channel, index, content, gesture));
            drag
        }
        Drag::Tension { channel, index, from_y, start, scurve, rising, height, gesture } => {
            // Dragging down bends the curve below its straight line: a rising segment then rises late, which is a
            // positive tension, and a falling one falls early, a negative one.
            let down = (pointer.at.y - from_y) / height * 2.0;
            let tension = ((start + if rising { down } else { -down }) * 100.0).round() / 100.0;
            let tension = tension.clamp(-1.0, 1.0);
            let curve = if scurve { json!({ "sCurve": tension }) } else { json!({ "exp": tension }) };
            let unchanged = media::lane(project, channel).and_then(|l| l.points.get(index)).is_some_and(|p| media::curve_json(p.curve) == curve);
            if !unchanged {
                let _ = run_in(app, gesture, "automation.set_curve", json!({ "channel": channel, "index": index, "curve": curve }));
            }
            drag
        }
        Drag::Paint { gesture, next } => paint(app, project, canvas, pointer, gesture, next),
        Drag::Erase { gesture } => {
            if let Some((clip, _)) = hit {
                let _ = run_in(app, gesture, "clip.remove", json!({ "clips": [clip.id] }));
            }
            drag
        }
        Drag::Mute { gesture, mut done } => {
            if let Some((clip, _)) = hit
                && !done.contains(&clip.id)
            {
                let _ = run_in(app, gesture, "clip.mute", json!({ "clips": [clip.id] }));
                done.push(clip.id);
            }
            Drag::Mute { gesture, done }
        }
        Drag::Slip { clip, gesture, last } => {
            let grid = snap.ticks(project).max(1) as f64;
            let by = ((at - last) / grid).trunc() * grid;
            if by != 0.0 {
                let _ = run_in(app, gesture, "clip.slip", json!({ "clips": [clip], "by": by as i64 }));
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

/// Moves point `index` of an automation lane to `local` ticks from the lane's start, kept between its neighbors, and to
/// the value at the pointer's height in the clip's content area.
fn move_point(app: &mut BodoApp, project: &Project, pointer: Pointer, local: i64, (channel, index, content, gesture): (ChannelId, usize, Rect, u64)) {
    let Some(lane) = media::lane(project, channel) else { return };
    let low = index.checked_sub(1).and_then(|i| lane.points.get(i)).map_or(0, |p| p.at.0);
    let high = lane.points.get(index + 1).map_or(i64::MAX, |p| p.at.0);
    let (at, value) = (local.clamp(low, high.max(low)), media::y_value(content, pointer.at.y));
    if lane.points.get(index).is_none_or(|p| p.at.0 == at && p.value == value) {
        return;
    }
    let points: Vec<serde_json::Value> = lane
        .points
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let (at, value) = if i == index { (at, value) } else { (p.at.0, p.value) };
            json!({ "at": at, "value": value, "curve": media::curve_json(p.curve) })
        })
        .collect();
    let _ = run_in(app, gesture, "automation.set_points", json!({ "channel": channel, "points": points }));
}

/// Lays down copies of the current clip source along the drag: a copy where the pointer is when it enters a track,
/// then one after another, each as long as the copy before, as the pointer moves on.
fn paint(app: &mut BodoApp, project: &Project, canvas: &Canvas, pointer: Pointer, gesture: u64, next: Option<(TrackId, i64)>) -> Drag {
    let (Some(row), Some(source)) = (canvas.row_at(pointer.at.y), current_source(app, project)) else { return Drag::Paint { gesture, next } };
    let at = canvas.scale.tick(pointer.at.x).max(0.0);
    let mut slot = match next {
        Some((track, slot)) if track == row.id => slot,
        _ => snap(app, pointer).floor(project, at).max(0),
    };
    let mut placed = 0;
    while (slot as f64) <= at && placed < PAINT_PER_FRAME {
        let mut length = match source {
            Source::Pattern(pattern) => bodo_engine::pattern_length(project, pattern).0.max(1),
            Source::Automation(channel) => media::automation_length(project, channel),
            Source::Audio(_) => 0,
        };
        if occupied(canvas, row.id, slot) {
            // A clip already there sets the step to the next copy when the source's own length isn't known.
            if length == 0 {
                length = clip_length_at(project, row.id, slot).unwrap_or(0);
            }
        } else {
            let (id, params) = source.place(row.id, slot);
            // An audio clip is as long as its sample; the command says how long that is.
            if let Ok(result) = run_in(app, gesture, id, params)
                && length == 0
            {
                length = result["ticks"].as_i64().unwrap_or(0);
            }
        }
        if length <= 0 {
            break;
        }
        slot += length;
        placed += 1;
    }
    Drag::Paint { gesture, next: Some((row.id, slot)) }
}

fn finish(app: &mut BodoApp, project: &Project, canvas: &Canvas, pointer: Pointer) {
    let Some(drag) = app.view.playlist.drag.take() else { return };
    match drag {
        Drag::Place { track, start, source } => {
            let (id, params) = source.place(track, start);
            let _ = app.run(id, params);
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
                    let _ = run_in(app, gesture, "clip.slice", json!({ "at": at, "tracks": [row.id] }));
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
        Drag::Move { .. }
        | Drag::Resize { .. }
        | Drag::Fade { .. }
        | Drag::Point { .. }
        | Drag::Tension { .. }
        | Drag::Paint { .. }
        | Drag::Erase { .. }
        | Drag::Mute { .. }
        | Drag::Slip { .. } => {}
    }
}

/// Zooms by `factor` keeping the tick under `x` in place.
fn zoom_around(app: &mut BodoApp, canvas: &Canvas, x: f32, factor: f32) {
    let anchor = canvas.scale.tick(x);
    let view = &mut app.view.playlist;
    view.zoom = (view.zoom * factor).clamp(4.0 / 3840.0, 2.0);
    view.scroll = (anchor - f64::from((x - canvas.rect.left()) / view.zoom)).max(0.0);
}

/// Draws the gesture in progress: the new clip (`ghost`, with its source's color), the cutting line, the selection or
/// zoom box.
fn overlay(app: &BodoApp, ui: &Ui, canvas: &Canvas, pointer: Pointer, ghost: Option<(Rect, Option<egui::Color32>)>) {
    let tk = Tokens::get();
    let painter = ui.painter().with_clip_rect(canvas.rect);
    match &app.view.playlist.drag {
        Some(Drag::Place { .. }) => {
            if let Some((rect, color)) = ghost {
                let color = color.unwrap_or(tk.accent);
                painter.rect(rect, 3.0, theme::faded(color, 0.35), Stroke::new(1.0, color), StrokeKind::Inside);
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
