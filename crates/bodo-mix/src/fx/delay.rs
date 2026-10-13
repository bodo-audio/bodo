//! Delay: a tempo-synced feedback echo with a high cut in the feedback path, optionally ping-ponging between the
//! sides. When the time or the tempo changes, the delay glides to the new time like tape instead of clicking.

use bodo_kit_core::{SampleRate, rt_safe};

use super::{Category, Effect, EffectBlock, EffectInfo, ParamKind, ParamSpec, Unit, no_latency, no_max_latency};

const TIME: usize = 0;
const FEEDBACK: usize = 1;
const MIX: usize = 2;
const PING_PONG: usize = 3;
const HIGH_CUT: usize = 4;

/// The delay times on offer, in beats.
const BEATS: [f64; 9] = [0.25, 1.0 / 3.0, 0.5, 0.75, 2.0 / 3.0, 1.0, 1.5, 2.0, 4.0];

/// The longest echo the buffer holds, in seconds: four beats at 60 BPM.
const MAX_SECONDS: f64 = 4.0;

const PARAMS: &[ParamSpec] = &[
    ParamSpec {
        id: 0,
        name: "Time",
        unit: Unit::None,
        min: 0.0,
        max: 8.0,
        default: 5.0,
        kind: ParamKind::Choice { choices: &["1/16", "1/8 triplet", "1/8", "1/8 dotted", "1/4 triplet", "1/4", "1/4 dotted", "1/2", "1 bar"] },
        tags: &["time", "rhythm", "sync"],
    },
    ParamSpec {
        id: 1,
        name: "Feedback",
        unit: Unit::Percent,
        min: 0.0,
        max: 95.0,
        default: 35.0,
        kind: ParamKind::Linear,
        tags: &["repeats", "decay"],
    },
    ParamSpec { id: 2, name: "Mix", unit: Unit::Percent, min: 0.0, max: 100.0, default: 30.0, kind: ParamKind::Linear, tags: &["wet", "amount"] },
    ParamSpec { id: 3, name: "Ping-pong", unit: Unit::None, min: 0.0, max: 1.0, default: 0.0, kind: ParamKind::Toggle, tags: &["stereo", "bounce"] },
    ParamSpec {
        id: 4,
        name: "High cut",
        unit: Unit::Hertz,
        min: 500.0,
        max: 20_000.0,
        default: 8000.0,
        kind: ParamKind::Logarithmic,
        tags: &["darkness", "tone", "warmth"],
    },
];

/// The catalog entry.
pub(super) const INFO: EffectInfo = EffectInfo {
    uid: "bodo.delay",
    name: "Delay",
    description: "Tempo-synced echo with feedback, a high cut and optional ping-pong.",
    category: Category::Delay,
    sidechain: false,
    hidden: false,
    params: PARAMS,
    create,
    latency: no_latency,
    max_latency: no_max_latency,
};

fn create(rate: SampleRate, _: usize) -> Box<dyn Effect> {
    let size = ((MAX_SECONDS * f64::from(rate.hz())) as usize + 4).next_power_of_two();
    Box::new(Delay { left: vec![0.0; size], right: vec![0.0; size], mask: size - 1, write: 0, delay: 0.0, low_l: 0.0, low_r: 0.0 })
}

/// The running delay.
#[derive(Debug)]
struct Delay {
    left: Vec<f32>,
    right: Vec<f32>,
    mask: usize,
    write: usize,
    /// The delay of the last sample, in frames.
    delay: f64,
    low_l: f32,
    low_r: f32,
}

impl Delay {
    /// The buffer of `channel` at `delay` frames before the write position, interpolated.
    fn read(&self, right: bool, delay: f64) -> f32 {
        let buffer = if right { &self.right } else { &self.left };
        let whole = delay.floor();
        let frac = (delay - whole) as f32;
        let at = |back: usize| buffer.get(self.write.wrapping_sub(back) & self.mask).copied().unwrap_or(0.0);
        let back = whole as usize;
        at(back) * (1.0 - frac) + at(back + 1) * frac
    }
}

impl Effect for Delay {
    /// RT-safe.
    #[rt_safe]
    fn process(&mut self, block: &mut EffectBlock<'_>) {
        let choice = (block.param(TIME, 5.0).round().max(0.0) as usize).min(BEATS.len() - 1);
        let beats = BEATS.get(choice).copied().unwrap_or(1.0);
        let tempo = if block.tempo.is_finite() && block.tempo > 0.0 { block.tempo } else { 120.0 };
        let fs = f64::from(block.rate.hz());
        let target = (beats * 60.0 / tempo * fs).clamp(1.0, (self.mask as f64) - 2.0);
        let feedback = (block.param(FEEDBACK, 35.0) / 100.0).clamp(0.0, 0.95);
        let mix = (block.param(MIX, 30.0) / 100.0).clamp(0.0, 1.0);
        let ping_pong = block.param(PING_PONG, 0.0) >= 0.5;
        let cut = f64::from(block.param(HIGH_CUT, 8000.0).clamp(500.0, 20_000.0));
        let smoothing = (1.0 - (-2.0 * core::f64::consts::PI * cut / fs).exp()) as f32;
        let frames = block.left.len().min(block.right.len());
        let start = if self.delay > 0.0 { self.delay } else { target };
        let step = (target - start) / frames.max(1) as f64;
        for i in 0..frames {
            let delay = start + step * (i + 1) as f64;
            let (Some(l), Some(r)) = (block.left.get(i).copied(), block.right.get(i).copied()) else { break };
            let (echo_l, echo_r) = (self.read(false, delay), self.read(true, delay));
            // A gentle low pass in the feedback path darkens each repeat.
            self.low_l += smoothing * (echo_l - self.low_l);
            self.low_r += smoothing * (echo_r - self.low_r);
            let (into_l, into_r) = if ping_pong {
                (0.5 * (l + r) + feedback * self.low_r, feedback * self.low_l)
            } else {
                (l + feedback * self.low_l, r + feedback * self.low_r)
            };
            if let (Some(a), Some(b)) = (self.left.get_mut(self.write), self.right.get_mut(self.write)) {
                *a = into_l;
                *b = into_r;
            }
            self.write = (self.write + 1) & self.mask;
            if let (Some(out_l), Some(out_r)) = (block.left.get_mut(i), block.right.get_mut(i)) {
                *out_l = l * (1.0 - mix) + echo_l * mix;
                *out_r = r * (1.0 - mix) + echo_r * mix;
            }
        }
        self.delay = target;
    }

    fn reset(&mut self) {
        self.left.fill(0.0);
        self.right.fill(0.0);
        self.low_l = 0.0;
        self.low_r = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_impulse_echoes_on_the_beat() {
        let rate = SampleRate::HZ_48000;
        let mut effect = create(rate, 512);
        // A quarter note at 120 BPM is half a second: 24 000 frames. Full wet, no feedback, no high cut to speak of.
        let params = [5.0, 0.0, 100.0, 0.0, 20_000.0];
        let mut out = Vec::new();
        for block in 0..60 {
            let mut l = vec![0.0f32; 512];
            if block == 0 {
                l[0] = 1.0;
            }
            let mut r = l.clone();
            let mut b = EffectBlock { left: &mut l, right: &mut r, sidechain: None, params: &params, rate, tempo: 120.0 };
            effect.process(&mut b);
            out.extend(l);
        }
        let peak = out.iter().enumerate().max_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).map(|(i, _)| i);
        assert_eq!(peak, Some(24_000));
        assert!(out[..23_990].iter().all(|s| s.abs() < 1e-6), "nothing before the echo when fully wet");
    }
}
