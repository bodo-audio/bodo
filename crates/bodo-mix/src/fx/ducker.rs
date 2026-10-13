//! Ducker: turns the signal down while its sidechain is loud, the classic kick-pumps-the-bass sound. The level of the
//! sidechain above the threshold, in dB, is taken off the signal, up to the depth, with attack and release times.
//! Without a sidechain it passes the signal through untouched.

use bodo_kit_core::{SampleRate, rt_safe};
use bodo_kit_dsp::gain::db_to_gain;

use super::{Category, Effect, EffectBlock, EffectInfo, ParamKind, ParamSpec, Unit, no_latency, no_max_latency};

const DEPTH: usize = 0;
const THRESHOLD: usize = 1;
const ATTACK: usize = 2;
const RELEASE: usize = 3;

const PARAMS: &[ParamSpec] = &[
    ParamSpec {
        id: 0,
        name: "Depth",
        unit: Unit::Decibels,
        min: 0.0,
        max: 48.0,
        default: 12.0,
        kind: ParamKind::Linear,
        tags: &["amount", "pumping", "sidechain"],
    },
    ParamSpec {
        id: 1,
        name: "Threshold",
        unit: Unit::Decibels,
        min: -60.0,
        max: 0.0,
        default: -30.0,
        kind: ParamKind::Linear,
        tags: &["sensitivity", "trigger"],
    },
    ParamSpec {
        id: 2,
        name: "Attack",
        unit: Unit::Milliseconds,
        min: 0.1,
        max: 100.0,
        default: 5.0,
        kind: ParamKind::Logarithmic,
        tags: &["speed", "response"],
    },
    ParamSpec {
        id: 3,
        name: "Release",
        unit: Unit::Milliseconds,
        min: 10.0,
        max: 1000.0,
        default: 150.0,
        kind: ParamKind::Logarithmic,
        tags: &["recovery", "groove"],
    },
];

/// The catalog entry.
pub(super) const INFO: EffectInfo = EffectInfo {
    uid: "bodo.ducker",
    name: "Ducker",
    description: "Lowers the signal while the sidechain plays, for sidechain pumping.",
    category: Category::Dynamics,
    sidechain: true,
    hidden: false,
    params: PARAMS,
    create,
    latency: no_latency,
    max_latency: no_max_latency,
};

fn create(_: SampleRate, _: usize) -> Box<dyn Effect> {
    Box::new(Ducker { envelope: 0.0, reduction_db: 0.0 })
}

/// The running ducker: the sidechain's envelope and the reduction it gives.
#[derive(Debug)]
struct Ducker {
    envelope: f32,
    reduction_db: f32,
}

/// The one-pole coefficient for a time constant of `ms` milliseconds.
fn coefficient(ms: f32, rate: SampleRate) -> f32 {
    let samples = (ms.max(0.01) * 0.001 * rate.hz() as f32).max(1.0);
    1.0 - (-1.0 / samples).exp()
}

impl Effect for Ducker {
    /// RT-safe.
    #[rt_safe]
    fn process(&mut self, block: &mut EffectBlock<'_>) {
        let Some((side_l, side_r)) = block.sidechain else {
            self.envelope = 0.0;
            self.reduction_db = 0.0;
            return;
        };
        let depth = block.param(DEPTH, 12.0).clamp(0.0, 48.0);
        let threshold = block.param(THRESHOLD, -30.0);
        let attack = coefficient(block.param(ATTACK, 5.0), block.rate);
        let release = coefficient(block.param(RELEASE, 150.0), block.rate);
        for (i, (l, r)) in block.left.iter_mut().zip(block.right.iter_mut()).enumerate() {
            let level = side_l.get(i).map_or(0.0, |s| s.abs()).max(side_r.get(i).map_or(0.0, |s| s.abs()));
            let rate = if level > self.envelope { attack } else { release };
            self.envelope += rate * (level - self.envelope);
            let level_db = 20.0 * self.envelope.max(1e-9).log10();
            let target = (level_db - threshold).clamp(0.0, depth);
            // The reduction follows the envelope, so it already has the attack and release.
            self.reduction_db = target;
            let gain = db_to_gain(-self.reduction_db);
            *l *= gain;
            *r *= gain;
        }
    }

    fn reset(&mut self) {
        self.envelope = 0.0;
        self.reduction_db = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_loud_sidechain_ducks_by_the_depth_and_silence_lets_go() {
        let rate = SampleRate::HZ_48000;
        let mut effect = create(rate, 512);
        let params = INFO.defaults();
        let loud = vec![0.9f32; 512];
        let quiet = vec![0.0f32; 512];
        let mut run = |side: &[f32]| {
            let (mut l, mut r) = (vec![1.0f32; 512], vec![1.0f32; 512]);
            let mut b = EffectBlock { left: &mut l, right: &mut r, sidechain: Some((side, side)), params: &params, rate, tempo: 120.0 };
            effect.process(&mut b);
            l[511]
        };
        for _ in 0..4 {
            run(&loud);
        }
        assert!((run(&loud) - db_to_gain(-12.0)).abs() < 1e-3, "full depth");
        for _ in 0..60 {
            run(&quiet);
        }
        assert!(run(&quiet) > 0.99, "released");
    }
}
