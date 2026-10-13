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

pub use fader::Fader;
pub use icon::Icon;
pub use knob::{DragAxis, Knob};
pub use led::Led;
pub use meter::Meter;
pub use palette::Palette;
pub use step::{StepRow, StepRowResponse};
