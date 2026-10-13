//! Time units: musical ticks, sample frames and seconds.
//!
//! The three units are distinct newtypes so that raw numbers never cross an API boundary. Musical time is
//! [`Tick`] at [`PPQ`] ticks per quarter note, audio time is [`Frames`] at a [`SampleRate`], and wall-clock
//! time is [`Seconds`]. Converting between musical and wall-clock time needs a tempo map, see
//! [`crate::tempo::TempoMap`].
//!
//! Integer arithmetic on [`Tick`] and [`Frames`] saturates instead of overflowing, so it never panics.
//! An `i64` of ticks at 960 PPQ covers millions of years, so saturation is unreachable in practice.

use core::num::NonZeroU32;
use core::ops::{Add, AddAssign, Neg, Sub, SubAssign};

/// Ticks per quarter note.
///
/// 960 = 2^6 * 3 * 5, so straight notes down to 1/256, triplets and quintuplets are exact, and the value fits a
/// Standard MIDI File header, so MIDI round trips are lossless.
pub const PPQ: i64 = 960;

/// A musical position or duration, in ticks of 1/[`PPQ`] quarter note.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(transparent))]
pub struct Tick(pub i64);

impl Tick {
    /// Tick zero, the start of the project.
    pub const ZERO: Self = Self(0);
    /// One quarter note.
    pub const QUARTER: Self = Self(PPQ);

    /// Returns the duration of `quarters` quarter notes, saturating at the `i64` range.
    pub const fn from_quarters(quarters: i64) -> Self {
        Self(quarters.saturating_mul(PPQ))
    }

    /// Returns this position in quarter notes, as a fraction.
    pub fn as_quarters(self) -> f64 {
        self.0 as f64 / PPQ as f64
    }
}

/// A position or duration in sample frames. One frame holds one sample per channel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(transparent))]
pub struct Frames(pub i64);

impl Frames {
    /// Frame zero.
    pub const ZERO: Self = Self(0);
}

/// A wall-clock position or duration in seconds.
#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(transparent))]
pub struct Seconds(pub f64);

impl Seconds {
    /// Zero seconds.
    pub const ZERO: Self = Self(0.0);
}

/// An audio sample rate in hertz. Never zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(transparent))]
pub struct SampleRate(NonZeroU32);

impl SampleRate {
    /// 44.1 kHz.
    pub const HZ_44100: Self = Self(NonZeroU32::MIN.saturating_add(44_099));
    /// 48 kHz.
    pub const HZ_48000: Self = Self(NonZeroU32::MIN.saturating_add(47_999));

    /// Returns the sample rate `hz`, or `None` when `hz` is zero.
    pub const fn new(hz: u32) -> Option<Self> {
        match NonZeroU32::new(hz) {
            Some(hz) => Some(Self(hz)),
            None => None,
        }
    }

    /// Returns the rate in hertz.
    pub const fn hz(self) -> u32 {
        self.0.get()
    }

    /// Converts a frame count to seconds.
    ///
    /// RT-safe.
    pub fn frames_to_seconds(self, frames: Frames) -> Seconds {
        Seconds(frames.0 as f64 / f64::from(self.hz()))
    }

    /// Converts seconds to the nearest frame. Rounds half away from zero, saturates at the `i64` range and maps
    /// NaN to frame zero.
    ///
    /// RT-safe.
    pub fn seconds_to_frames(self, seconds: Seconds) -> Frames {
        // `as` from f64 to i64 saturates and maps NaN to 0, which is the documented behavior.
        Frames((seconds.0 * f64::from(self.hz())).round() as i64)
    }
}

macro_rules! saturating_integer_ops {
    ($unit:ident) => {
        impl Add for $unit {
            type Output = Self;
            fn add(self, rhs: Self) -> Self {
                Self(self.0.saturating_add(rhs.0))
            }
        }

        impl Sub for $unit {
            type Output = Self;
            fn sub(self, rhs: Self) -> Self {
                Self(self.0.saturating_sub(rhs.0))
            }
        }

        impl AddAssign for $unit {
            fn add_assign(&mut self, rhs: Self) {
                *self = *self + rhs;
            }
        }

        impl SubAssign for $unit {
            fn sub_assign(&mut self, rhs: Self) {
                *self = *self - rhs;
            }
        }

        impl Neg for $unit {
            type Output = Self;
            fn neg(self) -> Self {
                Self(self.0.saturating_neg())
            }
        }
    };
}

saturating_integer_ops!(Tick);
saturating_integer_ops!(Frames);

impl Add for Seconds {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self(self.0 + rhs.0)
    }
}

impl Sub for Seconds {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self(self.0 - rhs.0)
    }
}

impl AddAssign for Seconds {
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl SubAssign for Seconds {
    fn sub_assign(&mut self, rhs: Self) {
        self.0 -= rhs.0;
    }
}

impl Neg for Seconds {
    type Output = Self;
    fn neg(self) -> Self {
        Self(-self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quarter_note_is_ppq_ticks() {
        assert_eq!(Tick::from_quarters(4), Tick(3840));
        assert_eq!(Tick::QUARTER.as_quarters(), 1.0);
        assert_eq!(Tick(480).as_quarters(), 0.5);
    }

    #[test]
    fn tick_and_frame_arithmetic_saturates() {
        assert_eq!(Tick(i64::MAX) + Tick(1), Tick(i64::MAX));
        assert_eq!(Tick(i64::MIN) - Tick(1), Tick(i64::MIN));
        assert_eq!(-Tick(i64::MIN), Tick(i64::MAX));
        assert_eq!(Tick::from_quarters(i64::MAX), Tick(i64::MAX));
        let mut f = Frames(10);
        f += Frames(5);
        f -= Frames(20);
        assert_eq!(f, Frames(-5));
    }

    #[test]
    fn sample_rate_rejects_zero() {
        assert_eq!(SampleRate::new(0), None);
        assert_eq!(SampleRate::new(48_000), Some(SampleRate::HZ_48000));
        assert_eq!(SampleRate::HZ_44100.hz(), 44_100);
    }

    #[test]
    fn frames_and_seconds_convert() {
        let rate = SampleRate::HZ_48000;
        assert_eq!(rate.frames_to_seconds(Frames(24_000)), Seconds(0.5));
        assert_eq!(rate.seconds_to_frames(Seconds(0.5)), Frames(24_000));
        assert_eq!(rate.seconds_to_frames(Seconds(-1.0)), Frames(-48_000));
        // Rounds to the nearest frame.
        assert_eq!(rate.seconds_to_frames(Seconds(1.4 / 48_000.0)), Frames(1));
        assert_eq!(rate.seconds_to_frames(Seconds(1.6 / 48_000.0)), Frames(2));
        // Non-finite input never panics.
        assert_eq!(rate.seconds_to_frames(Seconds(f64::NAN)), Frames(0));
        assert_eq!(rate.seconds_to_frames(Seconds(f64::INFINITY)), Frames(i64::MAX));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn units_serialize_as_plain_numbers() {
        assert_eq!(serde_json::to_string(&Tick(960)).unwrap(), "960");
        assert_eq!(serde_json::to_string(&SampleRate::HZ_44100).unwrap(), "44100");
        assert!(serde_json::from_str::<SampleRate>("0").is_err());
        assert_eq!(serde_json::from_str::<Seconds>("1.5").unwrap(), Seconds(1.5));
    }
}
