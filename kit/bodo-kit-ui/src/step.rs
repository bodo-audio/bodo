//! The step sequencer row (project.md sections 10.2 and 14.4).
//!
//! A row of step buttons drawn in groups of four. Pressing a step toggles it, and dragging paints: the first press
//! decides the value (pressing a silent step paints steps on, pressing a playing step paints them off) and every step
//! the pointer then crosses takes it. Dragging with the secondary button clears steps. Rows that share a paint id
//! keep painting as the pointer moves from one row to another, as in a channel rack.
//!
//! The row never changes the caller's steps. It reports the edits of the frame in [`StepRowResponse::edits`], and
//! the caller applies them (in Bodo, as commands that can be undone).

use egui::{Color32, Id, PointerButton, Rect, Response, Sense, Ui, Vec2, WidgetInfo, WidgetType, pos2, vec2};

use crate::palette::Palette;

/// Steps per group; groups alternate between the two silent shades.
pub const GROUP: usize = 4;

/// What a paint gesture is doing, kept in egui's temporary memory under the paint id.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Paint {
    /// A press started on a step and the button is still down.
    active: bool,
    /// The value every crossed step takes.
    value: bool,
    /// The step the pointer was last over, so a step is painted once per visit.
    last: Option<Id>,
}

/// One row of step buttons.
#[must_use = "show it with `row.show(ui)`"]
pub struct StepRow<'a> {
    label: String,
    steps: &'a [bool],
    paint_id: Option<Id>,
    step_size: Vec2,
    gap: f32,
    group_gap: f32,
    palette: Option<Palette>,
}

/// What a row did this frame.
#[derive(Debug)]
pub struct StepRowResponse {
    /// The whole row.
    pub response: Response,
    /// The steps to change, as `(index, on)` pairs in the order they happened. Empty on most frames.
    pub edits: Vec<(usize, bool)>,
}

impl<'a> StepRow<'a> {
    /// A row named `label` (its steps are announced as `<label> step <n>`) showing `steps`.
    pub fn new(label: impl Into<String>, steps: &'a [bool]) -> Self {
        Self { label: label.into(), steps, paint_id: None, step_size: vec2(16.0, 22.0), gap: 2.0, group_gap: 4.0, palette: None }
    }

    /// Shares painting with every row given the same id, so a drag continues across rows. Without it, a row paints
    /// on its own.
    pub fn paint_id(mut self, id: Id) -> Self {
        self.paint_id = Some(id);
        self
    }

    /// The size of one step in points; 16 by 22 by default.
    pub fn step_size(mut self, size: Vec2) -> Self {
        self.step_size = vec2(size.x.max(4.0), size.y.max(4.0));
        self
    }

    /// Colors for this row instead of the installed [`Palette`].
    pub fn palette(mut self, palette: Palette) -> Self {
        self.palette = Some(palette);
        self
    }

    /// The left edge of step `index`, relative to the row's left edge.
    pub fn offset(&self, index: usize) -> f32 {
        index as f32 * (self.step_size.x + self.gap) + (index / GROUP) as f32 * self.group_gap
    }

    /// The size of one step.
    pub fn size(&self) -> Vec2 {
        self.step_size
    }

    /// The row's width in points.
    pub fn width(&self) -> f32 {
        match self.steps.len() {
            0 => 0.0,
            n => self.offset(n - 1) + self.step_size.x,
        }
    }

    /// Draws the row and reports its edits.
    ///
    /// Complexity: O(n) in the number of steps.
    pub fn show(self, ui: &mut Ui) -> StepRowResponse {
        let (row_rect, response) = ui.allocate_exact_size(vec2(self.width(), self.step_size.y), Sense::hover());
        let row_id = response.id;
        let paint_id = self.paint_id.unwrap_or(row_id);
        let (pressed, secondary, down) =
            ui.input(|i| (i.pointer.any_pressed(), i.pointer.button_pressed(PointerButton::Secondary), i.pointer.any_down()));
        let mut paint: Paint = ui.data(|d| d.get_temp(paint_id)).unwrap_or_default();
        let before = paint;
        let palette = self.palette.unwrap_or_else(|| Palette::get(ui.ctx()));
        let enabled = ui.is_enabled();
        let mut edits = Vec::new();

        for (index, &on) in self.steps.iter().enumerate() {
            let rect = Rect::from_min_size(pos2(row_rect.left() + self.offset(index), row_rect.top()), self.step_size);
            let id = row_id.with(index);
            let step = ui.interact(rect, id, Sense::click());
            let label = &self.label;
            step.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, enabled, on, format!("{label} step {}", index + 1)));
            if pressed && step.is_pointer_button_down_on() {
                // A new gesture: the pressed step decides what to paint.
                let value = !secondary && !on;
                paint = Paint { active: true, value, last: Some(id) };
                if on != value {
                    edits.push((index, value));
                }
            } else if step.clicked() && !step.clicked_by(PointerButton::Primary) && !step.clicked_by(PointerButton::Secondary) {
                // The keyboard or a screen reader clicked it: a plain toggle.
                edits.push((index, !on));
            } else if paint.active && down && paint.last != Some(id) && step.contains_pointer() {
                paint.last = Some(id);
                if on != paint.value {
                    edits.push((index, paint.value));
                }
            }
            if ui.is_rect_visible(rect) {
                let shown = edits.iter().rev().find(|(i, _)| *i == index).map_or(on, |&(_, value)| value);
                let hot = step.hovered() || (paint.active && step.contains_pointer());
                ui.painter().rect_filled(rect, 3.0, color(&palette, index, shown, hot));
                if step.has_focus() {
                    ui.painter().rect_stroke(rect.expand(1.0), 4.0, egui::Stroke::new(1.0, palette.accent), egui::StrokeKind::Outside);
                }
            }
        }
        if !down {
            paint = Paint::default();
        }
        if paint != before {
            ui.data_mut(|d| {
                if paint == Paint::default() {
                    d.remove::<Paint>(paint_id);
                } else {
                    d.insert_temp(paint_id, paint);
                }
            });
        }
        StepRowResponse { response, edits }
    }
}

/// The fill of a step: the on color, or the silent shade of its group, lightened under the pointer.
fn color(palette: &Palette, index: usize, on: bool, hot: bool) -> Color32 {
    match (on, hot) {
        (true, false) => palette.step_on,
        (true, true) => palette.step_on.lerp_to_gamma(Color32::WHITE, 0.2),
        (false, true) => palette.step_hover,
        (false, false) if (index / GROUP).is_multiple_of(2) => palette.step_off_a,
        (false, false) => palette.step_off_b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_alternate_shades() {
        let p = Palette::dark();
        assert_eq!(color(&p, 0, false, false), p.step_off_a);
        assert_eq!(color(&p, 3, false, false), p.step_off_a);
        assert_eq!(color(&p, 4, false, false), p.step_off_b);
        assert_eq!(color(&p, 8, false, false), p.step_off_a);
        assert_eq!(color(&p, 5, true, false), p.step_on);
        assert_eq!(color(&p, 5, false, true), p.step_hover);
    }

    #[test]
    fn steps_leave_a_gap_between_groups() {
        let steps = [false; 16];
        let row = StepRow::new("Kick", &steps).step_size(vec2(10.0, 20.0));
        assert_eq!(row.offset(1), 12.0);
        assert_eq!(row.offset(4), 4.0 * 12.0 + 4.0);
        assert_eq!(row.width(), 15.0 * 12.0 + 3.0 * 4.0 + 10.0);
        assert_eq!(StepRow::new("Empty", &[]).width(), 0.0);
    }
}
