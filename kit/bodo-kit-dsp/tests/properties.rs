//! Property tests for the DSP building blocks.

#![allow(clippy::unwrap_used, reason = "test helpers outside #[test] functions")]

use bodo_kit_core::SampleRate;
use bodo_kit_dsp::delay::DelayLine;
use bodo_kit_dsp::filter::{BiquadCoeffs, BiquadKind};
use bodo_kit_dsp::gain::{db_to_gain, gain_to_db};
use bodo_kit_dsp::pan::{PanLaw, StereoPan};
use bodo_kit_dsp::stereo::stereo_separation;
use proptest::prelude::*;

fn rate() -> impl Strategy<Value = SampleRate> {
    prop_oneof![Just(44_100u32), Just(48_000), Just(96_000), Just(192_000)].prop_map(|hz| SampleRate::new(hz).unwrap())
}

fn kind() -> impl Strategy<Value = BiquadKind> {
    let gain = -24.0..24.0f64;
    prop_oneof![
        Just(BiquadKind::LowPass),
        Just(BiquadKind::HighPass),
        Just(BiquadKind::BandPass),
        Just(BiquadKind::Notch),
        Just(BiquadKind::AllPass),
        gain.clone().prop_map(|gain_db| BiquadKind::Peak { gain_db }),
        gain.clone().prop_map(|gain_db| BiquadKind::LowShelf { gain_db }),
        gain.prop_map(|gain_db| BiquadKind::HighShelf { gain_db }),
    ]
}

proptest! {
    #[test]
    fn decibels_round_trip(db in -120.0f32..24.0) {
        prop_assert!((gain_to_db(db_to_gain(db)) - db).abs() < 1e-3);
    }

    #[test]
    fn mono_pan_gains_stay_in_range(pan in -2.0f32..2.0) {
        for law in [PanLaw::ConstantPower3dB, PanLaw::Compromise4_5dB, PanLaw::Linear6dB] {
            let g = law.mono_gains(pan);
            prop_assert!((0.0..=1.0).contains(&g.left) && (0.0..=1.0).contains(&g.right));
        }
    }

    #[test]
    fn stereo_fold_keeps_crossfade_power(pan in -1.0f32..1.0) {
        let m = StereoPan::fold(pan);
        let (stay, cross) = if pan >= 0.0 { (m.ll, m.lr) } else { (m.rr, m.rl) };
        prop_assert!((stay * stay + cross * cross - 1.0).abs() < 1e-5);
    }

    #[test]
    fn separation_preserves_the_mid_signal(l in -1.0f32..1.0, r in -1.0f32..1.0, sep in -1.0f32..1.0) {
        let (mut left, mut right) = ([l], [r]);
        stereo_separation(&mut left, &mut right, sep);
        prop_assert!(((left[0] + right[0]) - (l + r)).abs() < 1e-5);
    }

    #[test]
    fn delay_is_exact(delay in 0usize..64, block in 1usize..37, input in prop::collection::vec(-1.0f32..1.0, 1..400)) {
        let mut line = DelayLine::new(64);
        let mut output = Vec::with_capacity(input.len());
        for chunk in input.chunks(block) {
            let mut chunk = chunk.to_vec();
            line.process(&mut chunk, delay);
            output.extend(chunk);
        }
        for (i, sample) in output.iter().enumerate() {
            let expected = if i >= delay { input[i - delay] } else { 0.0 };
            prop_assert_eq!(*sample, expected);
        }
    }

    #[test]
    fn designed_biquads_are_stable(kind in kind(), freq in 10.0f64..30_000.0, q in 0.1f64..20.0, rate in rate()) {
        let c = BiquadCoeffs::design(kind, freq, q, rate);
        // Both poles lie inside the unit circle (the stability triangle).
        prop_assert!(c.a2.abs() < 1.0, "{c:?}");
        prop_assert!(c.a1.abs() < 1.0 + c.a2, "{c:?}");
    }
}
