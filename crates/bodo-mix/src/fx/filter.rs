//! Filter: a state-variable low, high or band pass with resonance. The cutoff glides across each block, so sweeping
//! it by hand or by automation never zips.

use bodo_kit_core::{SampleRate, rt_safe};
use bodo_kit_dsp::filter::{Svf, SvfMode};

use super::{Category, Effect, EffectBlock, EffectInfo, ParamKind, ParamSpec, Unit, no_latency, no_max_latency};

const MODE: usize = 0;
const CUTOFF: usize = 1;
const RESONANCE: usize = 2;

const PARAMS: &[ParamSpec] = &[
    ParamSpec {
        id: 0,
        name: "Mode",
        unit: Unit::None,
        min: 0.0,
        max: 2.0,
        default: 0.0,
        kind: ParamKind::Choice { choices: &["Low pass", "High pass", "Band pass"] },
        tags: &["filter type"],
    },
    ParamSpec {
        id: 1,
        name: "Cutoff",
        unit: Unit::Hertz,
        min: 20.0,
        max: 20_000.0,
        default: 1000.0,
        kind: ParamKind::Logarithmic,
        tags: &["brightness", "frequency", "tone", "sweep"],
    },
    ParamSpec {
        id: 2,
        name: "Resonance",
        unit: Unit::Percent,
        min: 0.0,
        max: 100.0,
        default: 20.0,
        kind: ParamKind::Linear,
        tags: &["peak", "emphasis", "q"],
    },
];

/// The catalog entry.
pub(super) const INFO: EffectInfo = EffectInfo {
    uid: "bodo.filter",
    name: "Filter",
    description: "Low, high or band pass filter with resonance, for sweeps and tone shaping.",
    category: Category::Filter,
    sidechain: false,
    hidden: false,
    params: PARAMS,
    create,
    latency: no_latency,
    max_latency: no_max_latency,
};

fn create(rate: SampleRate, _: usize) -> Box<dyn Effect> {
    let svf = Svf::new(1000.0, q_of(20.0), rate);
    Box::new(Filter { left: svf, right: svf, cutoff: 1000.0 })
}

/// Q for a resonance in percent: 0.5 (no peak) to 10 (a sharp one).
fn q_of(resonance: f32) -> f64 {
    0.5 + f64::from(resonance.clamp(0.0, 100.0)) / 100.0 * 9.5
}

/// The running filter: one per channel, and the cutoff of the last block.
#[derive(Debug)]
struct Filter {
    left: Svf,
    right: Svf,
    cutoff: f32,
}

impl Effect for Filter {
    /// RT-safe.
    #[rt_safe]
    fn process(&mut self, block: &mut EffectBlock<'_>) {
        let mode = match block.param(MODE, 0.0).round() as i32 {
            1 => SvfMode::HighPass,
            2 => SvfMode::BandPass,
            _ => SvfMode::LowPass,
        };
        let cutoff = block.param(CUTOFF, 1000.0).clamp(20.0, 20_000.0);
        let q = q_of(block.param(RESONANCE, 20.0));
        let frames = block.left.len().min(block.right.len()).max(1);
        // Glide geometrically, as the ear hears pitch.
        let ratio = (cutoff / self.cutoff.max(1.0)).powf(1.0 / frames as f32);
        let mut current = self.cutoff.max(1.0);
        for (l, r) in block.left.iter_mut().zip(block.right.iter_mut()) {
            if ratio != 1.0 {
                current *= ratio;
                self.left.set(f64::from(current), q, block.rate);
                self.right.set(f64::from(current), q, block.rate);
            }
            *l = self.left.process_sample(*l, mode);
            *r = self.right.process_sample(*r, mode);
        }
        self.left.set(f64::from(cutoff), q, block.rate);
        self.right.set(f64::from(cutoff), q, block.rate);
        self.cutoff = cutoff;
    }

    fn reset(&mut self) {
        self.left.reset();
        self.right.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RMS of a sine at `frequency` after the filter with `params`.
    fn level(params: &[f32], frequency: f32) -> f32 {
        let rate = SampleRate::HZ_48000;
        let mut effect = create(rate, 512);
        let mut sum = 0.0;
        let mut count = 0;
        for block in 0..40 {
            let mut l: Vec<f32> = (0..512).map(|i| (2.0 * core::f32::consts::PI * frequency * (block * 512 + i) as f32 / 48_000.0).sin()).collect();
            let mut r = l.clone();
            let mut b = EffectBlock { left: &mut l, right: &mut r, sidechain: None, params, rate, tempo: 120.0 };
            effect.process(&mut b);
            if block >= 20 {
                sum += l.iter().map(|s| s * s).sum::<f32>();
                count += 512;
            }
        }
        (sum / count as f32).sqrt()
    }

    #[test]
    fn low_pass_keeps_lows_and_high_pass_keeps_highs() {
        let low = [0.0, 500.0, 0.0];
        assert!(level(&low, 100.0) > 0.6);
        assert!(level(&low, 8000.0) < 0.05);
        let high = [1.0, 500.0, 0.0];
        assert!(level(&high, 100.0) < 0.05);
        assert!(level(&high, 8000.0) > 0.6);
    }
}
