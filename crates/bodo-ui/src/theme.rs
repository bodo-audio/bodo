//! Design tokens and egui styling (project.md sections 14.2, 14.3 and 14.7).
//!
//! Panel code never uses literal colors: it reads [`Tokens::get`], the tokens of the look in use. Dark is the default;
//! Light, High contrast and System (follow the operating system) are supported. The values come from pixel3d's
//! tokens, extended for a DAW.
//!
//! The color-blind-safe option swaps the meters' green to red gradient for one that never relies on telling red from
//! green, and draws clip, pattern and channel colors from a palette built on Okabe and Ito's colors, which stay apart
//! for the common color-vision deficiencies. It changes only how colors are shown: the project keeps its own.

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
    /// White text on black with strong outlines: every text pair reads at 7:1 or better.
    HighContrast,
}

impl ThemePref {
    /// Every choice, in menu order.
    pub const ALL: [Self; 4] = [Self::Dark, Self::Light, Self::HighContrast, Self::System];

    /// Its name in `ui.set_theme` and the English source of its name in menus.
    pub fn label(self) -> &'static str {
        match self {
            Self::System => crate::i18n::en("System"),
            Self::Light => crate::i18n::en("Light"),
            Self::Dark => crate::i18n::en("Dark"),
            Self::HighContrast => crate::i18n::en("High contrast"),
        }
    }

    /// The look to show, given what the system reports.
    pub fn resolve(self, system: Option<Theme>) -> Look {
        match self {
            Self::System => match system {
                Some(Theme::Light) => Look::Light,
                _ => Look::Dark,
            },
            Self::Light => Look::Light,
            Self::Dark => Look::Dark,
            Self::HighContrast => Look::HighContrast,
        }
    }
}

/// The look being shown: the theme once System is resolved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Look {
    /// The dark theme.
    #[default]
    Dark,
    /// The light theme.
    Light,
    /// The high-contrast theme, a dark one.
    HighContrast,
}

impl Look {
    /// egui's theme it builds on: high contrast is a dark theme.
    pub fn egui(self) -> Theme {
        match self {
            Self::Light => Theme::Light,
            Self::Dark | Self::HighContrast => Theme::Dark,
        }
    }
}

impl From<Theme> for Look {
    fn from(theme: Theme) -> Self {
        match theme {
            Theme::Dark => Self::Dark,
            Theme::Light => Self::Light,
        }
    }
}

thread_local! {
    static ACTIVE: Cell<Look> = const { Cell::new(Look::Dark) };
    static COLOR_SAFE: Cell<bool> = const { Cell::new(false) };
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

    /// The high-contrast theme: white and near-white text on black, bright outlines and a yellow accent, so every
    /// text pair reads at 7:1 or better and selections never rely on a faint tint.
    pub const HIGH_CONTRAST: Self = Self {
        chrome: hex(0x000000),
        chrome_dark: hex(0x000000),
        panel: hex(0x000000),
        control: hex(0x1f1f1f),
        control_hover: hex(0x383838),
        border: hex(0x9e9e9e),
        field: hex(0x000000),
        popup: hex(0x0a0a0a),
        text: hex(0xffffff),
        text_dim: hex(0xe6e6e6),
        text_faint: hex(0xcccccc),
        accent: hex(0xffd60a),
        accent_hover: hex(0xffe766),
        accent_soft: hex(0x423800),
        on_accent: hex(0x000000),
        select_bg: hex(0x3a3000),
        ok: hex(0x5cff8a),
        warn: hex(0xffb340),
        error: hex(0xff7a7a),
        record: hex(0xff6161),
        playhead: hex(0xffd60a),
        grid_bar: hex(0x8f8f8f),
        grid_beat: hex(0x525252),
        grid_step: hex(0x2e2e2e),
        marquee: hex(0x33e6ff),
        key_black: hex(0x1f1f1f),
        key_white: hex(0xf2f2f2),
        row_black: hex(0x101010),
        shadow: Color32::from_black_alpha(160),
    };

    /// The tokens of the look in use on this thread.
    pub fn get() -> Self {
        Self::of_look(ACTIVE.with(Cell::get))
    }

    /// The tokens of egui's `theme`, dark or light.
    pub fn of(theme: Theme) -> Self {
        Self::of_look(theme.into())
    }

    /// The tokens of `look`.
    pub fn of_look(look: Look) -> Self {
        match look {
            Look::Dark => Self::DARK,
            Look::Light => Self::LIGHT,
            Look::HighContrast => Self::HIGH_CONTRAST,
        }
    }
}

/// The look in use on this thread.
pub fn look() -> Look {
    ACTIVE.with(Cell::get)
}

/// The base size of the interface, in points: body text. Every other size is a multiple of it, as CSS sizes are of
/// `rem`, so the whole layout keeps its proportions when it changes. The interface size preference then scales the
/// lot (project.md section 14.7).
pub const REM: f32 = 14.0;

/// `n` times the base size [`REM`], in whole points so edges stay on pixels: `rem(1.0)` is body text, `rem(2.0)`
/// twice that. Sizes under a point (hairlines) are left unrounded.
pub const fn rem(n: f32) -> f32 {
    let points = n * REM;
    if points.abs() < 1.0 { points } else { points.round() }
}

/// Font sizes and the heights of the interface's rows (project.md section 14.3), all in [`rem`].
pub mod size {
    use super::rem;

    /// Small labels.
    pub const SMALL: f32 = rem(0.9);
    /// Body text.
    pub const BODY: f32 = rem(1.0);
    /// Numbers.
    pub const MONO: f32 = rem(0.96);
    /// Headings.
    pub const HEADING: f32 = rem(1.2);
    /// The song position in the top bar.
    pub const TIME: f32 = rem(1.6);
    /// A row of the top bar.
    pub const TOP_ROW: f32 = rem(2.56);
    /// A window's title bar.
    pub const TITLE_BAR: f32 = rem(2.08);
    /// A list row.
    pub const ROW: f32 = rem(1.76);
}

/// Makes `look` the one [`Tokens::get`] returns and styles `ctx` with it, with the color-blind-safe colors when
/// `color_safe` is set.
pub fn apply(ctx: &egui::Context, look: impl Into<Look>, color_safe: bool) {
    let look = look.into();
    let theme = look.egui();
    ACTIVE.with(|active| active.set(look));
    COLOR_SAFE.with(|safe| safe.set(color_safe));
    let tk = Tokens::of_look(look);
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
    // egui draws a widget with keyboard focus in its active style: an accent outline makes focus always visible.
    visuals.widgets.active.bg_stroke = Stroke::new(1.5, tk.accent);
    if look == Look::HighContrast {
        // Every control shows its edge, and focus and hovering are thick and bright.
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, tk.text_faint);
        visuals.widgets.hovered.bg_stroke = Stroke::new(2.0, tk.accent);
        visuals.widgets.active.bg_stroke = Stroke::new(2.0, tk.accent);
        visuals.widgets.open.bg_stroke = Stroke::new(2.0, tk.text);
        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, tk.text);
        visuals.selection.stroke = Stroke::new(2.0, tk.accent);
    }
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
        style.spacing.item_spacing = egui::vec2(rem(0.48), rem(0.32));
        style.spacing.button_padding = egui::vec2(rem(0.48), rem(0.24));
        style.spacing.interact_size = egui::vec2(rem(3.2), rem(1.6));
        style.spacing.icon_width = rem(1.1);
        style.spacing.icon_width_inner = rem(0.64);
        style.spacing.icon_spacing = rem(0.32);
        style.spacing.slider_width = rem(8.0);
        style.spacing.combo_width = rem(8.0);
        style.spacing.text_edit_width = rem(22.0);
        style.spacing.menu_margin = egui::Margin::same(rem(0.48) as i8);
        style.spacing.window_margin = egui::Margin::same(rem(0.64) as i8);
        style.animation_time = 0.12;
    });
    palette_for(look, color_safe).install(ctx);
}

/// The Kit widget palette that matches `look`, with the color-blind-safe meter gradient when `color_safe` is set.
pub fn palette_for(look: impl Into<Look>, color_safe: bool) -> Palette {
    let look = look.into();
    let tk = Tokens::of_look(look);
    let base = match look {
        Look::Light => Palette::light(),
        Look::Dark => Palette::dark(),
        Look::HighContrast => Palette {
            track: hex(0x333333),
            fill: tk.accent,
            body: hex(0x1f1f1f),
            body_hover: hex(0x383838),
            outline: tk.text_faint,
            led_off: hex(0x333333),
            step_off_a: hex(0x262626),
            step_off_b: hex(0x404040),
            step_hover: hex(0x5c5c5c),
            ..Palette::dark()
        },
    };
    let palette = Palette { accent: tk.accent, text: tk.text, text_dim: tk.text_dim, ..base };
    if !color_safe {
        return palette;
    }
    // Blue to yellow to vermillion: apart in lightness as well as hue, so no step depends on telling red from green.
    let (low, mid, high) = match look {
        Look::Light => (hex(0x0072b2), hex(0xe6c200), hex(0xd55e00)),
        Look::Dark | Look::HighContrast => (hex(0x56b4e9), hex(0xf0e442), hex(0xe86a1a)),
    };
    Palette { meter_low: low, meter_mid: mid, meter_high: high, clip: high, led_on: hex(0x56b4e9), ..palette }
}

/// Whether the color-blind-safe colors are on for this thread.
pub fn color_safe() -> bool {
    COLOR_SAFE.with(Cell::get)
}

/// The colors clips, patterns and channels take with the color-blind-safe option, one for each of the sixteen
/// standard hues in order: Okabe and Ito's colors and lighter shades of them, laid out so neighbors on the color
/// wheel stay apart in lightness.
pub const SAFE_HUES: [Color32; 16] = [
    hex(0xd55e00),
    hex(0xe69f00),
    hex(0xf2c77a),
    hex(0xf0e442),
    hex(0xf7ef9a),
    hex(0x009e73),
    hex(0x5fc9aa),
    hex(0xa0d6f3),
    hex(0x56b4e9),
    hex(0x5ba3d6),
    hex(0x0072b2),
    hex(0xc4c4c4),
    hex(0xe3aecb),
    hex(0xcc79a7),
    hex(0xea9a66),
    hex(0x999999),
];

/// `color` at `alpha` (0 to 1), for overlays such as muted items.
pub fn faded(color: Color32, alpha: f32) -> Color32 {
    color.gamma_multiply(alpha.clamp(0.0, 1.0))
}

/// A model color as an egui color: as stored, or with the color-blind-safe option, the safe color standing for the
/// nearest of the standard hues.
///
/// Complexity: O(1); sixteen comparisons with the option on.
pub fn color(c: bodo_model::Color) -> Color32 {
    if color_safe() { safe_color(c) } else { Color32::from_rgb(c.r, c.g, c.b) }
}

/// The color-blind-safe stand-in for `c`: the safe color of the standard hue nearest to it. Grays stay as they are.
pub fn safe_color(c: bodo_model::Color) -> Color32 {
    let (max, min) = (c.r.max(c.g).max(c.b), c.r.min(c.g).min(c.b));
    if max - min < 24 {
        return Color32::from_rgb(c.r, c.g, c.b);
    }
    let distance = |h: &bodo_model::Color| {
        let d = |a: u8, b: u8| (i32::from(a) - i32::from(b)).pow(2);
        d(h.r, c.r) + d(h.g, c.g) + d(h.b, c.b)
    };
    let nearest = bodo_engine::palette::HUES.iter().enumerate().min_by_key(|(_, h)| distance(h)).map_or(0, |(i, _)| i);
    SAFE_HUES[nearest % SAFE_HUES.len()]
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
    fn text_meets_wcag_contrast_in_every_theme() {
        for tk in [Tokens::DARK, Tokens::LIGHT, Tokens::HIGH_CONTRAST] {
            for surface in [tk.panel, tk.chrome, tk.popup] {
                assert!(contrast(tk.text, surface) >= 7.0, "body text on {surface:?}: {:.2}", contrast(tk.text, surface));
                assert!(contrast(tk.text_dim, surface) >= 4.5, "dim text on {surface:?}: {:.2}", contrast(tk.text_dim, surface));
            }
            assert!(contrast(tk.on_accent, tk.accent) >= 4.5, "text on the accent: {:.2}", contrast(tk.on_accent, tk.accent));
        }
    }

    #[test]
    fn high_contrast_text_reads_at_seven_to_one_everywhere() {
        let tk = Tokens::HIGH_CONTRAST;
        let surfaces = [tk.panel, tk.chrome, tk.chrome_dark, tk.popup, tk.field, tk.control, tk.control_hover, tk.select_bg, tk.accent_soft];
        for surface in surfaces {
            for (name, text) in [("body", tk.text), ("dim", tk.text_dim), ("faint", tk.text_faint)] {
                assert!(contrast(text, surface) >= 7.0, "{name} text on {surface:?}: {:.2}", contrast(text, surface));
            }
        }
        assert!(contrast(tk.on_accent, tk.accent) >= 7.0, "text on the accent: {:.2}", contrast(tk.on_accent, tk.accent));
        for signal in [tk.accent, tk.ok, tk.warn, tk.error, tk.record, tk.marquee, tk.playhead] {
            assert!(contrast(signal, tk.panel) >= 7.0, "{signal:?} on the panel: {:.2}", contrast(signal, tk.panel));
        }
    }

    #[test]
    fn color_safe_hues_are_all_different_and_stand_for_the_standard_ones() {
        for (i, a) in SAFE_HUES.iter().enumerate() {
            for b in &SAFE_HUES[i + 1..] {
                assert_ne!(a, b);
            }
        }
        for (i, hue) in bodo_engine::palette::HUES.iter().enumerate() {
            assert_eq!(safe_color(*hue), SAFE_HUES[i]);
        }
        let gray = bodo_model::Color::rgb(0x80, 0x80, 0x80);
        assert_eq!(safe_color(gray), Color32::from_gray(0x80), "grays stay gray");
    }

    #[test]
    fn the_color_safe_meter_reads_by_lightness_not_red_and_green() {
        for look in [Look::Dark, Look::Light, Look::HighContrast] {
            let p = palette_for(look, true);
            assert!((luminance(p.meter_low) - luminance(p.meter_mid)).abs() > 0.1, "{look:?}");
            assert!((luminance(p.meter_mid) - luminance(p.meter_high)).abs() > 0.1, "{look:?}");
            assert!(p.meter_low.g() < p.meter_low.b(), "{look:?}: the low end is blue, not green");
        }
    }
}
