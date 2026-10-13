//! egui widgets for Bodo Kit (project.md sections 5 and 14.4, ADR 0001).
//!
//! - [`icon`] and [`icons`]: line icons stroked from SVG path data, [`Icon`].
//! - [`palette`]: [`Palette`], the colors every widget draws with.

#![forbid(unsafe_code)]

pub mod icon;
pub mod icons;
pub mod palette;

pub use icon::Icon;
pub use palette::Palette;
