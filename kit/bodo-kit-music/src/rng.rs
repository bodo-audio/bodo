//! A small seeded random number generator (xorshift64*), so humanizing and random arpeggios are reproducible.

/// A deterministic generator: the same seed always gives the same sequence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rng {
    state: u64,
}

impl Rng {
    /// A generator seeded with `seed`. Seeds that differ in any bit give unrelated sequences.
    pub fn new(seed: u64) -> Self {
        // SplitMix64 spreads the seed, so small seeds such as 1 and 2 don't start alike, and the state is never zero.
        let mut z = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        Self { state: if z == 0 { 0x2545_f491_4f6c_dd1d } else { z } }
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// A number in `0.0..1.0`.
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// A number in `0..n`, or 0 when `n` is 0.
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next_u64() % n as u64) as usize }
    }

    /// A normally distributed number with mean 0 and standard deviation 1 (Box-Muller).
    pub fn gaussian(&mut self) -> f64 {
        let u1 = self.next_f64().max(f64::MIN_POSITIVE);
        let u2 = self.next_f64();
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeds_repeat_and_differ() {
        let mut a = Rng::new(1);
        let mut b = Rng::new(1);
        let mut c = Rng::new(2);
        let first: Vec<u64> = (0..8).map(|_| a.next_u64()).collect();
        assert_eq!(first, (0..8).map(|_| b.next_u64()).collect::<Vec<_>>());
        assert_ne!(first, (0..8).map(|_| c.next_u64()).collect::<Vec<_>>());
        let mut r = Rng::new(7);
        assert!((0..1000).all(|_| (0.0..1.0).contains(&r.next_f64())));
        assert!((0..1000).all(|_| r.below(5) < 5));
        let mean = (0..20_000).map(|_| r.gaussian()).sum::<f64>() / 20_000.0;
        assert!(mean.abs() < 0.05, "{mean}");
    }
}
