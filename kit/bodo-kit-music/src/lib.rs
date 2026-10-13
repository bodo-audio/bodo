//! Music theory and performance for Bodo Kit (project.md sections 5 and 13.5).
//!
//! - [`pitch`]: pitch classes and note names, with C4 as MIDI key 60.
//! - [`scale`]: scales and modes, and snapping keys into them.
//! - [`chord`]: chord symbols such as `Cmaj7` or `Am7b5/G`, voiced in close position.
//! - [`timing`]: quantize with strength and swing, strums and arpeggio orders.
//! - [`humanize`]: correlated timing drift and velocity variation.
//! - [`rng`]: the small seeded generator everything random here uses.
//!
//! Everything is pure and deterministic: the same input and seed always give the same result, so a performance can be
//! replayed and undone exactly.

#![forbid(unsafe_code)]

pub mod chord;
pub mod humanize;
pub mod pitch;
pub mod rng;
pub mod scale;
pub mod timing;

pub use chord::{Chord, ChordError};
pub use humanize::Humanizer;
pub use pitch::{PitchClass, PitchError};
pub use rng::Rng;
pub use scale::{Scale, ScaleError, ScaleKind};
pub use timing::{ArpPattern, Direction, arp_order, quantize, strum_offsets};
