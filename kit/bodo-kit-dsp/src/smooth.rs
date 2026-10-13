//! Parameter smoothing, so that knob and fader changes never click.
//!
//! - [`LinearSmoother`] ramps to a new target in a fixed number of samples and lands on it exactly. Use it where the
//!   end point matters, such as faders and mute ramps.
//! - [`OnePole`] follows its target exponentially, which sounds natural for continuous controls such as cutoff.

use bodo_kit_core::{SampleRate, Seconds, rt_safe};

/// Ramps linearly to each new target over a fixed time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinearSmoother {
    current: f32,
    target: f32,
    step: f32,
    remaining: u32,
    ramp_samples: u32,
}

impl LinearSmoother {
    /// A smoother resting at `value` that takes `ramp_samples` samples to reach each new target. Zero jumps.
    pub fn new(value: f32, ramp_samples: u32) -> Self {
        Self { current: value, target: value, step: 0.0, remaining: 0, ramp_samples }
    }

    /// A smoother whose ramps last `time` at `rate`.
    pub fn with_time(value: f32, time: Seconds, rate: SampleRate) -> Self {
        Self::new(value, ramp_samples(time, rate))
    }

    /// Starts a ramp from the current value to `target`.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn set_target(&mut self, target: f32) {
        if target == self.target {
            return;
        }
        self.target = target;
        if self.ramp_samples == 0 {
            self.current = target;
            self.remaining = 0;
        } else {
            self.remaining = self.ramp_samples;
            self.step = (target - self.current) / self.ramp_samples as f32;
        }
    }

    /// Jumps to `value` at once.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn reset(&mut self, value: f32) {
        self.current = value;
        self.target = value;
        self.remaining = 0;
    }

    /// Advances one sample and returns the new value.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn next_value(&mut self) -> f32 {
        if self.remaining > 0 {
            self.remaining -= 1;
            self.current = if self.remaining == 0 { self.target } else { self.current + self.step };
        }
        self.current
    }

    /// Fills `out` with the next values.
    ///
    /// RT-safe. Complexity: O(n) in the length of `out`.
    #[rt_safe]
    pub fn fill(&mut self, out: &mut [f32]) {
        if self.remaining == 0 {
            out.fill(self.current);
            return;
        }
        for value in out {
            *value = self.next_value();
        }
    }

    /// The value now.
    pub fn current(&self) -> f32 {
        self.current
    }

    /// The value the ramp is heading to.
    pub fn target(&self) -> f32 {
        self.target
    }

    /// Whether the value has reached its target.
    pub fn is_settled(&self) -> bool {
        self.remaining == 0
    }
}

/// Follows its target exponentially: each sample moves a fixed fraction of the remaining distance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OnePole {
    current: f32,
    target: f32,
    coefficient: f32,
}

impl OnePole {
    /// The remaining distance below which the value snaps onto the target.
    const SNAP: f32 = 1e-6;

    /// A smoother resting at `value` with time constant `time`: after `time`, about 63% of a step is covered.
    pub fn new(value: f32, time: Seconds, rate: SampleRate) -> Self {
        let samples = time.0 * f64::from(rate.hz());
        let coefficient = if samples <= 0.0 { 1.0 } else { 1.0 - (-1.0 / samples).exp() };
        Self { current: value, target: value, coefficient: coefficient as f32 }
    }

    /// Sets the value to follow.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn set_target(&mut self, target: f32) {
        self.target = target;
    }

    /// Advances one sample and returns the new value.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn next_value(&mut self) -> f32 {
        let distance = self.target - self.current;
        let next = self.current + distance * self.coefficient;
        // Snap when close, or when the step has become too small for f32 to move the value at all.
        self.current = if distance.abs() < Self::SNAP || next == self.current { self.target } else { next };
        self.current
    }

    /// The value now.
    pub fn current(&self) -> f32 {
        self.current
    }
}

/// The number of samples in `time` at `rate`, rounded and clamped to the `u32` range.
fn ramp_samples(time: Seconds, rate: SampleRate) -> u32 {
    (time.0 * f64::from(rate.hz())).round().clamp(0.0, f64::from(u32::MAX)) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_ramp_lands_exactly_on_target() {
        let mut s = LinearSmoother::new(0.0, 3);
        s.set_target(1.0);
        let values: Vec<f32> = (0..5).map(|_| s.next_value()).collect();
        assert_eq!(values[2], 1.0);
        assert_eq!(values[4], 1.0);
        assert!(s.is_settled());
        assert!((values[0] - 1.0 / 3.0).abs() < 1e-7);
    }

    #[test]
    fn retargeting_mid_ramp_starts_from_the_current_value() {
        let mut s = LinearSmoother::new(0.0, 4);
        s.set_target(1.0);
        s.next_value();
        s.next_value();
        s.set_target(0.0);
        let mut out = [0.0f32; 4];
        s.fill(&mut out);
        assert_eq!(out[3], 0.0);
        assert!(out[0] < 0.5 && out[0] > 0.0);
    }

    #[test]
    fn zero_length_ramp_jumps() {
        let mut s = LinearSmoother::with_time(0.5, Seconds(0.0), SampleRate::HZ_48000);
        s.set_target(0.25);
        assert_eq!(s.current(), 0.25);
    }

    #[test]
    fn one_pole_converges_and_snaps() {
        let mut s = OnePole::new(0.0, Seconds(0.001), SampleRate::HZ_48000);
        s.set_target(1.0);
        let after_time_constant = (0..48).map(|_| s.next_value()).last().unwrap();
        assert!((after_time_constant - 0.632).abs() < 0.01, "{after_time_constant}");
        for _ in 0..10_000 {
            s.next_value();
        }
        assert_eq!(s.current(), 1.0);
    }
}
