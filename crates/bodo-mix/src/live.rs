//! Parameters the control thread changes while the audio thread plays, without recompiling the graph.
//!
//! Each processor shares one of these with the engine's control side. The control side stores new values whenever
//! the project changes; the processor reads them once per block and smooths toward them.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// An `f32` that can be shared between threads.
#[derive(Debug, Default)]
pub struct AtomicF32(AtomicU32);

impl AtomicF32 {
    /// A cell holding `value`.
    pub fn new(value: f32) -> Self {
        Self(AtomicU32::new(value.to_bits()))
    }

    /// The value.
    pub fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }

    /// Stores `value`.
    pub fn set(&self, value: f32) {
        self.0.store(value.to_bits(), Ordering::Relaxed);
    }
}

/// Live settings of a channel.
#[derive(Debug)]
pub struct ChannelLive {
    /// Volume as a linear gain.
    pub volume: AtomicF32,
    /// Pan, -1 to 1.
    pub pan: AtomicF32,
    /// Pitch in cents.
    pub pitch_cents: AtomicF32,
    /// Whether the channel is muted.
    pub muted: AtomicBool,
}

impl Default for ChannelLive {
    fn default() -> Self {
        Self { volume: AtomicF32::new(1.0), pan: AtomicF32::new(0.0), pitch_cents: AtomicF32::new(0.0), muted: AtomicBool::new(false) }
    }
}

/// Live settings of a mixer insert or the master.
#[derive(Debug, Default)]
pub struct StripLive {
    /// The fader in dB.
    pub volume_db: AtomicF32,
    /// Pan, -1 to 1.
    pub pan: AtomicF32,
    /// Stereo separation, -1 to 1.
    pub stereo_sep: AtomicF32,
    /// Polarity inverted.
    pub polarity_invert: AtomicBool,
    /// Left and right swapped.
    pub swap_lr: AtomicBool,
    /// Muted.
    pub muted: AtomicBool,
}
