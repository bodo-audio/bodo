//! Gain in decibels and as a linear factor, and applying it to audio.

use bodo_kit_core::rt_safe;

/// Converts decibels to a linear gain factor. Negative infinity gives exactly zero.
///
/// RT-safe.
#[rt_safe]
pub fn db_to_gain(db: f32) -> f32 {
    if db == f32::NEG_INFINITY { 0.0 } else { 10f64.powf(f64::from(db) / 20.0) as f32 }
}

/// Converts a linear gain factor to decibels. Zero and negative factors give negative infinity.
///
/// RT-safe.
#[rt_safe]
pub fn gain_to_db(gain: f32) -> f32 {
    if gain <= 0.0 { f32::NEG_INFINITY } else { (20.0 * f64::from(gain).log10()) as f32 }
}

/// Multiplies every sample by `gain`. A gain of exactly one leaves the samples untouched.
///
/// RT-safe. Complexity: O(n) in the number of samples.
#[rt_safe]
pub fn apply_gain(samples: &mut [f32], gain: f32) {
    if gain == 1.0 {
        return;
    }
    for sample in samples {
        *sample *= gain;
    }
}

/// Multiplies the samples by a gain that moves linearly from `from` to `to` across the block, reaching `to` on the
/// last sample. Used for click-free gain changes, such as the 5 ms mute ramp.
///
/// RT-safe. Complexity: O(n) in the number of samples.
#[rt_safe]
pub fn ramp_gain(samples: &mut [f32], from: f32, to: f32) {
    if from == to {
        apply_gain(samples, to);
        return;
    }
    let len = samples.len();
    let step = (to - from) / len.max(1) as f32;
    for (index, sample) in samples.iter_mut().enumerate() {
        let gain = if index + 1 == len { to } else { from + step * (index + 1) as f32 };
        *sample *= gain;
    }
}

/// Adds `source` times `gain` into `target`, sample by sample, over the shorter of the two.
///
/// RT-safe. Complexity: O(n) in the number of samples.
#[rt_safe]
pub fn mix_into(target: &mut [f32], source: &[f32], gain: f32) {
    if gain == 1.0 {
        for (out, input) in target.iter_mut().zip(source) {
            *out += *input;
        }
    } else if gain != 0.0 {
        for (out, input) in target.iter_mut().zip(source) {
            *out += *input * gain;
        }
    }
}

/// Adds `source` into `target` with a gain that moves linearly from `from` to `to` across the block, reaching `to`
/// on the last sample. With equal gains it is [`mix_into`].
///
/// RT-safe. Complexity: O(n) in the number of samples.
#[rt_safe]
pub fn mix_into_ramped(target: &mut [f32], source: &[f32], from: f32, to: f32) {
    if from == to {
        mix_into(target, source, to);
        return;
    }
    let len = target.len().min(source.len());
    let step = (to - from) / len.max(1) as f32;
    for (index, (out, input)) in target.iter_mut().zip(source).enumerate() {
        let gain = if index + 1 == len { to } else { from + step * (index + 1) as f32 };
        *out += *input * gain;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_between_decibels_and_gain() {
        assert_eq!(db_to_gain(0.0), 1.0);
        assert_eq!(db_to_gain(f32::NEG_INFINITY), 0.0);
        assert!((db_to_gain(6.0) - 1.995_262_3).abs() < 1e-6);
        assert!((db_to_gain(-20.0) - 0.1).abs() < 1e-7);
        assert_eq!(gain_to_db(1.0), 0.0);
        assert_eq!(gain_to_db(0.0), f32::NEG_INFINITY);
        assert!((gain_to_db(0.5) + 6.020_6).abs() < 1e-4);
    }

    #[test]
    fn unity_gain_is_bit_exact() {
        let original = [0.1f32, -0.7, 1e-30, f32::MAX];
        let mut samples = original;
        apply_gain(&mut samples, 1.0);
        assert_eq!(samples, original);
    }

    #[test]
    fn ramp_ends_on_the_target() {
        let mut samples = [1.0f32; 4];
        ramp_gain(&mut samples, 1.0, 0.0);
        assert_eq!(samples, [0.75, 0.5, 0.25, 0.0]);
    }

    #[test]
    fn mixes_with_a_ramp() {
        let mut target = [0.0f32; 4];
        mix_into_ramped(&mut target, &[1.0; 4], 0.0, 1.0);
        assert_eq!(target, [0.25, 0.5, 0.75, 1.0]);
    }

    #[test]
    fn mixes_with_gain() {
        let mut target = [1.0f32, 1.0, 1.0];
        mix_into(&mut target, &[2.0, 4.0], 0.5);
        assert_eq!(target, [2.0, 3.0, 1.0]);
    }
}
