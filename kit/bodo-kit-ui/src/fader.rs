//! The volume fader (project.md section 14.4): a vertical fader in decibels, from silence (-inf dB) to +6 dB.
//!
//! The position follows the cube root of the linear gain, so the upper part of the travel, where mixing happens,
//! gets most of the resolution: 0 dB sits about 80% of the way up. Dragging snaps to exactly 0 dB within a small
//! detent around it; Shift drags finely, double-click resets to the default (0 dB unless set), and the wheel and the
//! arrow keys step it. Like the knob, it reports `changed()`, `drag_started()` and `drag_stopped()` through its
//! [`egui::Response`].

use egui::{EventFilter, Id, Modifiers, Rect, Response, Sense, Stroke, Ui, Vec2, Widget, WidgetInfo, pos2, vec2};

use crate::knob::{self, FINE, STEP};
use crate::palette::Palette;

/// The top of the fader, in dB.
pub const MAX_DB: f32 = 6.0;
/// How close to the 0 dB position, as a fraction of the travel, a drag snaps to exactly 0 dB.
pub const DETENT: f32 = 0.015;
/// The dB marks drawn when ticks are on.
pub const TICKS: [f32; 6] = [6.0, 0.0, -6.0, -12.0, -24.0, -48.0];

/// The linear gain at the top of the fader.
fn max_gain() -> f32 {
    db_to_gain(MAX_DB)
}

/// The linear gain of `db`; 0 for -inf.
pub fn db_to_gain(db: f32) -> f32 {
    if db == f32::NEG_INFINITY { 0.0 } else { 10f32.powf(db / 20.0) }
}

/// The level of a linear gain in dB; -inf for 0 or less.
pub fn gain_to_db(gain: f32) -> f32 {
    if gain > 0.0 { 20.0 * gain.log10() } else { f32::NEG_INFINITY }
}

/// Where `db` sits on the fader: 0 at the bottom (-inf dB), 1 at the top ([`MAX_DB`]).
pub fn db_to_position(db: f32) -> f32 {
    if db.is_nan() {
        return 0.0;
    }
    (db_to_gain(db.min(MAX_DB)) / max_gain()).cbrt().clamp(0.0, 1.0)
}

/// The level at position `position` (0 to 1) of the fader, in dB.
pub fn position_to_db(position: f32) -> f32 {
    let p = position.clamp(0.0, 1.0);
    if p <= 0.0 { f32::NEG_INFINITY } else { gain_to_db(max_gain() * p * p * p).min(MAX_DB) }
}

/// The level at a raw drag position, snapped to exactly 0 dB inside the [`DETENT`].
pub fn snapped_db(position: f32) -> f32 {
    if (position - db_to_position(0.0)).abs() < DETENT { 0.0 } else { position_to_db(position) }
}

/// Shows a level as `-3.2 dB`, `+1.5 dB`, `0.0 dB` or `-inf dB`.
pub fn format_db(db: f32) -> String {
    if db == f32::NEG_INFINITY || db < -200.0 {
        String::from("-inf dB")
    } else if db.abs() < 0.05 {
        String::from("0.0 dB")
    } else if db > 0.0 {
        format!("+{db:.1} dB")
    } else {
        format!("{db:.1} dB")
    }
}

/// A vertical fader that edits a level in dB.
#[must_use = "add it with `ui.add(fader)`"]
pub struct Fader<'a> {
    db: &'a mut f32,
    default: f32,
    label: String,
    size: Option<Vec2>,
    ticks: bool,
    palette: Option<Palette>,
}

impl<'a> Fader<'a> {
    /// A fader for the level `db`, which may be `f32::NEG_INFINITY`.
    pub fn new(db: &'a mut f32) -> Self {
        Self { db, default: 0.0, label: String::new(), size: None, ticks: false, palette: None }
    }

    /// The level a double-click resets to; 0 dB by default.
    pub fn default(mut self, db: f32) -> Self {
        self.default = db;
        self
    }

    /// What the fader controls, for the tooltip and screen readers.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// The fader's size in points; by default 1.92 by 11.2 times [`crate::em`], 24 by 140 beside 12.5-point text.
    pub fn size(mut self, size: Vec2) -> Self {
        self.size = Some(vec2(size.x.max(10.0), size.y.max(40.0)));
        self
    }

    /// Draws dB marks beside the groove at [`TICKS`].
    pub fn ticks(mut self, ticks: bool) -> Self {
        self.ticks = ticks;
        self
    }

    /// Colors for this fader instead of the installed [`Palette`].
    pub fn palette(mut self, palette: Palette) -> Self {
        self.palette = Some(palette);
        self
    }

    fn tooltip(&self, db: f32) -> String {
        if self.label.is_empty() { format_db(db) } else { format!("{}: {}", self.label, format_db(db)) }
    }
}

/// The handle's height in points.
const HANDLE: f32 = 14.0;

impl Widget for Fader<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let em = crate::em(ui);
        let size = self.size.unwrap_or_else(|| vec2(1.92 * em, 11.2 * em));
        let (rect, mut response) = ui.allocate_exact_size(size, Sense::click_and_drag());
        let travel = (rect.height() - HANDLE).max(1.0);
        let old = *self.db;
        let fine = ui.input(|i| i.modifiers.shift);
        let step = STEP * if fine { FINE } else { 1.0 };
        // While dragging, the unsnapped position lives in memory so the detent never traps the handle.
        let raw_id = response.id.with("raw");
        let mut new = old;

        if response.double_clicked() {
            new = self.default;
        } else if response.dragged() {
            let start = if response.drag_started() { db_to_position(old) } else { raw(ui, raw_id).unwrap_or_else(|| db_to_position(old)) };
            let position = (start - response.drag_delta().y / travel * if fine { FINE } else { 1.0 }).clamp(0.0, 1.0);
            ui.data_mut(|d| d.insert_temp(raw_id, position));
            new = snapped_db(position);
        } else if response.drag_stopped() {
            ui.data_mut(|d| d.remove::<f32>(raw_id));
        }
        let mut nudge = 0.0;
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                nudge += knob::wheel_steps(scroll) * step;
                ui.input_mut(|i| i.smooth_scroll_delta.y = 0.0);
            }
        }
        if response.has_focus() {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(response.id, EventFilter { horizontal_arrows: true, vertical_arrows: true, ..Default::default() })
            });
            let modifiers = if fine { Modifiers::SHIFT } else { Modifiers::NONE };
            let keys = ui.input_mut(|i| knob::KeySteps::take(i, modifiers));
            if keys != knob::KeySteps::default() {
                let position = keys.apply(db_to_position(new), step, Some(db_to_position(self.default)));
                nudge += position - db_to_position(new);
            }
        }
        if nudge != 0.0 {
            new = position_to_db((db_to_position(new) + nudge).clamp(0.0, 1.0));
        }
        if new != old && !(new.is_nan() && old.is_nan()) {
            *self.db = new;
            response.mark_changed();
        }

        let db = *self.db;
        let enabled = ui.is_enabled();
        let label = self.label.clone();
        // Screen readers hear the level in dB, "-inf dB" at the bottom.
        let text = format_db(db);
        response.widget_info(|| WidgetInfo { current_text_value: Some(text.clone()), ..WidgetInfo::slider(enabled, f64::from(db), &label) });

        if ui.is_rect_visible(rect) {
            let palette = self.palette.unwrap_or_else(|| Palette::get(ui.ctx()));
            paint(ui, rect, db_to_position(db), self.ticks, response.hovered() || response.dragged(), response.has_focus(), &palette);
        }
        if response.dragged() {
            response.show_tooltip_text(self.tooltip(db));
        } else {
            response = response.on_hover_ui(|ui| {
                ui.label(self.tooltip(db));
            });
        }
        response
    }
}

fn raw(ui: &Ui, id: Id) -> Option<f32> {
    ui.data(|d| d.get_temp::<f32>(id))
}

fn paint(ui: &Ui, rect: Rect, position: f32, ticks: bool, hot: bool, focused: bool, palette: &Palette) {
    let painter = ui.painter();
    let travel = rect.height() - HANDLE;
    let y_of = |p: f32| rect.bottom() - HANDLE / 2.0 - p * travel;
    let x = rect.center().x;
    let groove = 4.0;
    let top = y_of(1.0);
    let bottom = y_of(0.0);
    painter.rect_filled(Rect::from_min_max(pos2(x - groove / 2.0, top), pos2(x + groove / 2.0, bottom)), 2.0, palette.track);
    let y = y_of(position);
    painter.rect_filled(Rect::from_min_max(pos2(x - groove / 2.0, y), pos2(x + groove / 2.0, bottom)), 2.0, palette.fill.gamma_multiply(0.7));
    if ticks {
        for db in TICKS {
            let ty = y_of(db_to_position(db));
            let long = if db == 0.0 { 6.0 } else { 3.5 };
            let color = if db == 0.0 { palette.text } else { palette.text_dim };
            painter.line_segment([pos2(rect.left(), ty), pos2(rect.left() + long, ty)], Stroke::new(1.0, color));
        }
    }
    let handle = Rect::from_center_size(pos2(x, y), vec2(rect.width() - if ticks { 8.0 } else { 0.0 }, HANDLE));
    let handle = handle.translate(vec2(if ticks { 4.0 } else { 0.0 }, 0.0));
    painter.rect(handle, 3.0, if hot { palette.body_hover } else { palette.body }, Stroke::new(1.0, palette.outline), egui::StrokeKind::Inside);
    painter.line_segment([pos2(handle.left() + 3.0, y), pos2(handle.right() - 3.0, y)], Stroke::new(1.5, palette.accent));
    if focused {
        painter.rect_stroke(handle.expand(1.5), 4.0, Stroke::new(1.0, palette.accent), egui::StrokeKind::Outside);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_map_to_positions_and_back() {
        assert_eq!(db_to_position(f32::NEG_INFINITY), 0.0);
        assert!((db_to_position(MAX_DB) - 1.0).abs() < 1e-6);
        let zero = db_to_position(0.0);
        assert!(zero > 0.75 && zero < 0.85, "0 dB sits about 80% up: {zero}");
        assert_eq!(db_to_position(20.0), 1.0, "clamped at the top");
        assert_eq!(position_to_db(0.0), f32::NEG_INFINITY);
        for db in [-60.0, -24.0, -6.0, -0.5, 0.0, 3.0, 6.0] {
            assert!((position_to_db(db_to_position(db)) - db).abs() < 1e-3, "{db}");
        }
        // Monotonic: higher up is louder.
        let mut last = f32::NEG_INFINITY;
        for k in 1..=100 {
            let db = position_to_db(k as f32 / 100.0);
            assert!(db > last);
            last = db;
        }
    }

    #[test]
    fn the_detent_snaps_to_exactly_zero() {
        let zero = db_to_position(0.0);
        assert_eq!(snapped_db(zero + DETENT * 0.9), 0.0);
        assert_eq!(snapped_db(zero - DETENT * 0.9), 0.0);
        assert!(snapped_db(zero + DETENT * 1.5) > 0.0);
        assert!(snapped_db(zero - DETENT * 1.5) < 0.0);
    }

    #[test]
    fn gains_and_levels_convert() {
        assert_eq!(db_to_gain(f32::NEG_INFINITY), 0.0);
        assert!((db_to_gain(-6.0206) - 0.5).abs() < 1e-4);
        assert_eq!(gain_to_db(0.0), f32::NEG_INFINITY);
        assert!((gain_to_db(1.0)).abs() < 1e-6);
    }

    #[test]
    fn levels_format_with_a_sign() {
        assert_eq!(format_db(f32::NEG_INFINITY), "-inf dB");
        assert_eq!(format_db(0.0), "0.0 dB");
        assert_eq!(format_db(-0.01), "0.0 dB");
        assert_eq!(format_db(1.54), "+1.5 dB");
        assert_eq!(format_db(-12.0), "-12.0 dB");
    }
}
