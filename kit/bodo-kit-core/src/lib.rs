//! Core building blocks of Bodo Kit.
//!
//! - [`time`]: the [`Tick`], [`Frames`] and [`Seconds`] units, [`SampleRate`] and [`PPQ`].
//!
//! Functions that are safe to call on the audio thread are documented with `RT-safe`.

#![forbid(unsafe_code)]

pub mod time;

pub use time::{Frames, PPQ, SampleRate, Seconds, Tick};
