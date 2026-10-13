//! MIDI for Bodo Kit.
//!
//! - [`message`]: MIDI 1.0 channel voice messages and the [`U4`], [`U7`] and [`U14`] value types.
//! - [`smf`]: reading and writing Standard MIDI Files with [`Smf`].

#![forbid(unsafe_code)]

pub mod message;
pub mod smf;

pub use message::{MidiMessage, U4, U7, U14};
pub use smf::{Smf, SmfError};
