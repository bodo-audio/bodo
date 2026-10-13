//! Checks that the per-block DSP functions never allocate. Debug builds only.

#![cfg(debug_assertions)]

use assert_no_alloc::{AllocDisabler, assert_no_alloc, reset_violation_count, violation_count};
use bodo_kit_core::{SampleRate, Seconds};
use bodo_kit_dsp::AudioBuffer;
use bodo_kit_dsp::delay::DelayLine;
use bodo_kit_dsp::envelope::{Adsr, AdsrParams};
use bodo_kit_dsp::filter::{Biquad, BiquadCoeffs, BiquadKind, Svf, SvfMode};
use bodo_kit_dsp::gain::{apply_gain, mix_into, ramp_gain};
use bodo_kit_dsp::pan::{PanLaw, StereoPan};
use bodo_kit_dsp::smooth::LinearSmoother;
use bodo_kit_dsp::stereo::{invert_polarity, stereo_separation, swap_channels};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

#[test]
fn block_processing_does_not_allocate() {
    let rate = SampleRate::HZ_48000;
    let mut buffer = AudioBuffer::new(2, 512);
    let mut scratch = AudioBuffer::new(2, 512);
    let mut delay = DelayLine::new(1024);
    let mut biquad = Biquad::new(BiquadCoeffs::design(BiquadKind::LowPass, 1000.0, 0.7, rate));
    let mut svf = Svf::new(1000.0, 0.7, rate);
    let mut env = Adsr::new(AdsrParams::default(), rate);
    let mut smoother = LinearSmoother::with_time(0.0, Seconds(0.01), rate);
    let mut ramp = [0.0f32; 512];
    reset_violation_count();
    assert_no_alloc(|| {
        buffer.set_frames(256);
        env.gate_on();
        smoother.set_target(1.0);
        for _ in 0..8 {
            if let Some((left, right)) = buffer.stereo_mut() {
                left.fill(0.5);
                right.fill(-0.25);
                StereoPan::fold(0.3).process(left, right);
                stereo_separation(left, right, 0.4);
                swap_channels(left, right);
                invert_polarity(left);
                let g = PanLaw::ConstantPower3dB.mono_gains(0.2);
                apply_gain(left, g.left);
                ramp_gain(right, 0.0, g.right);
                delay.process(left, 100);
                biquad.process(left);
                svf.process(right, SvfMode::BandPass);
                env.apply(right);
            }
            smoother.fill(&mut ramp[..256]);
            scratch.copy_from(&buffer);
            mix_into(buffer.channel_mut(0), scratch.channel(1), 0.5);
            buffer.clear();
        }
    });
    assert_eq!(violation_count(), 0);
}
