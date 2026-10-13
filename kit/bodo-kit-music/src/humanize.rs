//! Humanizing (project.md section 13.5): timing drifts the way a player's does, correlated from one note to the next
//! (AR(1) noise, never white noise), and velocities vary a little.

use crate::rng::Rng;

/// How strongly one note's timing follows the previous note's: 0 is independent, near 1 drifts slowly.
const TIMING_CORRELATION: f64 = 0.7;
/// Velocities follow each other less closely than timing.
const VELOCITY_CORRELATION: f64 = 0.3;
/// Draws are clamped to this many standard deviations, so no single note jumps out.
const LIMIT: f64 = 2.5;

/// Seeded humanizing: the same seed always gives the same performance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Humanizer {
    seed: u64,
}

/// An AR(1) series of `count` values with unit variance: each value is `phi` times the previous one plus fresh noise.
fn series(rng: &mut Rng, count: usize, phi: f64) -> Vec<f64> {
    let fresh = (1.0 - phi * phi).sqrt();
    let mut value = rng.gaussian().clamp(-LIMIT, LIMIT);
    (0..count)
        .map(|i| {
            if i > 0 {
                value = (phi * value + fresh * rng.gaussian()).clamp(-LIMIT, LIMIT);
            }
            value
        })
        .collect()
}

impl Humanizer {
    /// A humanizer with `seed`.
    pub const fn new(seed: u64) -> Self {
        Self { seed }
    }

    /// Timing offsets for `count` notes in time order, in units of the drift's standard deviation: multiply by how
    /// far notes should wander. Neighbors are correlated, as a player's timing is.
    ///
    /// Complexity: O(n) in notes.
    pub fn timing(&self, count: usize) -> Vec<f64> {
        series(&mut Rng::new(self.seed), count, TIMING_CORRELATION)
    }

    /// Velocity variations for `count` notes in time order, in units of their standard deviation: multiply by how
    /// much velocities should vary.
    ///
    /// Complexity: O(n) in notes.
    pub fn velocity(&self, count: usize) -> Vec<f64> {
        series(&mut Rng::new(self.seed ^ 0x5eed_ba11_7e10_c17e), count, VELOCITY_CORRELATION)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn correlation(values: &[f64]) -> f64 {
        let pairs: f64 = values.windows(2).map(|w| w[0] * w[1]).sum::<f64>() / (values.len() - 1) as f64;
        let variance: f64 = values.iter().map(|v| v * v).sum::<f64>() / values.len() as f64;
        pairs / variance
    }

    #[test]
    fn humanizing_is_seeded_bounded_and_correlated() {
        let a = Humanizer::new(3);
        assert_eq!(a.timing(50), Humanizer::new(3).timing(50));
        assert_ne!(a.timing(50), Humanizer::new(4).timing(50));
        assert_ne!(a.timing(50), a.velocity(50), "timing and velocity draw separately");
        let long = a.timing(20_000);
        assert!(long.iter().all(|v| v.abs() <= LIMIT));
        let r = correlation(&long);
        assert!((r - TIMING_CORRELATION).abs() < 0.05, "neighbors follow each other: {r}");
        let variance = long.iter().map(|v| v * v).sum::<f64>() / long.len() as f64;
        assert!((variance - 1.0).abs() < 0.1, "unit variance: {variance}");
        assert!(a.timing(0).is_empty());
    }
}
