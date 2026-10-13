//! The knob (project.md section 14.4).
//!
//! Drag vertically (or horizontally) to turn it, with Shift for fine control; double-click resets it to its
//! default; the mouse wheel and the arrow keys (when it has keyboard focus) step it. Hovering shows the value and
//! unit. The arc fills from an origin, so a pan knob fills outward from the center.
//!
//! The knob changes the value it borrows and reports what happened through the usual [`egui::Response`]:
//! `changed()` on every change, `drag_started()` and `drag_stopped()` around a gesture, so a host can fold the whole
//! gesture into one undo step.

use std::f32::consts::PI;
use std::ops::RangeInclusive;

use egui::{EventFilter, Key, Modifiers, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2, Widget, WidgetInfo, vec2};

use crate::palette::Palette;

/// Points of drag that sweep the whole range.
pub const DRAG_POINTS: f32 = 200.0;
/// How much finer Shift makes a drag, a wheel step or an arrow key.
pub const FINE: f32 = 0.1;
/// Points of wheel scrolling that make one step.
pub const WHEEL_POINTS: f32 = 50.0;
/// The default step, as a fraction of the range: a hundredth.
pub const STEP: f32 = 0.01;

/// Where the arc starts, in radians clockwise from the right: lower left.
const START: f32 = 0.75 * PI;
/// How far the arc sweeps: three quarters of a turn.
const SWEEP: f32 = 1.5 * PI;

/// The direction a drag turns the knob in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DragAxis {
    /// Dragging up increases the value.
    #[default]
    Vertical,
    /// Dragging right increases the value.
    Horizontal,
}

/// `value` as a fraction of `range`: 0 at the start, 1 at the end. An empty range gives 0.
pub fn normalize(value: f32, range: &RangeInclusive<f32>) -> f32 {
    let span = range.end() - range.start();
    if span.abs() <= f32::EPSILON || !value.is_finite() { 0.0 } else { ((value - range.start()) / span).clamp(0.0, 1.0) }
}

/// The value at fraction `t` of `range`.
pub fn denormalize(t: f32, range: &RangeInclusive<f32>) -> f32 {
    range.start() + t.clamp(0.0, 1.0) * (range.end() - range.start())
}

/// The change, as a fraction of the range, that a drag of `points` toward larger values makes; a tenth as much
/// when `fine`.
pub fn drag_change(points: f32, fine: bool) -> f32 {
    points / DRAG_POINTS * if fine { FINE } else { 1.0 }
}

/// How many steps `points` of wheel scrolling make; positive scrolls up, toward larger values.
pub fn wheel_steps(points: f32) -> f32 {
    points / WHEEL_POINTS
}

/// The default display of a value: decimals fitting the size of the range, then the unit.
pub fn format_value(value: f32, range: &RangeInclusive<f32>, unit: &str) -> String {
    let span = (range.end() - range.start()).abs();
    let decimals = if span >= 100.0 {
        0
    } else if span >= 10.0 {
        1
    } else {
        2
    };
    let number = format!("{value:.decimals$}");
    // "-0.0" reads as an error.
    let number =
        if number.trim_start_matches('-').chars().all(|c| c == '0' || c == '.') { number.trim_start_matches('-').to_owned() } else { number };
    if unit.is_empty() { number } else { format!("{number} {unit}") }
}

/// A knob that edits an `f32` in a range.
#[must_use = "add it with `ui.add(knob)`"]
pub struct Knob<'a> {
    value: &'a mut f32,
    range: RangeInclusive<f32>,
    default: Option<f32>,
    origin: Option<f32>,
    label: String,
    unit: String,
    format: Option<fn(f32) -> String>,
    diameter: f32,
    axis: DragAxis,
    step: f32,
    palette: Option<Palette>,
}

impl<'a> Knob<'a> {
    /// A knob for `value` within `range`.
    pub fn new(value: &'a mut f32, range: RangeInclusive<f32>) -> Self {
        Self {
            value,
            range,
            default: None,
            origin: None,
            label: String::new(),
            unit: String::new(),
            format: None,
            diameter: 24.0,
            axis: DragAxis::Vertical,
            step: STEP,
            palette: None,
        }
    }

    /// The value a double-click resets to. Without one, double-clicking does nothing.
    pub fn default(mut self, default: f32) -> Self {
        self.default = Some(default);
        self
    }

    /// Where the value arc starts. Without one it starts at the start of the range; a pan knob starts at 0.
    pub fn origin(mut self, origin: f32) -> Self {
        self.origin = Some(origin);
        self
    }

    /// What the knob controls, for the tooltip and screen readers.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// The unit shown after the value, such as `dB` or `%`.
    pub fn unit(mut self, unit: impl Into<String>) -> Self {
        self.unit = unit.into();
        self
    }

    /// How to show the value, unit included, instead of the default display.
    pub fn format(mut self, format: fn(f32) -> String) -> Self {
        self.format = Some(format);
        self
    }

    /// The knob's diameter in points; 24 by default.
    pub fn diameter(mut self, diameter: f32) -> Self {
        self.diameter = diameter.max(8.0);
        self
    }

    /// Which way a drag turns the knob; vertical by default.
    pub fn axis(mut self, axis: DragAxis) -> Self {
        self.axis = axis;
        self
    }

    /// The wheel and arrow-key step as a fraction of the range; [`STEP`] by default.
    pub fn step(mut self, step: f32) -> Self {
        self.step = step.abs();
        self
    }

    /// Colors for this knob instead of the installed [`Palette`].
    pub fn palette(mut self, palette: Palette) -> Self {
        self.palette = Some(palette);
        self
    }

    fn text(&self, value: f32) -> String {
        match self.format {
            Some(format) => format(value),
            None => format_value(value, &self.range, &self.unit),
        }
    }
}

impl Widget for Knob<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let (rect, mut response) = ui.allocate_exact_size(Vec2::splat(self.diameter), Sense::click_and_drag());
        let old = *self.value;
        let mut t = normalize(old, &self.range);
        let fine = ui.input(|i| i.modifiers.shift);
        let step = self.step * if fine { FINE } else { 1.0 };

        if response.double_clicked() {
            if let Some(default) = self.default {
                t = normalize(default, &self.range);
            }
        } else if response.dragged() {
            let delta = response.drag_delta();
            let points = match self.axis {
                DragAxis::Vertical => -delta.y,
                DragAxis::Horizontal => delta.x,
            };
            t += drag_change(points, fine);
        }
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                t += wheel_steps(scroll) * step;
                // The knob took the scroll: a scroll area around it must not move too.
                ui.input_mut(|i| i.smooth_scroll_delta.y = 0.0);
            }
        }
        if response.has_focus() {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(response.id, EventFilter { horizontal_arrows: true, vertical_arrows: true, ..Default::default() })
            });
            let modifiers = if fine { Modifiers::SHIFT } else { Modifiers::NONE };
            let (up, down) = ui.input_mut(|i| {
                let up = i.consume_key(modifiers, Key::ArrowUp) | i.consume_key(modifiers, Key::ArrowRight);
                let down = i.consume_key(modifiers, Key::ArrowDown) | i.consume_key(modifiers, Key::ArrowLeft);
                (up, down)
            });
            if up {
                t += step;
            }
            if down {
                t -= step;
            }
        }

        let new = if t == normalize(old, &self.range) { old } else { denormalize(t, &self.range) };
        if new != old {
            *self.value = new;
            response.mark_changed();
        }
        let value = *self.value;
        let enabled = ui.is_enabled();
        let label = self.label.clone();
        response.widget_info(|| WidgetInfo::slider(enabled, f64::from(value), &label));

        if ui.is_rect_visible(rect) {
            let palette = self.palette.unwrap_or_else(|| Palette::get(ui.ctx()));
            let hot = response.hovered() || response.dragged();
            let origin = normalize(self.origin.unwrap_or(*self.range.start()), &self.range);
            paint(ui, rect, normalize(value, &self.range), origin, hot, response.has_focus(), &palette);
        }

        if response.dragged() {
            response.show_tooltip_text(self.tooltip(value));
        } else {
            response = response.on_hover_ui(|ui| {
                ui.label(self.tooltip(value));
            });
        }
        response
    }
}

impl Knob<'_> {
    fn tooltip(&self, value: f32) -> String {
        if self.label.is_empty() { self.text(value) } else { format!("{}: {}", self.label, self.text(value)) }
    }
}

/// The point at fraction `t` of the arc around `center`.
fn arc_point(center: Pos2, radius: f32, t: f32) -> Pos2 {
    let angle = START + SWEEP * t;
    center + vec2(angle.cos(), angle.sin()) * radius
}

/// An arc from fraction `a` to `b` as a polyline.
fn arc(center: Pos2, radius: f32, a: f32, b: f32, stroke: Stroke) -> Shape {
    let (a, b) = if a <= b { (a, b) } else { (b, a) };
    let n = ((SWEEP * (b - a) * radius / 2.0).ceil() as usize).clamp(2, 64);
    let points = (0..=n).map(|k| arc_point(center, radius, a + (b - a) * k as f32 / n as f32)).collect();
    Shape::line(points, stroke)
}

fn paint(ui: &Ui, rect: Rect, t: f32, origin: f32, hot: bool, focused: bool, palette: &Palette) {
    let painter = ui.painter();
    let center = rect.center();
    let radius = rect.width().min(rect.height()) / 2.0;
    let width = (radius * 0.2).max(2.0);
    let ring = radius - width / 2.0;
    painter.add(arc(center, ring, 0.0, 1.0, Stroke::new(width, palette.track)));
    if (t - origin).abs() > 1e-4 {
        painter.add(arc(center, ring, origin, t, Stroke::new(width, palette.fill)));
    }
    let body = radius - width * 1.6;
    painter.circle(center, body, if hot { palette.body_hover } else { palette.body }, Stroke::new(1.0, palette.outline));
    let tip = arc_point(center, body * 0.85, t);
    let base = arc_point(center, body * 0.3, t);
    painter.line_segment([base, tip], Stroke::new((radius * 0.12).max(1.5), palette.text));
    if focused {
        painter.circle_stroke(center, radius + 1.5, Stroke::new(1.0, palette.accent));
    }
}

/// A knob for a pan: -1 (left) to 1 (right), filling from the center, resetting to the center.
pub fn pan(value: &mut f32) -> Knob<'_> {
    Knob::new(value, -1.0..=1.0).origin(0.0).default(0.0).format(format_pan)
}

/// Shows a pan as `C`, `50% L` or `100% R`.
pub fn format_pan(value: f32) -> String {
    let percent = (value * 100.0).round();
    if percent == 0.0 {
        String::from("C")
    } else if percent < 0.0 {
        format!("{}% L", -percent)
    } else {
        format!("{percent}% R")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_map_to_fractions_and_back() {
        let range = -60.0..=6.0;
        assert_eq!(normalize(-60.0, &range), 0.0);
        assert_eq!(normalize(6.0, &range), 1.0);
        assert!((normalize(-27.0, &range) - 0.5).abs() < 1e-6);
        assert_eq!(normalize(100.0, &range), 1.0, "clamped");
        assert_eq!(normalize(f32::NAN, &range), 0.0);
        assert_eq!(normalize(3.0, &(2.0..=2.0)), 0.0, "an empty range");
        for t in [0.0, 0.25, 0.5, 1.0] {
            assert!((normalize(denormalize(t, &range), &range) - t).abs() < 1e-6);
        }
    }

    #[test]
    fn drags_sweep_the_range_and_shift_is_finer() {
        assert!((drag_change(DRAG_POINTS, false) - 1.0).abs() < 1e-6);
        assert!((drag_change(DRAG_POINTS, true) - FINE).abs() < 1e-6);
        assert!((drag_change(-50.0, false) + 0.25).abs() < 1e-6);
    }

    #[test]
    fn wheel_points_make_steps() {
        assert!((wheel_steps(WHEEL_POINTS) - 1.0).abs() < 1e-6);
        assert!((wheel_steps(-2.0 * WHEEL_POINTS) + 2.0).abs() < 1e-6);
        assert!((wheel_steps(5.0) - 0.1).abs() < 1e-6, "a trackpad scrolls in small amounts");
    }

    #[test]
    fn values_format_with_their_unit() {
        assert_eq!(format_value(0.5, &(0.0..=1.0), ""), "0.50");
        assert_eq!(format_value(-3.25, &(-60.0..=6.0), "dB"), "-3.2 dB");
        assert_eq!(format_value(440.0, &(20.0..=20000.0), "Hz"), "440 Hz");
        assert_eq!(format_value(-0.001, &(-1.0..=1.0), ""), "0.00", "no negative zero");
        assert_eq!(format_pan(0.0), "C");
        assert_eq!(format_pan(-0.5), "50% L");
        assert_eq!(format_pan(1.0), "100% R");
    }

    #[test]
    fn the_arc_runs_from_lower_left_over_the_top_to_lower_right() {
        let c = Pos2::ZERO;
        let start = arc_point(c, 1.0, 0.0);
        let top = arc_point(c, 1.0, 0.5);
        let end = arc_point(c, 1.0, 1.0);
        assert!(start.x < 0.0 && start.y > 0.0, "{start:?}");
        assert!(top.x.abs() < 1e-5 && top.y < -0.99, "{top:?}");
        assert!(end.x > 0.0 && end.y > 0.0, "{end:?}");
    }
}
