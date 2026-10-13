//! The LED toggle: a small light that is on or off, such as a channel's mute LED (lit while the channel plays).

use egui::{Color32, Response, Sense, Stroke, Ui, Vec2, Widget, WidgetInfo, WidgetType};

use crate::palette::Palette;

/// A small on/off light that toggles a `bool` when clicked.
#[must_use = "add it with `ui.add(led)`"]
pub struct Led<'a> {
    on: &'a mut bool,
    label: String,
    size: Option<f32>,
    palette: Option<Palette>,
}

impl<'a> Led<'a> {
    /// An LED showing and toggling `on`.
    pub fn new(on: &'a mut bool) -> Self {
        Self { on, label: String::new(), size: None, palette: None }
    }

    /// What the LED switches, for the tooltip and screen readers.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// The clickable size in points; the light is a little smaller. By default 1.12 times [`crate::em`], 14 points
    /// beside 12.5-point text.
    pub fn size(mut self, size: f32) -> Self {
        self.size = Some(size.max(6.0));
        self
    }

    /// Colors for this LED instead of the installed [`Palette`].
    pub fn palette(mut self, palette: Palette) -> Self {
        self.palette = Some(palette);
        self
    }
}

impl Widget for Led<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let side = self.size.unwrap_or_else(|| 1.12 * crate::em(ui));
        let (rect, mut response) = ui.allocate_exact_size(Vec2::splat(side), Sense::click());
        if response.clicked() {
            *self.on = !*self.on;
            response.mark_changed();
        }
        let on = *self.on;
        let enabled = ui.is_enabled();
        let label = self.label.clone();
        response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, enabled, on, &label));
        if ui.is_rect_visible(rect) {
            let palette = self.palette.unwrap_or_else(|| Palette::get(ui.ctx()));
            let painter = ui.painter();
            let center = rect.center();
            let radius = side * 0.32;
            if on {
                painter.circle_filled(center, radius * 1.6, palette.led_on.gamma_multiply(0.18));
            }
            let fill = if on { palette.led_on } else { palette.led_off };
            let fill = if response.hovered() { fill.lerp_to_gamma(Color32::WHITE, 0.15) } else { fill };
            painter.circle(center, radius, fill, Stroke::new(1.0, palette.outline));
            if response.has_focus() {
                painter.circle_stroke(center, radius + 2.0, Stroke::new(1.0, palette.accent));
            }
        }
        if self.label.is_empty() { response } else { response.on_hover_text(self.label) }
    }
}
