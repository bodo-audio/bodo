//! Design tokens and egui styling (project.md sections 14.2 and 14.3).
//!
//! Panel code never uses literal colors: it reads [`Tokens::get`], the tokens of the theme in use. Dark is the
//! default; Light and System (follow the operating system) are supported. The values come from pixel3d's tokens,
//! extended for a DAW.

use std::cell::Cell;

use bodo_kit_ui::Palette;
use craft_ui::theme::WidgetPalette;
use egui::{Color32, CornerRadius, FontFamily, FontId, Shadow, Stroke, TextStyle, Theme, Visuals};

/// The theme the user chose.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThemePref {
    /// Follow the operating system's light or dark appearance.
    System,
    /// Always light.
    Light,
    /// Always dark.
    #[default]
    Dark,
}

impl ThemePref {
    /// Every choice, in menu order.
    pub const ALL: [Self; 3] = [Self::Dark, Self::Light, Self::System];

    /// Its name in menus.
    pub fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    /// The theme to show, given what the system reports.
    pub fn resolve(self, system: Option<Theme>) -> Theme {
        match self {
            Self::System => system.unwrap_or(Theme::Dark),
            Self::Light => Theme::Light,
            Self::Dark => Theme::Dark,
        }
    }
}

thread_local! {
    static ACTIVE: Cell<Theme> = const { Cell::new(Theme::Dark) };
}

/// Every color the interface uses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    /// The top bar and window title bars.
    pub chrome: Color32,
    /// Darker chrome: the menu row, inactive tabs.
    pub chrome_dark: Color32,
    /// Panel and window bodies.
    pub panel: Color32,
    /// Buttons and other controls.
    pub control: Color32,
    /// Controls under the pointer.
    pub control_hover: Color32,
    /// Borders and the background between windows.
    pub border: Color32,
    /// Text fields and other inset backgrounds.
    pub field: Color32,
    /// Menus and tooltips.
    pub popup: Color32,
    /// Body text.
    pub text: Color32,
    /// Secondary text.
    pub text_dim: Color32,
    /// Hints and disabled text.
    pub text_faint: Color32,
    /// The accent: selection, the playhead, active buttons.
    pub accent: Color32,
    /// The accent under the pointer.
    pub accent_hover: Color32,
    /// The accent mixed into the background: selected rows and buttons.
    pub accent_soft: Color32,
    /// Text drawn on an accent fill.
    pub on_accent: Color32,
    /// Selected text and list rows.
    pub select_bg: Color32,
    /// Success.
    pub ok: Color32,
    /// Warnings.
    pub warn: Color32,
    /// Errors.
    pub error: Color32,
    /// Clip indicators, record arm and the record button.
    pub record: Color32,
    /// The playhead line and its triangle on the ruler.
    pub playhead: Color32,
    /// Bar lines in the playlist and piano roll.
    pub grid_bar: Color32,
    /// Beat lines.
    pub grid_beat: Color32,
    /// Step lines.
    pub grid_step: Color32,
    /// Selection rectangles.
    pub marquee: Color32,
    /// Black keys of the piano roll keyboard.
    pub key_black: Color32,
    /// White keys of the piano roll keyboard.
    pub key_white: Color32,
    /// Rows of black keys in the piano roll grid.
    pub row_black: Color32,
    /// The shadow under windows and menus.
    pub shadow: Color32,
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

impl Tokens {
    /// The dark theme (project.md section 14.2).
    pub const DARK: Self = Self {
        chrome: hex(0x171717),
        chrome_dark: hex(0x0f0f0f),
        panel: hex(0x141414),
        control: hex(0x262626),
        control_hover: hex(0x333333),
        border: hex(0x0a0a0a),
        field: hex(0x111111),
        popup: hex(0x1f1f1f),
        text: hex(0xe5e5e5),
        text_dim: hex(0xa3a3a3),
        text_faint: hex(0x737373),
        accent: hex(0xfacc15),
        accent_hover: hex(0xfde047),
        accent_soft: hex(0x3a3110),
        on_accent: hex(0x171717),
        select_bg: hex(0x5c4a0b),
        ok: hex(0x4ade80),
        warn: hex(0xfbbf24),
        error: hex(0xf87171),
        record: hex(0xef4444),
        playhead: hex(0xfacc15),
        grid_bar: hex(0x3a3a3a),
        grid_beat: hex(0x262626),
        grid_step: hex(0x1c1c1c),
        marquee: hex(0xfacc15),
        key_black: hex(0x1a1a1a),
        key_white: hex(0xd4d4d4),
        row_black: hex(0x111111),
        shadow: Color32::from_black_alpha(120),
    };

    /// The light theme.
    pub const LIGHT: Self = Self {
        chrome: hex(0xf5f5f5),
        chrome_dark: hex(0xe5e5e5),
        panel: hex(0xfafafa),
        control: hex(0xe5e5e5),
        control_hover: hex(0xd4d4d4),
        border: hex(0xd4d4d4),
        field: hex(0xffffff),
        popup: hex(0xffffff),
        text: hex(0x171717),
        text_dim: hex(0x404040),
        text_faint: hex(0x737373),
        accent: hex(0xca8a04),
        accent_hover: hex(0xa16207),
        accent_soft: hex(0xfef3c7),
        on_accent: hex(0x171717),
        select_bg: hex(0xfde68a),
        ok: hex(0x16a34a),
        warn: hex(0xd97706),
        error: hex(0xdc2626),
        record: hex(0xdc2626),
        playhead: hex(0xca8a04),
        grid_bar: hex(0xa3a3a3),
        grid_beat: hex(0xd4d4d4),
        grid_step: hex(0xe5e5e5),
        marquee: hex(0xca8a04),
        key_black: hex(0x262626),
        key_white: hex(0xfafafa),
        row_black: hex(0xeeeeee),
        shadow: Color32::from_black_alpha(40),
    };

    /// The tokens of the theme in use on this thread.
    pub fn get() -> Self {
        Self::of(ACTIVE.with(Cell::get))
    }

    /// The tokens of `theme`.
    pub fn of(theme: Theme) -> Self {
        match theme {
            Theme::Dark => Self::DARK,
            Theme::Light => Self::LIGHT,
        }
    }
}

/// Font sizes (project.md section 14.3).
pub mod size {
    /// Small labels.
    pub const SMALL: f32 = 11.5;
    /// Body text.
    pub const BODY: f32 = 12.5;
    /// Numbers.
    pub const MONO: f32 = 12.0;
    /// Headings.
    pub const HEADING: f32 = 15.0;
    /// The song position in the top bar.
    pub const TIME: f32 = 20.0;
    /// A row of the top bar.
    pub const TOP_ROW: f32 = 32.0;
    /// A window's title bar.
    pub const TITLE_BAR: f32 = 26.0;
    /// A list row.
    pub const ROW: f32 = 22.0;
}

/// Makes `theme` the one [`Tokens::get`] returns and styles `ctx` with it.
pub fn apply(ctx: &egui::Context, theme: Theme) {
    ACTIVE.with(|active| active.set(theme));
    let tk = Tokens::of(theme);
    let mut visuals = match theme {
        Theme::Dark => Visuals::dark(),
        Theme::Light => Visuals::light(),
    };
    visuals.panel_fill = tk.panel;
    visuals.window_fill = tk.popup;
    visuals.extreme_bg_color = tk.field;
    visuals.faint_bg_color = tk.chrome;
    visuals.code_bg_color = tk.field;
    visuals.window_stroke = Stroke::new(1.0, tk.border);
    visuals.window_corner_radius = CornerRadius::same(8);
    visuals.menu_corner_radius = CornerRadius::same(8);
    visuals.window_shadow = Shadow { offset: [0, 6], blur: 18, spread: 0, color: tk.shadow };
    visuals.popup_shadow = Shadow { offset: [0, 6], blur: 18, spread: 0, color: tk.shadow };
    visuals.selection.bg_fill = tk.select_bg;
    visuals.selection.stroke = Stroke::new(1.0, tk.accent);
    visuals.hyperlink_color = tk.accent;
    visuals.warn_fg_color = tk.warn;
    visuals.error_fg_color = tk.error;
    visuals.override_text_color = None;
    let palette =
        WidgetPalette { noninteractive: tk.panel, inactive: tk.control, hovered: tk.control_hover, active: tk.accent_soft, open: tk.control_hover };
    palette.apply_to(&mut visuals.widgets, CornerRadius::same(6), Stroke::new(1.0, tk.text));
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, tk.text_dim);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, tk.border);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, tk.accent);
    ctx.set_visuals_of(theme, visuals);
    ctx.set_theme(theme);
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Small, FontId::new(size::SMALL, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(size::BODY, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(size::BODY, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(size::MONO, FontFamily::Monospace)),
            (TextStyle::Heading, FontId::new(size::HEADING, FontFamily::Proportional)),
        ]
        .into();
        style.spacing.item_spacing = egui::vec2(6.0, 4.0);
        style.spacing.button_padding = egui::vec2(6.0, 3.0);
        style.spacing.interact_size.y = 20.0;
        style.animation_time = 0.12;
    });
    palette_for(theme).install(ctx);
}

/// The Kit widget palette that matches `theme`.
pub fn palette_for(theme: Theme) -> Palette {
    let tk = Tokens::of(theme);
    let base = match theme {
        Theme::Dark => Palette::dark(),
        Theme::Light => Palette::light(),
    };
    Palette { accent: tk.accent, text: tk.text, text_dim: tk.text_dim, ..base }
}

/// `color` at `alpha` (0 to 1), for overlays such as muted items.
pub fn faded(color: Color32, alpha: f32) -> Color32 {
    color.gamma_multiply(alpha.clamp(0.0, 1.0))
}

/// A model color as an egui color.
pub fn color(c: bodo_model::Color) -> Color32 {
    Color32::from_rgb(c.r, c.g, c.b)
}

/// `color` mixed over the panel at `amount` (0 to 1): the body of a clip or a channel button.
pub fn tint(color: Color32, amount: f32) -> Color32 {
    let base = Tokens::get().panel;
    let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * amount.clamp(0.0, 1.0)).round() as u8;
    Color32::from_rgb(mix(base.r(), color.r()), mix(base.g(), color.g()), mix(base.b(), color.b()))
}

/// Black or white, whichever reads better on `fill`.
pub fn text_on(fill: Color32) -> Color32 {
    let luma = 0.2126 * f32::from(fill.r()) + 0.7152 * f32::from(fill.g()) + 0.0722 * f32::from(fill.b());
    if luma > 140.0 { hex(0x171717) } else { hex(0xf5f5f5) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG relative luminance.
    fn luminance(c: Color32) -> f64 {
        let channel = |v: u8| {
            let v = f64::from(v) / 255.0;
            if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
    }

    fn contrast(a: Color32, b: Color32) -> f64 {
        let (x, y) = (luminance(a), luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn text_meets_wcag_contrast_in_both_themes() {
        for tk in [Tokens::DARK, Tokens::LIGHT] {
            for surface in [tk.panel, tk.chrome, tk.popup] {
                assert!(contrast(tk.text, surface) >= 7.0, "body text on {surface:?}: {:.2}", contrast(tk.text, surface));
                assert!(contrast(tk.text_dim, surface) >= 4.5, "dim text on {surface:?}: {:.2}", contrast(tk.text_dim, surface));
            }
            assert!(contrast(tk.on_accent, tk.accent) >= 4.5, "text on the accent: {:.2}", contrast(tk.on_accent, tk.accent));
        }
    }
}
