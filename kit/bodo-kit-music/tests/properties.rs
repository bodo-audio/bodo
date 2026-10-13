//! Property tests of the timing helpers.

use bodo_kit_core::Tick;
use bodo_kit_music::{Humanizer, Scale, ScaleKind, quantize};
use proptest::prelude::*;

proptest! {
    #[test]
    fn full_strength_lands_on_the_grid(start in -100_000i64..100_000, grid in 1i64..4000) {
        let q = quantize(Tick(start), Tick(grid), 1.0, 0.0);
        prop_assert_eq!(q.0.rem_euclid(grid), 0);
        prop_assert!((q.0 - start).abs() * 2 <= grid);
    }

    #[test]
    fn zero_strength_changes_nothing(start in -100_000i64..100_000, grid in 1i64..4000, swing in 0.0f64..1.0) {
        prop_assert_eq!(quantize(Tick(start), Tick(grid), 0.0, swing), Tick(start));
    }

    #[test]
    fn partial_strength_moves_part_of_the_way(start in 0i64..100_000, grid in 2i64..4000, strength in 0.0f64..1.0) {
        let full = quantize(Tick(start), Tick(grid), 1.0, 0.0).0;
        let part = quantize(Tick(start), Tick(grid), strength, 0.0).0;
        prop_assert!((part - start).abs() <= (full - start).abs());
        prop_assert!((part - full).abs() <= (full - start).abs());
    }

    #[test]
    fn humanizing_repeats_for_a_seed(seed in any::<u64>(), count in 0usize..200) {
        let h = Humanizer::new(seed);
        prop_assert_eq!(h.timing(count), Humanizer::new(seed).timing(count));
        prop_assert_eq!(h.velocity(count), Humanizer::new(seed).velocity(count));
    }

    #[test]
    fn snapped_keys_are_in_the_scale(key in 0u8..=127, root in 0u8..12, kind in 0usize..14) {
        let scale = Scale::new(bodo_kit_music::PitchClass::new(root), ScaleKind::ALL[kind]);
        let snapped = scale.snap(key);
        prop_assert!(scale.contains(snapped));
        prop_assert!(snapped.abs_diff(key) <= 6);
    }
}
