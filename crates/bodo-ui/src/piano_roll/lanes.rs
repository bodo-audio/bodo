//! The piano roll's lanes (project.md section 10.4): one value of each note drawn as a stem at the note's start,
//! edited with brush strokes. A stroke sets every note it passes over to the height of the pointer there, and is one
//! undo step. When some of the channel's notes are selected, strokes change only those. A right-click puts the notes
//! under the pointer back to the default value.

use bodo_kit_core::Tick;
use bodo_model::{ChannelId, Note, NoteId, Project};
use egui::{Align2, FontId, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::json;

use super::{Grid, KEYS};
use crate::theme::{self, Tokens, size};
use crate::{BodoApp, control};

/// How close to a stem the pointer has to pass to change it.
const REACH: f32 = 3.0;

/// A value of each note that the lanes edit.
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
}

impl Lane {
    /// Every lane, in the menu's order.
    pub const ALL: [Self; 6] = [Self::Velocity, Self::Release, Self::FinePitch, Self::Pan, Self::ModX, Self::ModY];

    /// Its name.
    pub fn label(self) -> &'static str {
        match self {
            Self::Velocity => "Velocity",
            Self::Release => "Release velocity",
            Self::FinePitch => "Fine pitch",
            Self::Pan => "Pan",
            Self::ModX => "Mod X",
            Self::ModY => "Mod Y",
        }
    }

    /// A name short enough for the lane's corner.
    fn short(self) -> &'static str {
        match self {
            Self::Velocity => "Vel",
            Self::Release => "Rel",
            Self::FinePitch => "Fine",
            Self::Pan => "Pan",
            Self::ModX => "Mod X",
            Self::ModY => "Mod Y",
        }
    }

    /// The `note.set_props` parameter it sets.
    fn param(self) -> &'static str {
        match self {
            Self::Velocity => "velocity",
            Self::Release => "releaseVelocity",
            Self::FinePitch => "finePitchCents",
            Self::Pan => "pan",
            Self::ModX => "modX",
            Self::ModY => "modY",
        }
    }

    /// Whether its stems grow from the middle, for values with a center.
    fn centered(self) -> bool {
        matches!(self, Self::FinePitch | Self::Pan)
    }

    /// `note`'s value, from 0 at the bottom of the lane to 1 at the top.
    fn height(self, note: &Note) -> f32 {
        match self {
            Self::Velocity => f32::from(note.velocity) / f32::from(u16::MAX),
            Self::Release => f32::from(note.release_velocity) / f32::from(u16::MAX),
            Self::FinePitch => (note.fine_pitch_cents + 1200.0) / 2400.0,
            Self::Pan => (note.pan + 1.0) / 2.0,
            Self::ModX => note.mod_x,
            Self::ModY => note.mod_y,
        }
        .clamp(0.0, 1.0)
    }

    /// The parameter value for height `h`, 0 to 1.
    fn value(self, h: f32) -> f64 {
        let h = f64::from(h.clamp(0.0, 1.0));
        match self {
            Self::Velocity | Self::Release | Self::ModX | Self::ModY => (h * 1000.0).round() / 1000.0,
            Self::FinePitch => (h * 2400.0 - 1200.0).round(),
            Self::Pan => ((h * 2.0 - 1.0) * 100.0).round() / 100.0,
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

/// A brush stroke in progress: its gesture and where the pointer was last frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Brush {
    gesture: u64,
    last: Pos2,
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

/// Draws the lanes and handles their brush strokes.
///
/// Complexity: O(k) in the notes in view, per frame and per stroke step.
pub(super) fn show(app: &mut BodoApp, ui: &mut Ui, project: &Project, rect: Rect, grid: &Grid, channel: ChannelId, notes: &[Note]) {
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

    let selected = app.session.editor().notes.clone();
    let any_selected = notes.iter().any(|n| selected.contains(&n.id));
    let color = project.channels.get(&channel).map_or(tk.accent, |c| theme::color(c.color));
    let y_of = |h: f32| area.bottom() - h * area.height();
    let stems = painter.with_clip_rect(area.expand2(vec2(0.0, 4.0)));
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
        let top = y_of(lane.height(note));
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

    let response = ui.interact(area, ui.id().with("piano-lane"), Sense::click_and_drag());
    control::record(ui, &response, "Piano roll lane", "lane", None);
    app.hint(&response, "Drag across the stems to set each note's value; right-click resets the notes under the pointer");
    let (pointer, primary, secondary, down) =
        ui.input(|i| (i.pointer.latest_pos(), i.pointer.primary_pressed(), i.pointer.secondary_pressed(), i.pointer.primary_down()));
    let Some(pointer) = pointer else { return };
    let editable = |note: &&Note| !any_selected || selected.contains(&note.id);
    if (response.hovered() || response.is_pointer_button_down_on()) && area.contains(pointer) {
        if primary {
            app.view.piano.brush = Some(Brush { gesture: app.new_gesture(), last: pointer });
        } else if secondary {
            let gesture = app.new_gesture();
            let reset = lane.value(lane.default_height());
            let ids: Vec<u32> =
                notes.iter().filter(editable).filter(|n| (grid.x(n.start.0 as f64) - pointer.x).abs() <= REACH).map(|n| n.id.0).collect();
            if !ids.is_empty() {
                let _ = app.invoke(bodo_engine::Call::new("note.set_props", json!({ "notes": ids, lane.param(): reset })).in_gesture(gesture));
            }
        }
    }
    let Some(brush) = app.view.piano.brush else { return };
    if !down {
        app.view.piano.brush = None;
        return;
    }
    // Every stem between last frame's pointer and this one's takes the height of the line between them.
    let (from, to) = if brush.last.x <= pointer.x { (brush.last, pointer) } else { (pointer, brush.last) };
    for note in notes.iter().filter(editable) {
        let x = grid.x(note.start.0 as f64);
        if x < from.x - REACH || x > to.x + REACH {
            continue;
        }
        let t = if to.x - from.x > 0.5 { ((x - from.x) / (to.x - from.x)).clamp(0.0, 1.0) } else { 1.0 };
        let y = from.y + (to.y - from.y) * t;
        let h = (area.bottom() - y) / area.height();
        let value = lane.value(h);
        if (lane.value(lane.height(note)) - value).abs() > 1e-6 {
            let call = bodo_engine::Call::new("note.set_props", json!({ "notes": [note.id.0], lane.param(): value }));
            let _ = app.invoke(call.in_gesture(brush.gesture));
        }
    }
    app.view.piano.brush = Some(Brush { last: pointer, ..brush });
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
        for choice in Lane::ALL {
            if ui.selectable_label(choice == lane, choice.label()).clicked() {
                app.view.piano.lane = choice;
            }
        }
    });
}
