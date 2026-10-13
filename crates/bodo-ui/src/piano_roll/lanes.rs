//! The piano roll's lanes (project.md section 10.4).
//!
//! **Note lanes** (velocity, release, fine pitch, pan, Mod X and Mod Y) draw one value of each note as a stem at the
//! note's start. A brush stroke sets every note it passes over to the height of the pointer there; when some of the
//! channel's notes are selected, strokes change only those. A right-click puts the notes under the pointer back to
//! the default value.
//!
//! **Controller lanes** (pitch bend, channel pressure and any CC) are the pattern's own automation lanes for the
//! channel, drawn as a curve. A brush stroke draws a point on every snap step it crosses, and a right-drag erases the
//! points under it. Either kind of stroke is one undo step.

use std::collections::BTreeMap;

use bodo_kit_core::Tick;
use bodo_model::automation::{ChannelParam, ParamTarget};
use bodo_model::{ChannelId, Note, NoteId, Pattern, Project};
use egui::{Align2, FontId, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::{Value, json};

use super::{Grid, KEYS};
use crate::theme::{self, Tokens, size};
use crate::{BodoApp, control};

/// How close to a stem or a point the pointer has to pass to change it.
const REACH: f32 = 3.0;

/// What a lane edits: a value of each note, or a controller of the channel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Lane {
    /// How hard the note is struck.
    #[default]
    Velocity,
    /// How fast the key is let go.
    Release,
    /// Fine pitch, in cents.
    FinePitch,
    /// Pan.
    Pan,
    /// Mod X, mapped to a parameter by the instrument.
    ModX,
    /// Mod Y, mapped to a parameter by the instrument.
    ModY,
    /// MIDI pitch bend, a lane of the pattern.
    PitchBend,
    /// MIDI channel pressure, a lane of the pattern.
    Pressure,
    /// A MIDI control change, a lane of the pattern.
    Cc(u8),
}

impl Lane {
    /// The lanes of note values, in the menu's order.
    pub const NOTES: [Self; 6] = [Self::Velocity, Self::Release, Self::FinePitch, Self::Pan, Self::ModX, Self::ModY];

    /// The controller lanes the menu offers directly; any other CC is a number away.
    pub const CONTROLLERS: [Self; 8] =
        [Self::PitchBend, Self::Pressure, Self::Cc(1), Self::Cc(7), Self::Cc(10), Self::Cc(11), Self::Cc(64), Self::Cc(74)];

    /// Its name.
    pub fn label(self) -> String {
        match self {
            Self::Velocity => String::from("Velocity"),
            Self::Release => String::from("Release velocity"),
            Self::FinePitch => String::from("Fine pitch"),
            Self::Pan => String::from("Pan"),
            Self::ModX => String::from("Mod X"),
            Self::ModY => String::from("Mod Y"),
            Self::PitchBend => String::from("Pitch bend"),
            Self::Pressure => String::from("Channel pressure"),
            Self::Cc(n) => match n {
                1 => String::from("CC 1 Mod wheel"),
                7 => String::from("CC 7 Volume"),
                10 => String::from("CC 10 Pan"),
                11 => String::from("CC 11 Expression"),
                64 => String::from("CC 64 Sustain"),
                74 => String::from("CC 74 Brightness"),
                n => format!("CC {n}"),
            },
        }
    }

    /// A name short enough for the lane's corner.
    fn short(self) -> String {
        match self {
            Self::Velocity => String::from("Vel"),
            Self::Release => String::from("Rel"),
            Self::FinePitch => String::from("Fine"),
            Self::Pan => String::from("Pan"),
            Self::ModX => String::from("Mod X"),
            Self::ModY => String::from("Mod Y"),
            Self::PitchBend => String::from("Bend"),
            Self::Pressure => String::from("Press"),
            Self::Cc(n) => format!("CC {n}"),
        }
    }

    /// The channel parameter a controller lane automates; `None` for a note lane.
    pub fn controller(self) -> Option<ChannelParam> {
        match self {
            Self::PitchBend => Some(ChannelParam::PitchBend),
            Self::Pressure => Some(ChannelParam::Pressure),
            Self::Cc(n) => Some(ChannelParam::Cc(n)),
            _ => None,
        }
    }

    /// The `note.set_props` parameter a note lane sets.
    fn param(self) -> &'static str {
        match self {
            Self::Release => "releaseVelocity",
            Self::FinePitch => "finePitchCents",
            Self::Pan => "pan",
            Self::ModX => "modX",
            Self::ModY => "modY",
            _ => "velocity",
        }
    }

    /// Whether its values have a center line: fine pitch, pan and pitch bend.
    fn centered(self) -> bool {
        matches!(self, Self::FinePitch | Self::Pan | Self::PitchBend)
    }

    /// `note`'s value, from 0 at the bottom of the lane to 1 at the top.
    fn height(self, note: &Note) -> f32 {
        match self {
            Self::Release => f32::from(note.release_velocity) / f32::from(u16::MAX),
            Self::FinePitch => (note.fine_pitch_cents + 1200.0) / 2400.0,
            Self::Pan => (note.pan + 1.0) / 2.0,
            Self::ModX => note.mod_x,
            Self::ModY => note.mod_y,
            _ => f32::from(note.velocity) / f32::from(u16::MAX),
        }
        .clamp(0.0, 1.0)
    }

    /// The note parameter's value for height `h`, 0 to 1.
    fn value(self, h: f32) -> f64 {
        let h = f64::from(h.clamp(0.0, 1.0));
        match self {
            Self::FinePitch => (h * 2400.0 - 1200.0).round(),
            Self::Pan => ((h * 2.0 - 1.0) * 100.0).round() / 100.0,
            _ => (h * 1000.0).round() / 1000.0,
        }
    }

    /// The height a new note has.
    fn default_height(self) -> f32 {
        self.height(&Note::new(NoteId(0), 60, Tick::ZERO, Tick(1)))
    }

    fn describe(self, note: &Note) -> String {
        match self {
            Self::FinePitch => format!("{:+.0} cents", note.fine_pitch_cents),
            Self::Pan => format!("{:+.0}%", note.pan * 100.0),
            _ => format!("{:.0}%", self.height(note) * 100.0),
        }
    }
}

/// A brush stroke in progress: its gesture, where the pointer was last frame, and whether it erases.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Brush {
    gesture: u64,
    last: Pos2,
    erase: bool,
}

/// The bar between the notes and the lanes; dragging it changes the lanes' height.
pub(super) fn split(app: &mut BodoApp, ui: &mut Ui, bar: Rect, body: Rect) {
    let tk = Tokens::get();
    ui.painter().rect_filled(bar, 0.0, tk.chrome_dark);
    let response = ui.interact(bar, ui.id().with("piano-lane-split"), Sense::drag());
    control::record(ui, &response, "Piano roll lane divider", "splitter", None);
    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }
    if response.drag_started() {
        app.view.piano.resizing_lanes = true;
    }
    if app.view.piano.resizing_lanes {
        let height = (body.bottom() - response.interact_pointer_pos().map_or(bar.bottom(), |p| p.y)).clamp(0.0, body.height() * 0.6);
        app.view.piano.lane_height = height;
        if response.drag_stopped() || !response.dragged() {
            app.view.piano.resizing_lanes = false;
        }
    }
}

/// What the pointer did this frame over a lane.
struct Pointer {
    at: Pos2,
    primary: bool,
    secondary: bool,
    down: bool,
}

/// Draws the lane and handles its brush strokes.
///
/// Complexity: O(k) in the notes in view, or O(p log p) in the lane's points, per frame.
#[allow(clippy::too_many_arguments, reason = "the body's layout, kept apart for reading")]
pub(super) fn show(
    app: &mut BodoApp,
    ui: &mut Ui,
    project: &Project,
    rect: Rect,
    grid: &Grid,
    channel: ChannelId,
    pattern: &Pattern,
    notes: &[Note],
) {
    let tk = Tokens::get();
    let lane = app.view.piano.lane;
    let label = Rect::from_min_size(rect.min, vec2(KEYS, rect.height()));
    let area = Rect::from_min_max(pos2(label.right(), rect.top() + 4.0), pos2(rect.right(), rect.bottom() - 4.0));
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 0.0, tk.panel);
    painter.rect_filled(label, 0.0, tk.chrome_dark);
    if lane.centered() {
        painter.hline(area.x_range(), area.center().y, Stroke::new(1.0, tk.grid_beat));
    }
    lane_menu(app, ui, label);

    let response = ui.interact(area, ui.id().with("piano-lane"), Sense::click_and_drag());
    control::record(ui, &response, "Piano roll lane", "lane", None);
    let pointer = ui.input(|i| {
        i.pointer.latest_pos().map(|at| Pointer {
            at,
            primary: i.pointer.primary_pressed(),
            secondary: i.pointer.secondary_pressed(),
            down: i.pointer.any_down(),
        })
    });
    let starts = pointer.as_ref().filter(|p| (response.hovered() || response.is_pointer_button_down_on()) && area.contains(p.at));
    match lane.controller() {
        Some(param) => {
            app.hint(&response, "Drag to draw the controller on every snap step; right-drag erases points");
            if let Some(p) = starts
                && (p.primary || p.secondary)
            {
                app.view.piano.brush = Some(Brush { gesture: app.new_gesture(), last: p.at, erase: p.secondary });
            }
            controller(app, ui, project, area, grid, channel, pattern, param, pointer.as_ref());
        }
        None => {
            app.hint(&response, "Drag across the stems to set each note's value; right-click resets the notes under the pointer");
            if let Some(p) = starts {
                if p.primary {
                    app.view.piano.brush = Some(Brush { gesture: app.new_gesture(), last: p.at, erase: false });
                } else if p.secondary {
                    reset(app, grid, notes, lane, p.at);
                }
            }
            note_values(app, ui, project, area, grid, channel, notes, pointer.as_ref());
        }
    }
}

/// The notes a note lane's stroke may change: the selected ones, or all of them when none are.
fn editable<'a>(app: &BodoApp, notes: &'a [Note]) -> Vec<&'a Note> {
    let selected = &app.session.editor().notes;
    let any = notes.iter().any(|n| selected.contains(&n.id));
    notes.iter().filter(|n| !any || selected.contains(&n.id)).collect()
}

/// Puts the notes under `at` back to a new note's value.
fn reset(app: &mut BodoApp, grid: &Grid, notes: &[Note], lane: Lane, at: Pos2) {
    let ids: Vec<u32> = editable(app, notes).into_iter().filter(|n| (grid.x(n.start.0 as f64) - at.x).abs() <= REACH).map(|n| n.id.0).collect();
    if !ids.is_empty() {
        let gesture = app.new_gesture();
        let call = bodo_engine::Call::new("note.set_props", json!({ "notes": ids, lane.param(): lane.value(lane.default_height()) }));
        let _ = app.invoke(call.in_gesture(gesture));
    }
}

#[allow(clippy::too_many_arguments, reason = "the body's layout, kept apart for reading")]
fn note_values(
    app: &mut BodoApp,
    ui: &Ui,
    project: &Project,
    area: Rect,
    grid: &Grid,
    channel: ChannelId,
    notes: &[Note],
    pointer: Option<&Pointer>,
) {
    let tk = Tokens::get();
    let lane = app.view.piano.lane;
    let selected = app.session.editor().notes.clone();
    let any_selected = notes.iter().any(|n| selected.contains(&n.id));
    let color = project.channels.get(&channel).map_or(tk.accent, |c| theme::color(c.color));
    let stems = ui.painter().with_clip_rect(area.expand2(vec2(0.0, 4.0)));
    for note in notes {
        let x = grid.x(note.start.0 as f64);
        if x < area.left() - 4.0 || x > area.right() + 4.0 {
            continue;
        }
        let picked = selected.contains(&note.id);
        let stroke_color = if picked {
            tk.accent
        } else if any_selected {
            theme::faded(color, 0.45)
        } else {
            color
        };
        let base = if lane.centered() { area.center().y } else { area.bottom() };
        let top = area.bottom() - lane.height(note) * area.height();
        stems.line_segment([pos2(x, base), pos2(x, top)], Stroke::new(2.0, stroke_color));
        stems.circle_filled(pos2(x, top), 3.0, stroke_color);
        control::record_rect(
            ui,
            ui.id().with(("lane", note.id)),
            Rect::from_center_size(pos2(x, top), vec2(8.0, 8.0)),
            &format!("{} of {} at {}: {}", lane.label(), bodo_engine::params::key_name(note.key), note.start.0, lane.describe(note)),
            "lane-point",
            Some(picked),
            false,
        );
    }
    let (Some(brush), Some(pointer)) = (app.view.piano.brush, pointer) else { return };
    if !pointer.down {
        app.view.piano.brush = None;
        return;
    }
    // Every stem between last frame's pointer and this one's takes the height of the line between them.
    let (from, to) = if brush.last.x <= pointer.at.x { (brush.last, pointer.at) } else { (pointer.at, brush.last) };
    let mut calls = Vec::new();
    for note in editable(app, notes) {
        let x = grid.x(note.start.0 as f64);
        if x < from.x - REACH || x > to.x + REACH {
            continue;
        }
        let t = if to.x - from.x > 0.5 { ((x - from.x) / (to.x - from.x)).clamp(0.0, 1.0) } else { 1.0 };
        let y = from.y + (to.y - from.y) * t;
        let value = lane.value((area.bottom() - y) / area.height());
        if (lane.value(lane.height(note)) - value).abs() > 1e-6 {
            calls.push(bodo_engine::Call::new("note.set_props", json!({ "notes": [note.id.0], lane.param(): value })).in_gesture(brush.gesture));
        }
    }
    for call in calls {
        let _ = app.invoke(call);
    }
    app.view.piano.brush = Some(Brush { last: pointer.at, ..brush });
}

/// A controller lane: the curve of the pattern's lane for `param` of `channel`, drawn and erased by strokes.
#[allow(clippy::too_many_arguments, reason = "the body's layout, kept apart for reading")]
fn controller(
    app: &mut BodoApp,
    ui: &Ui,
    project: &Project,
    area: Rect,
    grid: &Grid,
    channel: ChannelId,
    pattern: &Pattern,
    param: ChannelParam,
    pointer: Option<&Pointer>,
) {
    let tk = Tokens::get();
    let target = ParamTarget::Channel(channel, param);
    let existing = pattern.automation.iter().find(|l| l.target == target);
    let points: BTreeMap<i64, f32> = existing.map(|l| l.points.iter().map(|p| (p.at.0, p.value)).collect()).unwrap_or_default();
    let rest = existing.map_or(if param == ChannelParam::PitchBend { 0.5 } else { 0.0 }, |l| l.default);
    let color = project.channels.get(&channel).map_or(tk.accent, |c| theme::color(c.color));
    let y_of = |v: f32| area.bottom() - v.clamp(0.0, 1.0) * area.height();
    // Controllers step from event to event, so the curve holds each value until the next point.
    let painter = ui.painter().with_clip_rect(area.expand2(vec2(0.0, 4.0)));
    let mut line = vec![pos2(area.left(), y_of(points.values().next().copied().unwrap_or(rest)))];
    for (&at, &value) in &points {
        let x = grid.x(at as f64);
        if let Some(last) = line.last().copied() {
            line.push(pos2(x, last.y));
        }
        line.push(pos2(x, y_of(value)));
    }
    if let Some(last) = line.last().copied() {
        line.push(pos2(area.right(), last.y));
    }
    painter.add(egui::Shape::line(line, Stroke::new(1.5, color)));
    for (&at, &value) in points.range(grid.tick(area.left()).floor() as i64..=grid.tick(area.right()).ceil() as i64) {
        let at_point = pos2(grid.x(at as f64), y_of(value));
        painter.circle_filled(at_point, 2.5, color);
        control::record_rect(
            ui,
            ui.id().with(("controller", at)),
            Rect::from_center_size(at_point, vec2(6.0, 6.0)),
            &format!("{} at {at}: {:.0}%", Lane::from_param(param).label(), value * 100.0),
            "lane-point",
            None,
            false,
        );
    }

    let (Some(brush), Some(pointer)) = (app.view.piano.brush, pointer) else { return };
    if !pointer.down {
        app.view.piano.brush = None;
        return;
    }
    let (from, to) = if brush.last.x <= pointer.at.x { (brush.last, pointer.at) } else { (pointer.at, brush.last) };
    let mut edited = points.clone();
    if brush.erase {
        edited.retain(|at, _| {
            let x = grid.x(*at as f64);
            x < from.x - REACH || x > to.x + REACH
        });
    } else {
        // A point on every snap step the stroke crossed this frame, at the stroke's height there.
        let step = app.view.snap.ticks(project).max(60);
        let (first, last) = (grid.tick(from.x).max(0.0), grid.tick(to.x).max(0.0));
        let mut at = ((first / step as f64).ceil() as i64) * step;
        if to.x - from.x < 0.5 {
            at = ((first / step as f64).round() as i64) * step;
        }
        while (at as f64) <= last.max(first) + 0.5 {
            let x = grid.x(at as f64);
            let t = if to.x - from.x > 0.5 { ((x - from.x) / (to.x - from.x)).clamp(0.0, 1.0) } else { 1.0 };
            let y = from.y + (to.y - from.y) * t;
            let value = ((area.bottom() - y) / area.height()).clamp(0.0, 1.0);
            edited.insert(at, (value * 1000.0).round() / 1000.0);
            at += step;
        }
    }
    if edited != points {
        let param_name = match param {
            ChannelParam::PitchBend => String::from("pitchBend"),
            ChannelParam::Pressure => String::from("pressure"),
            ChannelParam::Cc(n) => format!("cc:{n}"),
            other => format!("{other:?}").to_lowercase(),
        };
        let target = json!({ "channel": channel, "param": param_name });
        let curve = if param == ChannelParam::PitchBend { "linear" } else { "hold" };
        let call = if edited.is_empty() {
            bodo_engine::Call::new("automation.remove_lane", json!({ "pattern": pattern.id, "target": target }))
        } else {
            let points: Vec<Value> = edited.iter().map(|(at, value)| json!({ "at": at, "value": value, "curve": curve })).collect();
            bodo_engine::Call::new("automation.set_lane", json!({ "pattern": pattern.id, "target": target, "points": points }))
        };
        let _ = app.invoke(call.in_gesture(brush.gesture));
    }
    app.view.piano.brush = Some(Brush { last: pointer.at, ..brush });
}

impl Lane {
    /// The controller lane for `param`.
    fn from_param(param: ChannelParam) -> Self {
        match param {
            ChannelParam::PitchBend => Self::PitchBend,
            ChannelParam::Pressure => Self::Pressure,
            ChannelParam::Cc(n) => Self::Cc(n),
            _ => Self::Velocity,
        }
    }
}

/// The lane's name in its corner, a menu of the other lanes.
fn lane_menu(app: &mut BodoApp, ui: &mut Ui, label: Rect) {
    let lane = app.view.piano.lane;
    let tk = Tokens::get();
    let response = ui.interact(label, ui.id().with("piano-lane-kind"), Sense::click());
    control::record(ui, &response, "Piano roll lane kind", "combo", None);
    app.hint(&response, "Click to choose what the lane edits");
    ui.painter().text(label.center_top() + vec2(0.0, 10.0), Align2::CENTER_CENTER, lane.short(), FontId::proportional(size::SMALL), tk.text_dim);
    egui::Popup::menu(&response).show(|ui| {
        for choice in Lane::NOTES.into_iter().chain(Lane::CONTROLLERS) {
            if choice == Lane::PitchBend {
                ui.separator();
            }
            if ui.selectable_label(choice == lane, choice.label()).clicked() {
                app.view.piano.lane = choice;
            }
        }
        ui.horizontal(|ui| {
            ui.label("Other CC");
            let mut number = match lane {
                Lane::Cc(n) => n,
                _ => 2,
            };
            if ui.add(egui::DragValue::new(&mut number).range(0..=127)).changed() {
                app.view.piano.lane = Lane::Cc(number);
            }
        });
    });
}
