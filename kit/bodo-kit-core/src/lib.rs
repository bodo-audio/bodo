//! Core building blocks of Bodo Kit.
//!
//! - [`time`]: the [`Tick`], [`Frames`] and [`Seconds`] units, [`SampleRate`] and [`PPQ`].
//! - [`tempo`]: [`TempoMap`], converting between ticks and seconds through jumps and linear ramps.
//! - [`queue`]: bounded single-producer, single-consumer queues with explicit overflow policies.
//!
//! Functions that are safe to call on the audio thread are documented with `RT-safe`.

#![forbid(unsafe_code)]

pub mod queue;
pub mod tempo;
pub mod time;

pub use tempo::{TempoCurve, TempoMap, TempoPoint};
pub use time::{Frames, PPQ, SampleRate, Seconds, Tick};
