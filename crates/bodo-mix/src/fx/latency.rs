//! Latency Test: delays the signal by a set number of samples and reports exactly that as its latency, as a plugin
//! with lookahead does. Delay compensation then lines everything else up with it, so in a compensated mix it is
//! inaudible; it exists to test and demonstrate delay compensation, and browsers hide it.

use bodo_kit_core::{SampleRate, rt_safe};
use bodo_kit_dsp::delay::DelayLine;

use super::{Category, Effect, EffectBlock, EffectInfo, ParamKind, ParamSpec, Unit};

/// The most latency it reports, in samples.
pub(crate) const MAX_SAMPLES: u32 = 8192;

const PARAMS: &[ParamSpec] = &[ParamSpec {
    id: 0,
    name: "Latency",
    unit: Unit::Samples,
    min: 0.0,
    max: MAX_SAMPLES as f32,
    default: 0.0,
    kind: ParamKind::Integer,
    tags: &["delay compensation", "test"],
}];

/// The catalog entry.
pub(super) const INFO: EffectInfo = EffectInfo {
    uid: "bodo.latency-test",
    name: "Latency Test",
    description: "Delays the signal and reports the delay as latency, to test delay compensation.",
    category: Category::Utility,
    sidechain: false,
    hidden: true,
    params: PARAMS,
    create,
    latency,
    max_latency,
};

fn create(_: SampleRate, _: usize) -> Box<dyn Effect> {
    Box::new(LatencyTest { lines: [DelayLine::new(MAX_SAMPLES as usize), DelayLine::new(MAX_SAMPLES as usize)] })
}

fn latency(params: &[f32], _: SampleRate) -> u32 {
    params.first().map_or(0, |v| v.round().clamp(0.0, MAX_SAMPLES as f32) as u32)
}

fn max_latency(_: SampleRate) -> u32 {
    MAX_SAMPLES
}

#[derive(Debug)]
struct LatencyTest {
    lines: [DelayLine; 2],
}

impl Effect for LatencyTest {
    /// RT-safe.
    #[rt_safe]
    fn process(&mut self, block: &mut EffectBlock<'_>) {
        let delay = latency(block.params, block.rate) as usize;
        let [left, right] = &mut self.lines;
        left.process(block.left, delay);
        right.process(block.right, delay);
    }

    fn reset(&mut self) {
        for line in &mut self.lines {
            line.reset();
        }
    }
}
