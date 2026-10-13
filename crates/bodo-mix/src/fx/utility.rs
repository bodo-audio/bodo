//! Utility: gain, pan, width, polarity and mono. At its defaults it passes the signal through bit for bit.

use bodo_kit_core::{SampleRate, rt_safe};
use bodo_kit_dsp::gain::db_to_gain;
use bodo_kit_dsp::pan::StereoPan;
use bodo_kit_dsp::stereo::invert_polarity;

use super::{Category, Effect, EffectBlock, EffectInfo, ParamKind, ParamSpec, Unit, glide, no_latency, no_max_latency};

const GAIN: usize = 0;
const PAN: usize = 1;
const WIDTH: usize = 2;
const POLARITY: usize = 3;
const MONO: usize = 4;

const PARAMS: &[ParamSpec] = &[
    ParamSpec {
        id: 0,
        name: "Gain",
        unit: Unit::Decibels,
        min: -60.0,
        max: 24.0,
        default: 0.0,
        kind: ParamKind::Linear,
        tags: &["volume", "level", "loudness"],
    },
    ParamSpec { id: 1, name: "Pan", unit: Unit::Pan, min: -1.0, max: 1.0, default: 0.0, kind: ParamKind::Linear, tags: &["balance", "stereo"] },
    ParamSpec {
        id: 2,
        name: "Width",
        unit: Unit::Percent,
        min: 0.0,
        max: 200.0,
        default: 100.0,
        kind: ParamKind::Linear,
        tags: &["stereo", "wide", "narrow", "mid side"],
    },
    ParamSpec {
        id: 3,
        name: "Invert polarity",
        unit: Unit::None,
        min: 0.0,
        max: 1.0,
        default: 0.0,
        kind: ParamKind::Toggle,
        tags: &["phase", "polarity"],
    },
    ParamSpec {
        id: 4,
        name: "Mono",
        unit: Unit::None,
        min: 0.0,
        max: 1.0,
        default: 0.0,
        kind: ParamKind::Toggle,
        tags: &["mono", "sum", "compatibility"],
    },
];

/// The catalog entry.
pub(super) const INFO: EffectInfo = EffectInfo {
    uid: "bodo.utility",
    name: "Utility",
    description: "Sets gain, pan and stereo width, inverts polarity, or sums to mono.",
    category: Category::Utility,
    sidechain: false,
    hidden: false,
    params: PARAMS,
    create,
    latency: no_latency,
    max_latency: no_max_latency,
};

fn create(_: SampleRate, _: usize) -> Box<dyn Effect> {
    Box::new(Utility { gain: 1.0, pan: 0.0, width: 1.0 })
}

/// The running effect: the settings of the last block, glided from.
#[derive(Debug)]
struct Utility {
    gain: f32,
    pan: f32,
    width: f32,
}

impl Effect for Utility {
    /// RT-safe.
    #[rt_safe]
    fn process(&mut self, block: &mut EffectBlock<'_>) {
        let frames = block.left.len().min(block.right.len());
        let gain = db_to_gain(block.param(GAIN, 0.0));
        let pan = block.param(PAN, 0.0).clamp(-1.0, 1.0);
        let width = (block.param(WIDTH, 100.0) / 100.0).clamp(0.0, 2.0);
        if block.param(MONO, 0.0) >= 0.5 {
            for (l, r) in block.left.iter_mut().zip(block.right.iter_mut()) {
                let m = 0.5 * (*l + *r);
                *l = m;
                *r = m;
            }
        } else if width != 1.0 || self.width != 1.0 {
            let (left, right) = (&mut *block.left, &mut *block.right);
            glide(self.width, width, frames, |i, w| {
                if let (Some(l), Some(r)) = (left.get(i).copied(), right.get(i).copied()) {
                    let (m, s) = (0.5 * (l + r), 0.5 * (l - r));
                    left[i] = m + w * s;
                    right[i] = m - w * s;
                }
            });
        }
        self.width = width;
        if block.param(POLARITY, 0.0) >= 0.5 {
            invert_polarity(block.left);
            invert_polarity(block.right);
        }
        if pan != 0.0 || self.pan != 0.0 {
            let (left, right) = (&mut *block.left, &mut *block.right);
            glide(self.pan, pan, frames, |i, p| {
                let m = StereoPan::fold(p);
                if let (Some(l), Some(r)) = (left.get(i).copied(), right.get(i).copied()) {
                    left[i] = m.ll * l + m.rl * r;
                    right[i] = m.lr * l + m.rr * r;
                }
            });
        }
        self.pan = pan;
        if gain != 1.0 || self.gain != 1.0 {
            let (left, right) = (&mut *block.left, &mut *block.right);
            glide(self.gain, gain, frames, |i, g| {
                if let (Some(l), Some(r)) = (left.get_mut(i), right.get_mut(i)) {
                    *l *= g;
                    *r *= g;
                }
            });
        }
        self.gain = gain;
    }

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(effect: &mut dyn Effect, params: &[f32], left: &mut [f32], right: &mut [f32]) {
        let mut block = EffectBlock { left, right, sidechain: None, params, rate: SampleRate::HZ_48000, tempo: 120.0 };
        effect.process(&mut block);
    }

    #[test]
    fn defaults_pass_bit_exactly_and_mono_sums() {
        let mut effect = create(SampleRate::HZ_48000, 64);
        let defaults = INFO.defaults();
        let (mut l, mut r) = ([0.1f32, -0.3, 0.7], [0.2f32, 0.5, -0.9]);
        run(effect.as_mut(), &defaults, &mut l, &mut r);
        assert_eq!((l, r), ([0.1, -0.3, 0.7], [0.2, 0.5, -0.9]));
        let mut mono = defaults.clone();
        mono[MONO] = 1.0;
        run(effect.as_mut(), &mono, &mut l, &mut r);
        assert_eq!(l, r);
        let mut quiet = defaults;
        quiet[GAIN] = -6.0206;
        let (mut l, mut r) = ([1.0f32; 64], [1.0f32; 64]);
        run(effect.as_mut(), &quiet, &mut l, &mut r);
        run(effect.as_mut(), &quiet, &mut l, &mut r);
        assert!((l[63] - 0.25).abs() < 1e-3, "twice -6 dB after the glide: {}", l[63]);
    }
}
