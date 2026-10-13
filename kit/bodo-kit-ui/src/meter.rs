//! The peak meter (project.md sections 9.7 and 14.4): vertical bars, mono or stereo, with a clip LED on top.
//!
//! The scale is linear in dB from [`MIN_DB`] at the bottom to [`MAX_DB`] at the top, and the bars change color in
//! zones: up to -18 dBFS, -18 to -6 dBFS, and above -6 dBFS (project.md section 14.2). Meters move with audio
//! ballistics, which belong to whoever measures the levels; this widget only draws the values it is given.

use egui::{Rect, Response, Sense, Stroke, Ui, Vec2, Widget, WidgetInfo, WidgetType, pos2, vec2};

use crate::fader::gain_to_db;
use crate::palette::Palette;

/// The bottom of the scale, in dBFS.
pub const MIN_DB: f32 = -60.0;
/// The top of the scale, in dBFS.
pub const MAX_DB: f32 = 6.0;
/// Where the mid zone starts, in dBFS.
pub const MID_DB: f32 = -18.0;
/// Where the high zone starts, in dBFS.
pub const HIGH_DB: f32 = -6.0;

/// The height of the clip LED in points.
const LED: f32 = 4.0;

/// Where `db` sits on the meter: 0 at the bottom ([`MIN_DB`] and below), 1 at the top ([`MAX_DB`]).
pub fn db_to_level(db: f32) -> f32 {
    if db.is_nan() { 0.0 } else { ((db - MIN_DB) / (MAX_DB - MIN_DB)).clamp(0.0, 1.0) }
}

/// A peak meter for one or two channels.
#[must_use = "add it with `ui.add(meter)`"]
pub struct Meter {
    peaks: [f32; 2],
    channels: usize,
    clip: Option<bool>,
    label: String,
    size: Option<Vec2>,
    palette: Option<Palette>,
}

impl Meter {
    /// A mono meter showing `peak`, a linear sample peak (1.0 is 0 dBFS).
    pub fn mono(peak: f32) -> Self {
        Self { peaks: [peak, peak], channels: 1, clip: None, label: String::new(), size: None, palette: None }
    }

    /// A stereo meter showing the linear peaks of the left and right channels.
    pub fn stereo(left: f32, right: f32) -> Self {
        Self { peaks: [left, right], channels: 2, clip: None, label: String::new(), size: None, palette: None }
    }

    /// Lights the clip LED or keeps it dark. Without this, it lights while a peak reaches 0 dBFS.
    pub fn clip(mut self, clip: bool) -> Self {
        self.clip = Some(clip);
        self
    }

    /// What the meter measures, for screen readers.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// The meter's size in points.
    pub fn size(mut self, size: Vec2) -> Self {
        self.size = Some(vec2(size.x.max(2.0), size.y.max(LED + 8.0)));
        self
    }

    /// Colors for this meter instead of the installed [`Palette`].
    pub fn palette(mut self, palette: Palette) -> Self {
        self.palette = Some(palette);
        self
    }
}

impl Widget for Meter {
    fn ui(self, ui: &mut Ui) -> Response {
        let em = crate::em(ui);
        let width = if self.channels == 2 { 0.8 * em } else { 0.48 * em };
        let size = self.size.unwrap_or_else(|| vec2(width, 9.6 * em));
        let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
        let peaks = &self.peaks[..self.channels];
        let loudest = peaks.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        let enabled = ui.is_enabled();
        let label = self.label.clone();
        response.widget_info(|| {
            let mut info = WidgetInfo::labeled(WidgetType::ProgressIndicator, enabled, &label);
            let db = f64::from(gain_to_db(loudest)).max(f64::from(MIN_DB));
            info.value = Some(db);
            // Screen readers hear the peak level in dB.
            info.current_text_value = Some(if db <= f64::from(MIN_DB) { String::from("-inf dB") } else { format!("{db:.1} dB") });
            info
        });
        if !ui.is_rect_visible(rect) {
            return response;
        }
        let palette = self.palette.unwrap_or_else(|| Palette::get(ui.ctx()));
        let painter = ui.painter();
        let clip = self.clip.unwrap_or(loudest >= 1.0);
        let led = Rect::from_min_size(rect.min, vec2(rect.width(), LED));
        painter.rect_filled(led, 1.0, if clip { palette.clip } else { palette.track });
        let bars = Rect::from_min_max(pos2(rect.left(), rect.top() + LED + 2.0), rect.max);
        let gap = if self.channels == 2 { 1.0 } else { 0.0 };
        let width = (bars.width() - gap) / self.channels as f32;
        let y_of = |db: f32| bars.bottom() - db_to_level(db) * bars.height();
        for (index, &peak) in peaks.iter().enumerate() {
            let left = bars.left() + index as f32 * (width + gap);
            let column = Rect::from_min_max(pos2(left, bars.top()), pos2(left + width, bars.bottom()));
            painter.rect_filled(column, 1.0, palette.track);
            let top = y_of(gain_to_db(peak.abs()));
            for (from, to, color) in
                [(MIN_DB, MID_DB, palette.meter_low), (MID_DB, HIGH_DB, palette.meter_mid), (HIGH_DB, MAX_DB, palette.meter_high)]
            {
                let (zone_bottom, zone_top) = (y_of(from), y_of(to).max(top));
                if zone_top < zone_bottom {
                    painter.rect_filled(Rect::from_min_max(pos2(column.left(), zone_top), pos2(column.right(), zone_bottom)), 0.0, color);
                }
            }
        }
        // A faint line marks 0 dBFS.
        let zero = y_of(0.0);
        painter.line_segment([pos2(bars.left(), zero), pos2(bars.right(), zero)], Stroke::new(1.0, palette.outline));
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_are_linear_in_db() {
        assert_eq!(db_to_level(MIN_DB), 0.0);
        assert_eq!(db_to_level(-100.0), 0.0);
        assert_eq!(db_to_level(f32::NEG_INFINITY), 0.0);
        assert_eq!(db_to_level(MAX_DB), 1.0);
        assert!((db_to_level(-27.0) - 0.5).abs() < 1e-6);
        assert!(db_to_level(MID_DB) < db_to_level(HIGH_DB));
    }
}
