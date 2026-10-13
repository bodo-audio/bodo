//! Timing helpers for the piano roll (project.md section 10.4): quantize, strums and arpeggio orders.

use bodo_kit_core::Tick;

use crate::rng::Rng;

/// Moves `start` toward the nearest line of a `grid`-tick grid by `strength` (0 leaves it, 1 lands on the line).
/// `swing` (0 to 1) moves every second grid line later by up to half a grid, for a shuffle. A grid of zero or less
/// leaves `start` unchanged.
pub fn quantize(start: Tick, grid: Tick, strength: f64, swing: f64) -> Tick {
    if grid.0 <= 0 {
        return start;
    }
    let line = (start.0 as f64 / grid.0 as f64).round() as i64;
    let mut target = line * grid.0;
    if line.rem_euclid(2) == 1 {
        target += (swing.clamp(0.0, 1.0) * grid.0 as f64 / 2.0).round() as i64;
    }
    let strength = strength.clamp(0.0, 1.0);
    Tick(start.0 + ((target - start.0) as f64 * strength).round() as i64)
}

/// Which way a strum or an arpeggio runs through a chord.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Direction {
    /// From the lowest key up.
    #[default]
    Up,
    /// From the highest key down.
    Down,
}

/// How far each of `voices` voices, sorted from the lowest key up, is delayed in a strum that rolls by `per_voice`
/// ticks per voice.
///
/// Complexity: O(n) in voices.
pub fn strum_offsets(voices: usize, per_voice: Tick, direction: Direction) -> Vec<Tick> {
    (0..voices)
        .map(|i| {
            let place = match direction {
                Direction::Up => i,
                Direction::Down => voices - 1 - i,
            };
            Tick(place as i64 * per_voice.0)
        })
        .collect()
}

/// The order an arpeggio plays a chord's voices in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ArpPattern {
    /// Lowest to highest, again and again.
    #[default]
    Up,
    /// Highest to lowest.
    Down,
    /// Up then down, without repeating the top and bottom.
    UpDown,
    /// Down then up, without repeating the top and bottom.
    DownUp,
    /// A random voice each step, never the same twice in a row.
    Random,
    /// In the order the voices were played.
    AsPlayed,
}

/// The voice index of each of `steps` arpeggio steps over `voices` voices. For every pattern but
/// [`ArpPattern::AsPlayed`] the voices are sorted from the lowest key up; for `AsPlayed` they are in the order played.
/// `seed` drives [`ArpPattern::Random`].
///
/// Complexity: O(steps).
pub fn arp_order(voices: usize, pattern: ArpPattern, steps: usize, seed: u64) -> Vec<usize> {
    if voices == 0 {
        return Vec::new();
    }
    let cycle: Vec<usize> = match pattern {
        ArpPattern::Up | ArpPattern::AsPlayed => (0..voices).collect(),
        ArpPattern::Down => (0..voices).rev().collect(),
        ArpPattern::UpDown => (0..voices).chain((1..voices.saturating_sub(1)).rev()).collect(),
        ArpPattern::DownUp => (0..voices).rev().chain(1..voices.saturating_sub(1)).collect(),
        ArpPattern::Random => {
            let mut rng = Rng::new(seed);
            let mut last = None;
            return (0..steps)
                .map(|_| {
                    let mut pick = rng.below(voices);
                    if voices > 1 && Some(pick) == last {
                        pick = (pick + 1 + rng.below(voices - 1)) % voices;
                    }
                    last = Some(pick);
                    pick
                })
                .collect();
        }
    };
    (0..steps).map(|i| cycle[i % cycle.len()]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantize_moves_by_strength_and_swings_off_beats() {
        let grid = Tick(240);
        assert_eq!(quantize(Tick(250), grid, 1.0, 0.0), Tick(240));
        assert_eq!(quantize(Tick(300), grid, 0.5, 0.0), Tick(270), "half the way from 300 to 240");
        assert_eq!(quantize(Tick(300), grid, 0.0, 0.0), Tick(300));
        assert_eq!(quantize(Tick(250), grid, 1.0, 1.0), Tick(360), "the second line swings half a grid later");
        assert_eq!(quantize(Tick(470), grid, 1.0, 1.0), Tick(480), "even lines don't swing");
        assert_eq!(quantize(Tick(-130), grid, 1.0, 0.0), Tick(-240));
        assert_eq!(quantize(Tick(5), Tick(0), 1.0, 0.0), Tick(5));
    }

    #[test]
    fn strums_roll_up_or_down() {
        assert_eq!(strum_offsets(3, Tick(20), Direction::Up), [Tick(0), Tick(20), Tick(40)]);
        assert_eq!(strum_offsets(3, Tick(20), Direction::Down), [Tick(40), Tick(20), Tick(0)]);
        assert!(strum_offsets(0, Tick(20), Direction::Up).is_empty());
    }

    #[test]
    fn arpeggio_orders() {
        assert_eq!(arp_order(3, ArpPattern::Up, 7, 0), [0, 1, 2, 0, 1, 2, 0]);
        assert_eq!(arp_order(3, ArpPattern::Down, 4, 0), [2, 1, 0, 2]);
        assert_eq!(arp_order(4, ArpPattern::UpDown, 8, 0), [0, 1, 2, 3, 2, 1, 0, 1]);
        assert_eq!(arp_order(3, ArpPattern::DownUp, 6, 0), [2, 1, 0, 1, 2, 1]);
        assert_eq!(arp_order(1, ArpPattern::UpDown, 3, 0), [0, 0, 0]);
        assert_eq!(arp_order(2, ArpPattern::AsPlayed, 3, 0), [0, 1, 0]);
        let random = arp_order(4, ArpPattern::Random, 64, 9);
        assert_eq!(random, arp_order(4, ArpPattern::Random, 64, 9), "the same seed plays the same");
        assert!(random.windows(2).all(|w| w[0] != w[1]), "never the same voice twice in a row");
        assert!(random.iter().all(|v| *v < 4));
        assert!(arp_order(0, ArpPattern::Up, 4, 0).is_empty());
    }
}
