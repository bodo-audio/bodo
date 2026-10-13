//! A mixer insert or the master, in the stage order of ADR 0009.
//!
//! M1 runs these stages: input sum (done by the graph), reverse polarity, swap left/right, stereo separation, pan,
//! fader and the 5 ms mute ramp. FX slots, the strip EQ, send taps other than post-fader, solo and negative track
//! latency arrive with the full mixer in M4. With every control at its default the strip is a bit-exact
//! pass-through, which null tests rely on.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use bodo_kit_core::{SampleRate, rt_safe};
use bodo_kit_dsp::gain::{db_to_gain, ramp_gain};
use bodo_kit_dsp::pan::StereoPan;
use bodo_kit_dsp::stereo::{invert_polarity, stereo_separation, swap_channels};
use bodo_kit_graph::NodeIo;

use crate::automation::{Param, param_key};
use crate::context::BlockContext;
use crate::live::StripLive;

/// The length of the mute ramp, so muting never clicks.
const MUTE_RAMP_SECONDS: f64 = 0.005;

/// A mixer strip's processor.
#[derive(Debug)]
pub struct StripNode {
    /// The insert's graph key, which its automated parameters are keyed by.
    key: u64,
    live: Arc<StripLive>,
    fader: f32,
    mute: f32,
    pan: f32,
}

impl StripNode {
    /// The strip of the insert with graph key `key`, reading its settings from `live`, starting where they are now.
    pub fn new(key: u64, live: Arc<StripLive>) -> Self {
        let fader = db_to_gain(live.volume_db.get());
        let mute = if live.muted.load(Ordering::Relaxed) { 0.0 } else { 1.0 };
        let pan = live.pan.get();
        Self { key, live, fader, mute, pan }
    }

    /// Processes one block.
    ///
    /// RT-safe. Complexity: O(n) in the frames.
    #[rt_safe]
    pub fn process(&mut self, io: &mut NodeIo<'_>, context: &BlockContext) {
        match io.inputs.first() {
            Some(input) => io.output.copy_from(input),
            None => io.output.clear(),
        }
        let Some((left, right)) = io.output.stereo_mut() else { return };
        // 1-2: polarity and swap, before the FX, like a console preamp.
        if self.live.polarity_invert.load(Ordering::Relaxed) {
            invert_polarity(left);
            invert_polarity(right);
        }
        if self.live.swap_lr.load(Ordering::Relaxed) {
            swap_channels(left, right);
        }
        // Automation, where it plays, stands in for the live settings.
        let automated = |param, live: f32| context.params.get(param_key(self.key, param)).unwrap_or(live);
        // 5-6: stereo separation, then pan.
        stereo_separation(left, right, automated(Param::StereoSep, self.live.stereo_sep.get()).clamp(-1.0, 1.0));
        let pan = automated(Param::Pan, self.live.pan.get()).clamp(-1.0, 1.0);
        apply_pan(left, right, self.pan, pan);
        self.pan = pan;
        // 7-8: the fader and the mute ramp, folded into one gain ramp.
        let fader = db_to_gain(automated(Param::Volume, self.live.volume_db.get()));
        let target_mute = if self.live.muted.load(Ordering::Relaxed) { 0.0 } else { 1.0 };
        let mute = step_toward(self.mute, target_mute, io.frames, context.rate);
        ramp_gain(left, self.fader * self.mute, fader * mute);
        ramp_gain(right, self.fader * self.mute, fader * mute);
        self.fader = fader;
        self.mute = mute;
    }
}

/// Moves a mute gain toward `target` by as much as `frames` frames of the 5 ms ramp allow.
fn step_toward(current: f32, target: f32, frames: usize, rate: SampleRate) -> f32 {
    let ramp_frames = (MUTE_RAMP_SECONDS * f64::from(rate.hz())).max(1.0);
    // Snap within a hair of the target, so float rounding never adds an extra block to the ramp.
    let step = (frames as f64 / ramp_frames) as f32 + 1e-6;
    if current < target { (current + step).min(target) } else { (current - step).max(target) }
}

/// Pans with stereo fold, interpolating the matrix across the block when the pan moved.
fn apply_pan(left: &mut [f32], right: &mut [f32], from: f32, to: f32) {
    let (a, b) = (StereoPan::fold(from), StereoPan::fold(to));
    if a == b {
        b.process(left, right);
        return;
    }
    let len = left.len().max(1) as f32;
    for (i, (l, r)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
        let t = (i + 1) as f32 / len;
        let mix = |x: f32, y: f32| x + (y - x) * t;
        let (in_l, in_r) = (*l, *r);
        *l = mix(a.ll, b.ll) * in_l + mix(a.rl, b.rl) * in_r;
        *r = mix(a.lr, b.lr) * in_l + mix(a.rr, b.rr) * in_r;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mute_ramp_takes_five_milliseconds() {
        let rate = SampleRate::HZ_48000;
        let mut gain = 1.0;
        let mut blocks = 0;
        while gain > 0.0 {
            gain = step_toward(gain, 0.0, 48, rate);
            blocks += 1;
        }
        assert_eq!(blocks, 5, "240 frames at 48 kHz in blocks of 48");
    }
}
