//! An audio clip channel: renders the segments of its clips that the sequencer put in the block (project.md section
//! 6.7), then applies the channel's volume, pan and mute like a sampler does.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use bodo_kit_audio::AudioData;
use bodo_kit_core::rt_safe;
use bodo_kit_dsp::gain::ramp_gain;
use bodo_kit_dsp::interp::hermite;
use bodo_kit_dsp::pan::{PanLaw, StereoPan};
use bodo_kit_graph::NodeIo;

use crate::context::BlockContext;
use crate::live::ChannelLive;

/// An audio clip channel's processor.
#[derive(Debug)]
pub struct AudioClipNode {
    key: u64,
    sample: Option<Arc<AudioData>>,
    /// Time-stretched copies of the sample by ratio key, sorted by key.
    stretched: Vec<(u32, Arc<AudioData>)>,
    live: Arc<ChannelLive>,
    pan_law: PanLaw,
    volume: f32,
    pan: f32,
}

impl AudioClipNode {
    /// The node of the channel with key `key`, playing `sample` and its time-stretched copies.
    pub fn new(key: u64, sample: Option<Arc<AudioData>>, mut stretched: Vec<(u32, Arc<AudioData>)>, live: Arc<ChannelLive>, pan_law: PanLaw) -> Self {
        stretched.sort_by_key(|(k, _)| *k);
        let (volume, pan) = (live.volume.get(), live.pan.get());
        Self { key, sample, stretched, live, pan_law, volume, pan }
    }

    /// Renders the block.
    ///
    /// RT-safe. Complexity: O(s * n) in the block's segments and frames, plus O(log v) per segment in stretched copies.
    #[rt_safe]
    pub fn process(&mut self, io: &mut NodeIo<'_>, context: &BlockContext) {
        io.output.clear();
        let frames = io.frames;
        let mono = self.sample.as_ref().is_none_or(|s| s.channel_count() < 2);
        if let Some((left, right)) = io.output.stereo_mut() {
            for segment in context.audio.for_key(self.key) {
                let stretched =
                    segment.stretched.and_then(|key| self.stretched.binary_search_by_key(&key, |(k, _)| *k).ok()).and_then(|i| self.stretched.get(i));
                let (data, position, step) = match (stretched, &self.sample) {
                    (Some((_, data)), _) => (data, segment.position, segment.step),
                    // Stretched audio that isn't there yet: read the sample faster instead, pitch and all.
                    (None, Some(data)) if segment.stretched.is_some() => (data, segment.position * segment.ratio, segment.ratio),
                    (None, Some(data)) => (data, segment.position, segment.step),
                    (None, None) => continue,
                };
                let (Some(first), second) = (data.channels.first(), data.channels.get(1)) else { continue };
                let from = (segment.frame as usize).min(frames);
                let to = (from + segment.frames as usize).min(frames);
                for (i, out) in (from..to).enumerate() {
                    let at = position + i as f64 * step;
                    let gain = segment.fade(i);
                    if let Some(sample) = left.get_mut(out) {
                        *sample += read(first, at) * gain;
                    }
                    if let (Some(second), Some(sample)) = (second, right.get_mut(out)) {
                        *sample += read(second, at) * gain;
                    }
                }
            }
            // A mono sample was summed on the left; the pan stage spreads it.
            if mono {
                let n = frames.min(left.len()).min(right.len());
                right[..n].copy_from_slice(&left[..n]);
            }
        }
        self.finish(io, mono);
    }

    /// Applies the channel's mute, volume and pan to the block, ramping from the last block's values.
    fn finish(&mut self, io: &mut NodeIo<'_>, mono: bool) {
        let muted = self.live.muted.load(Ordering::Relaxed);
        let volume = if muted { 0.0 } else { self.live.volume.get() };
        let pan = self.live.pan.get();
        let Some((left, right)) = io.output.stereo_mut() else { return };
        if mono {
            let (from, to) = (self.pan_law.mono_gains(self.pan), self.pan_law.mono_gains(pan));
            ramp_gain(left, from.left * self.volume, to.left * volume);
            ramp_gain(right, from.right * self.volume, to.right * volume);
        } else {
            let (from, to) = (StereoPan::fold(self.pan), StereoPan::fold(pan));
            if from == to {
                to.process(left, right);
            } else {
                let len = left.len().max(1) as f32;
                for (i, (l, r)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
                    let t = (i + 1) as f32 / len;
                    let mix = |a: f32, b: f32| a + (b - a) * t;
                    let (in_l, in_r) = (*l, *r);
                    *l = mix(from.ll, to.ll) * in_l + mix(from.rl, to.rl) * in_r;
                    *r = mix(from.lr, to.lr) * in_l + mix(from.rr, to.rr) * in_r;
                }
            }
            ramp_gain(left, self.volume, volume);
            ramp_gain(right, self.volume, volume);
        }
        self.volume = volume;
        self.pan = pan;
    }
}

/// The sample at fractional `position`, by four-point Hermite interpolation; exact at whole positions. Outside the
/// data it is silence.
fn read(data: &[f32], position: f64) -> f32 {
    let index = position.floor();
    let t = (position - index) as f32;
    let i = index as i64;
    let at = |k: i64| usize::try_from(k).ok().and_then(|k| data.get(k)).copied().unwrap_or(0.0);
    hermite(at(i - 1), at(i), at(i + 1), at(i + 2), t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_exactly_at_whole_positions_and_silence_outside() {
        let data = [0.25f32, -0.5, 0.75, 1.0];
        assert_eq!(read(&data, 1.0), -0.5);
        assert_eq!(read(&data, 3.0), 1.0);
        assert_eq!(read(&data, -2.0), 0.0);
        assert_eq!(read(&data, 9.0), 0.0);
    }
}
