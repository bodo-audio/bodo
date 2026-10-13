//! Interpolation between samples, for playing sounds back at other pitches.

use bodo_kit_core::rt_safe;

/// Linear interpolation between `x0` and `x1` at fraction `t` (0 gives `x0`, 1 gives `x1`).
///
/// RT-safe.
#[rt_safe]
#[inline]
pub fn linear(x0: f32, x1: f32, t: f32) -> f32 {
    x0 + (x1 - x0) * t
}

/// Four-point cubic Hermite (Catmull-Rom) interpolation between `x0` and `x1` at fraction `t`, using the neighbors
/// `xm1` before and `x2` after. It passes through the samples exactly and sounds much cleaner than linear when
/// pitching down.
///
/// RT-safe.
#[rt_safe]
#[inline]
pub fn hermite(xm1: f32, x0: f32, x1: f32, x2: f32, t: f32) -> f32 {
    let c1 = 0.5 * (x1 - xm1);
    let c2 = xm1 - 2.5 * x0 + 2.0 * x1 - 0.5 * x2;
    let c3 = 0.5 * (x2 - xm1) + 1.5 * (x0 - x1);
    ((c3 * t + c2) * t + c1) * t + x0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolators_pass_through_the_samples() {
        assert_eq!(linear(1.0, 3.0, 0.0), 1.0);
        assert_eq!(linear(1.0, 3.0, 1.0), 3.0);
        assert_eq!(linear(1.0, 3.0, 0.5), 2.0);
        assert_eq!(hermite(0.0, 1.0, 2.0, 3.0, 0.0), 1.0);
        assert!((hermite(0.0, 1.0, 2.0, 3.0, 1.0) - 2.0).abs() < 1e-6);
    }

    #[test]
    fn hermite_reproduces_straight_lines_and_parabolas() {
        assert!((hermite(0.0, 1.0, 2.0, 3.0, 0.25) - 1.25).abs() < 1e-6);
        // Catmull-Rom reproduces quadratics: x^2 at x = -1, 0, 1, 2, evaluated at 0.5.
        assert!((hermite(1.0, 0.0, 1.0, 4.0, 0.5) - 0.25).abs() < 1e-6);
    }
}
