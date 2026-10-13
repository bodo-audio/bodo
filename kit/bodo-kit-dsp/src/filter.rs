//! Filters: RBJ biquads and a state-variable filter.
//!
//! - [`Biquad`] implements the classic "Audio EQ Cookbook" responses (low/high pass, band pass, notch, peak,
//!   shelves, all pass) in transposed direct form II. Use it for EQ bands, whose settings change rarely.
//! - [`Svf`] is the trapezoidal state-variable filter by Andrew Simper. It stays stable and smooth when its cutoff is
//!   modulated every sample, so it suits synth and sampler filters.
//!
//! Coefficients are designed in `f64` and run in `f32`. Frequencies are clamped to 1 Hz up to just below Nyquist,
//! and Q to at least 0.01.

use bodo_kit_core::{SampleRate, rt_safe};
use core::f64::consts::PI;

/// The response of a [`Biquad`].
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(rename_all = "camelCase"))]
pub enum BiquadKind {
    /// Passes frequencies below the cutoff.
    LowPass,
    /// Passes frequencies above the cutoff.
    HighPass,
    /// Passes a band around the center, with 0 dB at its peak.
    BandPass,
    /// Removes a band around the center.
    Notch,
    /// Boosts or cuts a band around the center.
    Peak {
        /// Gain at the center, in dB.
        gain_db: f64,
    },
    /// Boosts or cuts everything below the corner.
    LowShelf {
        /// Gain of the shelf, in dB.
        gain_db: f64,
    },
    /// Boosts or cuts everything above the corner.
    HighShelf {
        /// Gain of the shelf, in dB.
        gain_db: f64,
    },
    /// Passes every frequency at the same level and shifts the phase around the center.
    AllPass,
}

/// Normalized biquad coefficients: `H(z) = (b0 + b1 z^-1 + b2 z^-2) / (1 + a1 z^-1 + a2 z^-2)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BiquadCoeffs {
    /// Feed-forward coefficient of the current input.
    pub b0: f64,
    /// Feed-forward coefficient of the previous input.
    pub b1: f64,
    /// Feed-forward coefficient of the input two samples ago.
    pub b2: f64,
    /// Feedback coefficient of the previous output.
    pub a1: f64,
    /// Feedback coefficient of the output two samples ago.
    pub a2: f64,
}

impl BiquadCoeffs {
    /// Coefficients that pass the signal unchanged.
    pub const IDENTITY: Self = Self { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0 };

    /// Designs `kind` at `frequency` Hz with quality `q`, using the formulas of the RBJ Audio EQ Cookbook.
    pub fn design(kind: BiquadKind, frequency: f64, q: f64, rate: SampleRate) -> Self {
        let fs = f64::from(rate.hz());
        let frequency = frequency.clamp(1.0, fs * 0.499);
        let q = q.max(0.01);
        let w0 = 2.0 * PI * frequency / fs;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * q);
        let amp = |gain_db: f64| 10f64.powf(gain_db / 40.0);
        let (b0, b1, b2, a0, a1, a2) = match kind {
            BiquadKind::LowPass => ((1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
            BiquadKind::HighPass => ((1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
            BiquadKind::BandPass => (alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
            BiquadKind::Notch => (1.0, -2.0 * cos, 1.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
            BiquadKind::AllPass => (1.0 - alpha, -2.0 * cos, 1.0 + alpha, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
            BiquadKind::Peak { gain_db } => {
                let a = amp(gain_db);
                (1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * cos, 1.0 - alpha / a)
            }
            BiquadKind::LowShelf { gain_db } => {
                let a = amp(gain_db);
                let s = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) - (a - 1.0) * cos + s),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                    a * ((a + 1.0) - (a - 1.0) * cos - s),
                    (a + 1.0) + (a - 1.0) * cos + s,
                    -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                    (a + 1.0) + (a - 1.0) * cos - s,
                )
            }
            BiquadKind::HighShelf { gain_db } => {
                let a = amp(gain_db);
                let s = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) + (a - 1.0) * cos + s),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                    a * ((a + 1.0) + (a - 1.0) * cos - s),
                    (a + 1.0) - (a - 1.0) * cos + s,
                    2.0 * ((a - 1.0) - (a + 1.0) * cos),
                    (a + 1.0) - (a - 1.0) * cos - s,
                )
            }
        };
        Self { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0 }
    }

    /// The magnitude of the response at `frequency` Hz, as a linear factor.
    pub fn magnitude(&self, frequency: f64, rate: SampleRate) -> f64 {
        let w = 2.0 * PI * frequency / f64::from(rate.hz());
        // Evaluate numerator and denominator at z = e^{jw}, using z^-1 = cos w - j sin w.
        let (s1, c1) = w.sin_cos();
        let (s2, c2) = (2.0 * w).sin_cos();
        let num_re = self.b0 + self.b1 * c1 + self.b2 * c2;
        let num_im = -(self.b1 * s1 + self.b2 * s2);
        let den_re = 1.0 + self.a1 * c1 + self.a2 * c2;
        let den_im = -(self.a1 * s1 + self.a2 * s2);
        ((num_re * num_re + num_im * num_im) / (den_re * den_re + den_im * den_im)).sqrt()
    }
}

/// A biquad filter in transposed direct form II.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    /// A filter with `coeffs` and silent state.
    pub fn new(coeffs: BiquadCoeffs) -> Self {
        let mut filter = Self { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0, z1: 0.0, z2: 0.0 };
        filter.set_coeffs(coeffs);
        filter
    }

    /// Replaces the coefficients and keeps the state, so a change doesn't click.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn set_coeffs(&mut self, coeffs: BiquadCoeffs) {
        self.b0 = coeffs.b0 as f32;
        self.b1 = coeffs.b1 as f32;
        self.b2 = coeffs.b2 as f32;
        self.a1 = coeffs.a1 as f32;
        self.a2 = coeffs.a2 as f32;
    }

    /// Filters one sample.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn process_sample(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    /// Filters a block in place.
    ///
    /// RT-safe. Complexity: O(n) in the number of samples.
    #[rt_safe]
    pub fn process(&mut self, samples: &mut [f32]) {
        for sample in samples {
            *sample = self.process_sample(*sample);
        }
    }

    /// Clears the state to silence.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

/// Which output of the [`Svf`] to use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(rename_all = "camelCase"))]
pub enum SvfMode {
    /// Low pass.
    #[default]
    LowPass,
    /// Band pass.
    BandPass,
    /// High pass.
    HighPass,
    /// Notch (band reject).
    Notch,
    /// Peak: low pass minus high pass.
    Peak,
    /// All pass.
    AllPass,
}

/// Andrew Simper's linear trapezoidal state-variable filter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Svf {
    k: f32,
    a1: f32,
    a2: f32,
    a3: f32,
    ic1eq: f32,
    ic2eq: f32,
}

impl Svf {
    /// A filter at `cutoff` Hz with quality `q`, with silent state.
    pub fn new(cutoff: f64, q: f64, rate: SampleRate) -> Self {
        let mut filter = Self { k: 0.0, a1: 0.0, a2: 0.0, a3: 0.0, ic1eq: 0.0, ic2eq: 0.0 };
        filter.set(cutoff, q, rate);
        filter
    }

    /// Changes cutoff and Q and keeps the state; safe to call every sample.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn set(&mut self, cutoff: f64, q: f64, rate: SampleRate) {
        let fs = f64::from(rate.hz());
        let g = (PI * cutoff.clamp(1.0, fs * 0.499) / fs).tan();
        let k = 1.0 / q.max(0.01);
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        self.k = k as f32;
        self.a1 = a1 as f32;
        self.a2 = a2 as f32;
        self.a3 = (g * a2) as f32;
    }

    /// Filters one sample and returns the output for `mode`.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn process_sample(&mut self, x: f32, mode: SvfMode) -> f32 {
        let v3 = x - self.ic2eq;
        let v1 = self.a1 * self.ic1eq + self.a2 * v3;
        let v2 = self.ic2eq + self.a2 * self.ic1eq + self.a3 * v3;
        self.ic1eq = 2.0 * v1 - self.ic1eq;
        self.ic2eq = 2.0 * v2 - self.ic2eq;
        let (low, band) = (v2, v1);
        let high = x - self.k * band - low;
        match mode {
            SvfMode::LowPass => low,
            SvfMode::BandPass => band,
            SvfMode::HighPass => high,
            SvfMode::Notch => low + high,
            SvfMode::Peak => low - high,
            SvfMode::AllPass => low + high - self.k * band,
        }
    }

    /// Filters a block in place.
    ///
    /// RT-safe. Complexity: O(n) in the number of samples.
    #[rt_safe]
    pub fn process(&mut self, samples: &mut [f32], mode: SvfMode) {
        for sample in samples {
            *sample = self.process_sample(*sample, mode);
        }
    }

    /// Clears the state to silence.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn reset(&mut self) {
        self.ic1eq = 0.0;
        self.ic2eq = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: SampleRate = SampleRate::HZ_48000;
    const BUTTERWORTH_Q: f64 = core::f64::consts::FRAC_1_SQRT_2;

    fn db(gain: f64) -> f64 {
        20.0 * gain.log10()
    }

    #[test]
    fn low_and_high_pass_meet_at_minus_three_db() {
        let lp = BiquadCoeffs::design(BiquadKind::LowPass, 1000.0, BUTTERWORTH_Q, RATE);
        let hp = BiquadCoeffs::design(BiquadKind::HighPass, 1000.0, BUTTERWORTH_Q, RATE);
        assert!((db(lp.magnitude(1000.0, RATE)) + 3.0103).abs() < 1e-3);
        assert!((db(hp.magnitude(1000.0, RATE)) + 3.0103).abs() < 1e-3);
        assert!(db(lp.magnitude(20.0, RATE)).abs() < 1e-3);
        assert!(db(lp.magnitude(10_000.0, RATE)) < -38.0);
        assert!(db(hp.magnitude(20_000.0, RATE)).abs() < 0.1);
    }

    #[test]
    fn peak_and_shelves_reach_their_gain() {
        let peak = BiquadCoeffs::design(BiquadKind::Peak { gain_db: 6.0 }, 2000.0, 1.0, RATE);
        assert!((db(peak.magnitude(2000.0, RATE)) - 6.0).abs() < 1e-6);
        assert!(db(peak.magnitude(20.0, RATE)).abs() < 0.05);
        let low = BiquadCoeffs::design(BiquadKind::LowShelf { gain_db: -9.0 }, 200.0, BUTTERWORTH_Q, RATE);
        assert!((db(low.magnitude(10.0, RATE)) + 9.0).abs() < 0.05);
        assert!(db(low.magnitude(15_000.0, RATE)).abs() < 0.05);
        let high = BiquadCoeffs::design(BiquadKind::HighShelf { gain_db: 4.0 }, 5000.0, BUTTERWORTH_Q, RATE);
        assert!((db(high.magnitude(23_000.0, RATE)) - 4.0).abs() < 0.1);
        assert!(db(high.magnitude(20.0, RATE)).abs() < 0.01);
    }

    #[test]
    fn all_pass_and_notch() {
        let ap = BiquadCoeffs::design(BiquadKind::AllPass, 700.0, 2.0, RATE);
        for f in [50.0, 700.0, 9000.0] {
            assert!((ap.magnitude(f, RATE) - 1.0).abs() < 1e-9);
        }
        let notch = BiquadCoeffs::design(BiquadKind::Notch, 1000.0, 4.0, RATE);
        assert!(notch.magnitude(1000.0, RATE) < 1e-6);
    }

    #[test]
    fn biquad_processing_matches_the_designed_dc_gain() {
        let mut lp = Biquad::new(BiquadCoeffs::design(BiquadKind::LowPass, 500.0, BUTTERWORTH_Q, RATE));
        let mut block = [1.0f32; 4800];
        lp.process(&mut block);
        assert!((block[4799] - 1.0).abs() < 1e-4);
        lp.reset();
        assert_eq!(lp.process_sample(0.0), 0.0);
    }

    #[test]
    fn svf_low_pass_passes_dc_and_high_pass_blocks_it() {
        let mut lp = Svf::new(1000.0, BUTTERWORTH_Q, RATE);
        let mut hp = lp;
        let (mut low, mut high) = (0.0, 0.0);
        for _ in 0..4800 {
            low = lp.process_sample(1.0, SvfMode::LowPass);
            high = hp.process_sample(1.0, SvfMode::HighPass);
        }
        assert!((low - 1.0).abs() < 1e-4, "{low}");
        assert!(high.abs() < 1e-4, "{high}");
    }

    #[test]
    fn svf_stays_stable_under_fast_modulation() {
        let mut svf = Svf::new(1000.0, 10.0, RATE);
        let mut peak = 0.0f32;
        for i in 0..48_000 {
            let cutoff = 200.0 + 18_000.0 * ((i as f64 / 100.0).sin() * 0.5 + 0.5);
            svf.set(cutoff, 10.0, RATE);
            let input = if i % 64 == 0 { 1.0 } else { 0.0 };
            peak = peak.max(svf.process_sample(input, SvfMode::LowPass).abs());
        }
        assert!(peak.is_finite() && peak < 20.0, "{peak}");
    }
}
