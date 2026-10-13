//! Core building blocks of Bodo Kit.
//!
//! - [`time`]: the [`Tick`], [`Frames`] and [`Seconds`] units, [`SampleRate`] and [`PPQ`].
//! - [`tempo`]: [`TempoMap`], converting between ticks and seconds through jumps and linear ramps.
//! - [`queue`]: bounded single-producer, single-consumer queues with explicit overflow policies.
//! - [`pool`]: [`RtPool`], real-time worker threads that help the audio thread in parallel regions.
//! - [`rt`]: the checks behind [`rt_safe`], and denormal flushing.
//!
//! Functions that are safe to call on the audio thread are documented with `RT-safe` and marked [`rt_safe`].

// Lets `#[rt_safe]` refer to this crate as `::bodo_kit_core`, also from inside it.
extern crate self as bodo_kit_core;

#[allow(unsafe_code)]
mod ffi;

pub mod pool;
pub mod queue;
pub mod rt;
pub mod tempo;
pub mod time;

pub use bodo_kit_macros::rt_safe;
pub use pool::{RtJob, RtPool};
pub use tempo::{TempoCurve, TempoMap, TempoPoint};
pub use time::{Frames, PPQ, SampleRate, Seconds, Tick};
