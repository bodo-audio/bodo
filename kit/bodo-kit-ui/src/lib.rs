//! egui widgets for Bodo Kit (project.md sections 5 and 14.4, ADR 0001).
//!
//! The same widgets draw Bodo's mixer and channel rack and the editors of plugins built with Bodo Kit:
//!
//! - [`icon`] and [`icons`]: line icons stroked from SVG path data, [`Icon`].
//! - [`knob`]: [`Knob`], with [`knob::pan`] for pan knobs.
//! - [`fader`]: [`Fader`], a volume fader in dB with a 0 dB detent.
//! - [`led`]: [`Led`], an on/off light such as a mute LED.
//! - [`step`]: [`StepRow`], a step sequencer row that toggles and paints.
//! - [`meter`]: [`Meter`], a peak meter with a clip LED.
//! - [`palette`]: [`Palette`], the colors every widget draws with.
//!
//! Sizes follow the host's text: a widget without a size of its own is a multiple of [`em`], the body text size of
//! its `Ui`'s style, the way CSS sizes follow `rem`. A host that makes its text larger makes every widget larger.
//!
//! Widgets never own the data they show: they edit a borrowed value (knob, fader, LED) or report edits for the caller
//! to apply (step row), and their responses mark the start and end of a gesture so a host can make it one undo step.
//! Every widget describes itself to screen readers and to `egui_kittest` through `widget_info`.

#![forbid(unsafe_code)]

pub mod fader;
pub mod icon;
pub mod icons;
pub mod knob;
pub mod led;
pub mod meter;
pub mod palette;
pub mod step;

/// The size Kit's widgets scale from: the body text size of `ui`'s style, in points; 12.5 when the style has none.
pub fn em(ui: &egui::Ui) -> f32 {
    ui.style().text_styles.get(&egui::TextStyle::Body).map_or(12.5, |font| font.size)
}

pub use fader::Fader;
pub use icon::Icon;
pub use knob::{DragAxis, Knob};
pub use led::Led;
pub use meter::Meter;
pub use palette::Palette;
pub use step::{StepRow, StepRowResponse};
