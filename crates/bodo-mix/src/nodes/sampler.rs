//! The built-in sampler: plays a sample at the pitch of each note (project.md section 10.2).

use std::sync::Arc;
use std::sync::atomic::Ordering;

use bodo_kit_audio::AudioData;
use bodo_kit_core::{SampleRate, rt_safe};
use bodo_kit_dsp::envelope::{Adsr, AdsrParams};
use bodo_kit_dsp::gain::ramp_gain;
use bodo_kit_dsp::interp::hermite;
use bodo_kit_dsp::pan::{PanLaw, StereoPan};
use bodo_kit_graph::NodeIo;
use bodo_model::channel::{LoopMode, SamplerParams};

use crate::context::{BlockContext, EventKind, NoteEvent};
use crate::live::ChannelLive;

/// One playing note.
#[derive(Clone, Debug)]
struct Voice {
    active: bool,
    voice: u64,
    velocity: f32,
    /// Read position in sample frames.
    position: f64,
    /// Frames to advance per output frame; negative when playing backwards.
    step: f64,
    envelope: Adsr,
    /// When the voice started, for stealing the oldest.
    age: u64,
}

/// A sampler channel's processor.
#[derive(Debug)]
pub struct SamplerNode {
    key: u64,
    sample: Option<Arc<AudioData>>,
    root_key: u8,
    key_range: (u8, u8),
    envelope: AdsrParams,
    start: f32,
    loop_mode: LoopMode,
    loop_start: f32,
    loop_end: f32,
    reverse: bool,
    /// Note-offs don't stop the sample, unless it loops.
    one_shot: bool,
    voices: Vec<Voice>,
    live: Arc<ChannelLive>,
    pan_law: PanLaw,
    rate: SampleRate,
    started: u64,
    volume: f32,
    pan: f32,
}

impl SamplerNode {
    /// A sampler for the channel with event key `key`, playing `sample` with `params`.
    ///
    /// Allocates its voices.
    pub fn new(key: u64, sample: Option<Arc<AudioData>>, params: &SamplerParams, live: Arc<ChannelLive>, pan_law: PanLaw, rate: SampleRate) -> Self {
        let voice = Voice { active: false, voice: 0, velocity: 0.0, position: 0.0, step: 1.0, envelope: Adsr::new(params.envelope, rate), age: 0 };
        let volume = live.volume.get();
        let pan = live.pan.get();
        Self {
            key,
            sample,
            root_key: params.root_key,
            key_range: params.key_range,
            envelope: params.envelope,
            start: params.start.clamp(0.0, 1.0),
            loop_mode: params.loop_mode,
            loop_start: params.loop_start.clamp(0.0, 1.0),
            loop_end: params.loop_end.clamp(0.0, 1.0),
            reverse: params.reverse,
            one_shot: params.one_shot,
            voices: vec![voice; usize::from(params.polyphony.max(1))],
            live,
            pan_law,
            rate,
            started: 0,
            volume,
            pan,
        }
    }

    /// Renders one block.
    ///
    /// RT-safe. Complexity: O(v * n) in active voices and frames, plus O(e) in events.
    #[rt_safe]
    pub fn process(&mut self, io: &mut NodeIo<'_>, context: &BlockContext) {
        io.output.clear();
        let frames = io.frames;
        let mut cursor = 0;
        for event in context.events_for(self.key) {
            let at = (event.frame as usize).min(frames);
            self.render(io, cursor, at);
            cursor = at;
            self.apply(event);
        }
        self.render(io, cursor, frames);
        self.finish(io);
    }

    fn apply(&mut self, event: &NoteEvent) {
        match event.kind {
            EventKind::AllOff => self.voices.iter_mut().for_each(|v| v.envelope.gate_off()),
            // A one-shot plays to the end of its sample; only stopping the transport cuts it.
            EventKind::Off if self.one_shot && self.loop_mode == LoopMode::Off => {}
            EventKind::Off => {
                if let Some(voice) = self.voices.iter_mut().find(|v| v.active && v.voice == event.voice) {
                    voice.envelope.gate_off();
                }
            }
            EventKind::On => self.note_on(event),
        }
    }

    fn note_on(&mut self, event: &NoteEvent) {
        let Some(sample) = self.sample.as_ref() else { return };
        if !(self.key_range.0..=self.key_range.1).contains(&event.key) {
            return;
        }
        let frames = sample.frames() as f64;
        if frames < 1.0 {
            return;
        }
        let semitones = f64::from(event.key) - f64::from(self.root_key) + f64::from(self.live.pitch_cents.get()) / 100.0;
        let speed = 2f64.powf(semitones / 12.0);
        let (position, step) =
            if self.reverse { ((frames - 1.0) * f64::from(1.0 - self.start), -speed) } else { (frames * f64::from(self.start), speed) };
        // Take a free voice, or steal the oldest.
        let index = self.voices.iter().position(|v| !v.active).or_else(|| self.voices.iter().enumerate().min_by_key(|(_, v)| v.age).map(|(i, _)| i));
        let Some(voice) = index.and_then(|i| self.voices.get_mut(i)) else { return };
        self.started += 1;
        voice.active = true;
        voice.voice = event.voice;
        voice.velocity = event.velocity;
        voice.position = position;
        voice.step = step;
        voice.age = self.started;
        voice.envelope.set_params(self.envelope, self.rate);
        voice.envelope.reset();
        voice.envelope.gate_on();
    }

    /// Adds the voices into the output from frame `from` up to `to`.
    fn render(&mut self, io: &mut NodeIo<'_>, from: usize, to: usize) {
        let Some(sample) = self.sample.as_ref() else { return };
        if from >= to {
            return;
        }
        let last = sample.frames() as f64;
        let (Some(first_channel), second_channel) = (sample.channels.first(), sample.channels.get(1)) else { return };
        let (loop_start, loop_end) = (f64::from(self.loop_start) * last, f64::from(self.loop_end) * last);
        let looping = self.loop_mode != LoopMode::Off && loop_end - loop_start >= 1.0;
        let stereo = second_channel.is_some();
        let Some((left, right)) = io.output.stereo_mut() else { return };
        for voice in self.voices.iter_mut().filter(|v| v.active) {
            for frame in from..to {
                if voice.envelope.is_idle() {
                    voice.active = false;
                    break;
                }
                let gain = voice.envelope.next_level() * voice.velocity;
                left[frame] += read(first_channel, voice.position) * gain;
                if let Some(second) = second_channel {
                    right[frame] += read(second, voice.position) * gain;
                }
                voice.position += voice.step;
                if looping && voice.step > 0.0 && voice.position >= loop_end {
                    match self.loop_mode {
                        LoopMode::PingPong => {
                            voice.position = 2.0 * loop_end - voice.position;
                            voice.step = -voice.step;
                        }
                        _ => voice.position -= loop_end - loop_start,
                    }
                } else if looping && voice.step < 0.0 && voice.position < loop_start && self.loop_mode == LoopMode::PingPong {
                    voice.position = 2.0 * loop_start - voice.position;
                    voice.step = -voice.step;
                } else if voice.position >= last || voice.position < 0.0 {
                    voice.active = false;
                    break;
                }
            }
        }
        // A mono sample's voices were summed on the left; the pan stage spreads them.
        if !stereo {
            right[from..to].copy_from_slice(&left[from..to]);
        }
    }

    /// Applies the channel's mute, volume and pan to the block, ramping from the last block's values.
    fn finish(&mut self, io: &mut NodeIo<'_>) {
        let muted = self.live.muted.load(Ordering::Relaxed);
        let volume = if muted { 0.0 } else { self.live.volume.get() };
        let pan = self.live.pan.get();
        let mono = self.sample.as_ref().is_none_or(|s| s.channel_count() < 2);
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

    /// How many voices are playing.
    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.active).count()
    }
}

/// The sample at fractional `position`, by four-point Hermite interpolation. Outside the data it is silence.
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
    fn reads_exact_samples_at_whole_positions() {
        let data = [0.0f32, 1.0, 2.0, 3.0];
        assert_eq!(read(&data, 1.0), 1.0);
        assert_eq!(read(&data, 2.0), 2.0);
        assert!((read(&data, 1.5) - 1.5).abs() < 1e-6);
        assert_eq!(read(&data, 10.0), 0.0);
    }
}
