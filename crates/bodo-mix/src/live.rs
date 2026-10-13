//! Parameters the control thread changes while the audio thread plays, without recompiling the graph.
//!
//! Each processor shares one of these with the engine's control side. The control side stores new values whenever
//! the project changes; the processor reads them once per block and smooths toward them.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// No device input channel, in [`StripLive::input_left`] and [`StripLive::input_right`].
pub const NO_INPUT: u32 = 0;

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
    /// Armed for recording; an armed insert also monitors its hardware input.
    pub armed: AtomicBool,
    /// The device input channel feeding the left side, plus one, or [`NO_INPUT`].
    pub input_left: AtomicU32,
    /// The device input channel feeding the right side, plus one, or [`NO_INPUT`].
    pub input_right: AtomicU32,
}

impl StripLive {
    /// The device input channels feeding this strip, left and right, when it has an input.
    pub fn input_channels(&self) -> Option<(usize, usize)> {
        let (left, right) = (self.input_left.load(Ordering::Relaxed), self.input_right.load(Ordering::Relaxed));
        (left != NO_INPUT && right != NO_INPUT).then(|| (left as usize - 1, right as usize - 1))
    }

    /// Feeds the strip from device input `channels`: one for mono, two for stereo, none for no input.
    pub fn set_input(&self, channels: &[u16]) {
        let code = |c: Option<&u16>| c.map_or(NO_INPUT, |c| u32::from(*c) + 1);
        let (left, right) = match channels {
            [mono] => (code(Some(mono)), code(Some(mono))),
            [left, right, ..] => (code(Some(left)), code(Some(right))),
            [] => (NO_INPUT, NO_INPUT),
        };
        self.input_left.store(left, Ordering::Relaxed);
        self.input_right.store(right, Ordering::Relaxed);
    }
}
