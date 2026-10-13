//! Stereo utilities: separation (mid/side width), polarity and channel swap (project.md section 9.5).

use bodo_kit_core::rt_safe;

/// Changes stereo width in mid/side: `M = (L + R) / 2`, `S = (L - R) / 2`, `w = 1 - separation`,
/// `L' = M + w * S`, `R' = M - w * S`. A separation of +1 makes the signal mono, 0 leaves it unchanged bit-exactly,
/// and -1 doubles the side signal. Values outside -1..=1 are clamped.
///
/// RT-safe. Complexity: O(n) in the number of frames.
#[rt_safe]
pub fn stereo_separation(left: &mut [f32], right: &mut [f32], separation: f32) {
    let separation = separation.clamp(-1.0, 1.0);
    if separation == 0.0 {
        return;
    }
    let width = 1.0 - separation;
    for (l, r) in left.iter_mut().zip(right.iter_mut()) {
        let mid = (*l + *r) * 0.5;
        let side = (*l - *r) * 0.5 * width;
        *l = mid + side;
        *r = mid - side;
    }
}

/// Inverts the polarity of every sample.
///
/// RT-safe. Complexity: O(n) in the number of samples.
#[rt_safe]
pub fn invert_polarity(samples: &mut [f32]) {
    for sample in samples {
        *sample = -*sample;
    }
}

/// Swaps the left and right channels.
///
/// RT-safe. Complexity: O(n) in the number of frames.
#[rt_safe]
pub fn swap_channels(left: &mut [f32], right: &mut [f32]) {
    let frames = left.len().min(right.len());
    left[..frames].swap_with_slice(&mut right[..frames]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_separation_is_bit_exact() {
        let (mut l, mut r) = ([0.1f32, 0.3], [0.7f32, -0.2]);
        stereo_separation(&mut l, &mut r, 0.0);
        assert_eq!((l, r), ([0.1, 0.3], [0.7, -0.2]));
    }

    #[test]
    fn full_separation_is_mono_and_negative_widens() {
        let (mut l, mut r) = ([1.0f32], [0.0f32]);
        stereo_separation(&mut l, &mut r, 1.0);
        assert_eq!((l[0], r[0]), (0.5, 0.5));
        let (mut l, mut r) = ([1.0f32], [0.0f32]);
        stereo_separation(&mut l, &mut r, -1.0);
        assert_eq!((l[0], r[0]), (1.5, -0.5));
    }

    #[test]
    fn polarity_and_swap() {
        let mut samples = [0.5f32, -0.25];
        invert_polarity(&mut samples);
        assert_eq!(samples, [-0.5, 0.25]);
        let (mut l, mut r) = ([1.0f32, 2.0], [3.0f32, 4.0]);
        swap_channels(&mut l, &mut r);
        assert_eq!((l, r), ([3.0, 4.0], [1.0, 2.0]));
    }
}
