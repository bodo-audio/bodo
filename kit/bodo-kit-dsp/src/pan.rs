//! Pan laws (project.md section 9.5).
//!
//! - **Mono sources** are placed with one of three classic laws, chosen per project ([`PanLaw`]).
//! - **Stereo sources** use stereo-fold panning ([`StereoPan::fold`]): at center the signal passes through
//!   bit-exactly, and panning crossfades the opposite side across with a constant-power curve, so at 100% both
//!   channels sum on one side.
//!
//! Pan positions run from -1 (hard left) through 0 (center) to +1 (hard right). Values outside are clamped.

use bodo_kit_core::rt_safe;
use core::f64::consts::FRAC_PI_2;

/// How a mono source is spread over two channels as it pans.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(rename_all = "camelCase"))]
pub enum PanLaw {
    /// Each side is -3 dB at center, so loudness stays constant across the field. The default.
    #[default]
    ConstantPower3dB,
    /// Each side is -4.5 dB at center, between constant power and linear.
    Compromise4_5dB,
    /// Each side is -6 dB at center; the two gains always add up to one.
    Linear6dB,
}

/// Left and right gains for a mono source.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanGains {
    /// Gain into the left channel.
    pub left: f32,
    /// Gain into the right channel.
    pub right: f32,
}

impl PanLaw {
    /// The gains that place a mono source at `pan`.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn mono_gains(self, pan: f32) -> PanGains {
        // x runs from 0 (hard left) to 1 (hard right).
        let x = (f64::from(pan.clamp(-1.0, 1.0)) + 1.0) / 2.0;
        let (left, right) = match self {
            Self::Linear6dB => (1.0 - x, x),
            Self::ConstantPower3dB => ((x * FRAC_PI_2).cos(), (x * FRAC_PI_2).sin()),
            Self::Compromise4_5dB => (((1.0 - x) * (x * FRAC_PI_2).cos()).sqrt(), (x * (x * FRAC_PI_2).sin()).sqrt()),
        };
        // Pin the ends so a hard pan silences the other side exactly.
        let (left, right) = if x <= 0.0 {
            (1.0, 0.0)
        } else if x >= 1.0 {
            (0.0, 1.0)
        } else {
            (left, right)
        };
        PanGains { left: left as f32, right: right as f32 }
    }
}

/// A 2x2 matrix that pans a stereo signal: `L' = ll * L + rl * R` and `R' = lr * L + rr * R`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StereoPan {
    /// Left input into the left output.
    pub ll: f32,
    /// Left input into the right output.
    pub lr: f32,
    /// Right input into the left output.
    pub rl: f32,
    /// Right input into the right output.
    pub rr: f32,
}

impl StereoPan {
    /// The matrix that leaves the signal unchanged.
    pub const IDENTITY: Self = Self { ll: 1.0, lr: 0.0, rl: 0.0, rr: 1.0 };

    /// Stereo-fold panning: panning right moves the left channel into the right with a constant-power crossfade,
    /// and panning left does the reverse. Center is the identity.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn fold(pan: f32) -> Self {
        let pan = pan.clamp(-1.0, 1.0);
        if pan == 0.0 {
            return Self::IDENTITY;
        }
        let theta = f64::from(pan.abs()) * FRAC_PI_2;
        let (stay, cross) = if pan.abs() >= 1.0 { (0.0, 1.0) } else { (theta.cos() as f32, theta.sin() as f32) };
        if pan > 0.0 { Self { ll: stay, lr: cross, rl: 0.0, rr: 1.0 } } else { Self { ll: 1.0, lr: 0.0, rl: cross, rr: stay } }
    }

    /// Applies the matrix to a stereo block in place. The identity leaves the samples bit-exact.
    ///
    /// RT-safe. Complexity: O(n) in the number of frames.
    #[rt_safe]
    pub fn process(&self, left: &mut [f32], right: &mut [f32]) {
        if *self == Self::IDENTITY {
            return;
        }
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let (in_l, in_r) = (*l, *r);
            *l = self.ll * in_l + self.rl * in_r;
            *r = self.lr * in_l + self.rr * in_r;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db(gain: f32) -> f32 {
        20.0 * gain.log10()
    }

    #[test]
    fn mono_laws_hit_their_center_levels() {
        let center = |law: PanLaw| law.mono_gains(0.0);
        assert!((db(center(PanLaw::ConstantPower3dB).left) + 3.0103).abs() < 1e-3);
        assert!((db(center(PanLaw::Compromise4_5dB).left) + 4.5154).abs() < 1e-3);
        assert!((db(center(PanLaw::Linear6dB).left) + 6.0206).abs() < 1e-3);
        for law in [PanLaw::ConstantPower3dB, PanLaw::Compromise4_5dB, PanLaw::Linear6dB] {
            let g = center(law);
            assert_eq!(g.left, g.right, "{law:?} is symmetric at center");
            assert_eq!(law.mono_gains(-1.0), PanGains { left: 1.0, right: 0.0 });
            assert_eq!(law.mono_gains(1.0), PanGains { left: 0.0, right: 1.0 });
            assert_eq!(law.mono_gains(5.0), law.mono_gains(1.0), "out of range is clamped");
        }
    }

    #[test]
    fn constant_power_keeps_power_constant() {
        for step in -10..=10 {
            let g = PanLaw::ConstantPower3dB.mono_gains(step as f32 / 10.0);
            assert!((g.left * g.left + g.right * g.right - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn stereo_fold_is_bit_exact_at_center() {
        let (mut l, mut r) = ([0.3f32, -0.1, 1e-38], [0.7f32, 0.2, -1e-38]);
        let (l0, r0) = (l, r);
        StereoPan::fold(0.0).process(&mut l, &mut r);
        assert_eq!((l, r), (l0, r0));
    }

    #[test]
    fn stereo_fold_sums_to_one_side_at_the_extremes() {
        let (mut l, mut r) = ([0.25f32], [0.5f32]);
        StereoPan::fold(1.0).process(&mut l, &mut r);
        assert_eq!((l[0], r[0]), (0.0, 0.75));
        let (mut l, mut r) = ([0.25f32], [0.5f32]);
        StereoPan::fold(-1.0).process(&mut l, &mut r);
        assert_eq!((l[0], r[0]), (0.75, 0.0));
    }

    #[test]
    fn stereo_fold_crossfades_with_constant_power() {
        let m = StereoPan::fold(0.5);
        assert!((m.ll * m.ll + m.lr * m.lr - 1.0).abs() < 1e-6);
        assert_eq!((m.rl, m.rr), (0.0, 1.0), "the right channel is untouched when panning right");
    }
}
