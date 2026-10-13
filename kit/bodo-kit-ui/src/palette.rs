//! The colors the widgets draw with.
//!
//! Bodo Kit knows nothing about an application's design tokens. A host or plugin editor fills a [`Palette`] from its
//! own theme and installs it with [`Palette::install`]; every widget then reads it with [`Palette::get`], unless a
//! widget is given a palette of its own. Without an installed palette, widgets use [`Palette::dark`] or
//! [`Palette::light`] to match egui's visuals.

use egui::{Color32, Context, Id};

/// Every color the widgets use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    /// The unfilled part of a knob's arc, a fader's groove and a meter's background.
    pub track: Color32,
    /// The value part of a knob's arc and a fader's groove.
    pub fill: Color32,
    /// Knob caps and fader handles.
    pub body: Color32,
    /// Knob caps and fader handles under the pointer.
    pub body_hover: Color32,
    /// Pointers, keyboard focus and other highlights.
    pub accent: Color32,
    /// Values and labels.
    pub text: Color32,
    /// Secondary labels and tick marks.
    pub text_dim: Color32,
    /// Edges of bodies and buttons.
    pub outline: Color32,
    /// A lit LED.
    pub led_on: Color32,
    /// An unlit LED.
    pub led_off: Color32,
    /// A step that plays.
    pub step_on: Color32,
    /// A silent step in the first, third and every odd group of four.
    pub step_off_a: Color32,
    /// A silent step in the second, fourth and every even group of four.
    pub step_off_b: Color32,
    /// A silent step under the pointer.
    pub step_hover: Color32,
    /// Meter levels up to -18 dBFS.
    pub meter_low: Color32,
    /// Meter levels from -18 to -6 dBFS.
    pub meter_mid: Color32,
    /// Meter levels above -6 dBFS.
    pub meter_high: Color32,
    /// The clip LED of a meter.
    pub clip: Color32,
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

impl Palette {
    /// The dark palette, from the dark design tokens of project.md section 14.2.
    pub const fn dark() -> Self {
        Self {
            track: hex(0x262626),
            fill: hex(0xfacc15),
            body: hex(0x333333),
            body_hover: hex(0x3d3d3d),
            accent: hex(0xfacc15),
            text: hex(0xe5e5e5),
            text_dim: hex(0xa3a3a3),
            outline: hex(0x0a0a0a),
            led_on: hex(0x4ade80),
            led_off: hex(0x2e2e2e),
            step_on: hex(0xfacc15),
            step_off_a: hex(0x2e2e2e),
            step_off_b: hex(0x242424),
            step_hover: hex(0x3d3d3d),
            meter_low: hex(0x4ade80),
            meter_mid: hex(0xfacc15),
            meter_high: hex(0xf87171),
            clip: hex(0xef4444),
        }
    }

    /// The light palette, from the light design tokens of project.md section 14.2. Accent-colored lines use the
    /// darker yellow the tokens give for contrast on light surfaces.
    pub const fn light() -> Self {
        Self {
            track: hex(0xd4d4d4),
            fill: hex(0xca8a04),
            body: hex(0xffffff),
            body_hover: hex(0xf5f5f5),
            accent: hex(0xca8a04),
            text: hex(0x171717),
            text_dim: hex(0x525252),
            outline: hex(0xa3a3a3),
            led_on: hex(0x16a34a),
            led_off: hex(0xd4d4d4),
            step_on: hex(0xca8a04),
            step_off_a: hex(0xd4d4d4),
            step_off_b: hex(0xe5e5e5),
            step_hover: hex(0xc4c4c4),
            meter_low: hex(0x16a34a),
            meter_mid: hex(0xca8a04),
            meter_high: hex(0xdc2626),
            clip: hex(0xdc2626),
        }
    }

    /// The palette widgets in `ctx` draw with: the installed one, or the default that matches egui's visuals.
    pub fn get(ctx: &Context) -> Self {
        ctx.data(|data| data.get_temp::<Self>(key()))
            .unwrap_or_else(|| if ctx.global_style().visuals.dark_mode { Self::dark() } else { Self::light() })
    }

    /// Makes this the palette of every widget in `ctx`, until another is installed or [`Palette::uninstall`] runs.
    pub fn install(self, ctx: &Context) {
        ctx.data_mut(|data| data.insert_temp(key(), self));
    }

    /// Goes back to the default palettes that follow egui's visuals.
    pub fn uninstall(ctx: &Context) {
        ctx.data_mut(|data| data.remove::<Self>(key()));
    }
}

impl Default for Palette {
    fn default() -> Self {
        Self::dark()
    }
}

fn key() -> Id {
    Id::new("bodo-kit-ui palette")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_installed_palette_wins_over_the_visuals() {
        let ctx = Context::default();
        ctx.set_visuals(egui::Visuals::light());
        assert_eq!(Palette::get(&ctx), Palette::light());
        ctx.set_visuals(egui::Visuals::dark());
        assert_eq!(Palette::get(&ctx), Palette::dark());
        let custom = Palette { accent: Color32::RED, ..Palette::dark() };
        custom.install(&ctx);
        assert_eq!(Palette::get(&ctx), custom);
        Palette::uninstall(&ctx);
        assert_eq!(Palette::get(&ctx), Palette::dark());
    }
}
