//! Delay lines with a fixed maximum length, allocated once.
//!
//! The mixer uses integer delays for plugin delay compensation, so they must be exact: a delay of `d` samples
//! returns each input sample exactly `d` samples later, bit for bit.

use bodo_kit_core::rt_safe;

/// A mono delay line that can delay by any whole number of samples up to its maximum.
#[derive(Clone, Debug)]
pub struct DelayLine {
    buffer: Vec<f32>,
    mask: usize,
    write: usize,
    max_delay: usize,
}

impl DelayLine {
    /// A silent delay line that can delay by up to `max_delay` samples.
    ///
    /// Allocates. Complexity: O(n) in `max_delay`.
    pub fn new(max_delay: usize) -> Self {
        let size = (max_delay + 1).next_power_of_two();
        Self { buffer: vec![0.0; size], mask: size - 1, write: 0, max_delay }
    }

    /// The longest delay this line supports.
    pub fn max_delay(&self) -> usize {
        self.max_delay
    }

    /// Delays `samples` in place by `delay` samples, clamped to the maximum. A delay of zero passes the samples
    /// through while still recording them, so the delay can grow later without a gap.
    ///
    /// RT-safe. Complexity: O(n) in the number of samples.
    #[rt_safe]
    pub fn process(&mut self, samples: &mut [f32], delay: usize) {
        let delay = delay.min(self.max_delay);
        for sample in samples {
            if let Some(slot) = self.buffer.get_mut(self.write) {
                *slot = *sample;
            }
            let read = self.write.wrapping_sub(delay) & self.mask;
            *sample = self.buffer.get(read).copied().unwrap_or(0.0);
            self.write = (self.write + 1) & self.mask;
        }
    }

    /// Clears the line to silence.
    ///
    /// RT-safe. Complexity: O(n) in the line's length.
    #[rt_safe]
    pub fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.write = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delays_exactly() {
        let mut line = DelayLine::new(5);
        let mut block: Vec<f32> = (1..=8).map(|i| i as f32).collect();
        line.process(&mut block, 3);
        assert_eq!(block, [0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 4.0, 5.0]);
        let mut next = [9.0f32, 10.0];
        line.process(&mut next, 3);
        assert_eq!(next, [6.0, 7.0]);
    }

    #[test]
    fn zero_delay_passes_through_and_delay_is_clamped() {
        let mut line = DelayLine::new(2);
        let mut block = [1.0f32, 2.0, 3.0];
        line.process(&mut block, 0);
        assert_eq!(block, [1.0, 2.0, 3.0]);
        let mut block = [4.0f32, 5.0];
        line.process(&mut block, 100);
        assert_eq!(block, [2.0, 3.0], "clamped to the maximum of 2, continuing from the earlier input");
    }

    #[test]
    fn reset_silences() {
        let mut line = DelayLine::new(4);
        let mut block = [1.0f32; 4];
        line.process(&mut block, 2);
        line.reset();
        let mut block = [0.0f32; 2];
        line.process(&mut block, 2);
        assert_eq!(block, [0.0, 0.0]);
    }
}
