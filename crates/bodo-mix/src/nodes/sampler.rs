//! The built-in sampler: plays a sample at the pitch of each note (project.md section 10.2).
//!
//! The sample arrives with its precomputed effects, reverse and stretch already rendered in ([`crate::sampler_source`]).
//! Each note plays the part between the start and end markers, from the start offset, or the slice or region of its
//! key; it loops forward or back and forth, fades in and out against clicks, and runs through the volume envelope and
//! the filter, whose cutoff and the pitch follow their own envelopes. The arpeggiator turns held keys into a run of
//! notes in time with the song. Modulation updates every [`MOD_FRAMES`] frames; everything else is per frame.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError};

use bodo_kit_audio::{AudioData, StreamConfig, StreamReader};
use bodo_kit_core::{PPQ, SampleRate, rt_safe};
use bodo_kit_dsp::envelope::Adsr;
use bodo_kit_dsp::filter::{Svf, SvfMode};
use bodo_kit_dsp::gain::ramp_gain;
use bodo_kit_dsp::interp::hermite;
use bodo_kit_dsp::pan::{PanLaw, StereoPan};
use bodo_kit_graph::NodeIo;
use bodo_model::channel::{FilterKind, LoopMode, SamplerParams, SamplerStretchMode};

use super::arp::{Arp, ArpAction};
use crate::automation::{Param, param_key};
use crate::context::{BlockContext, EventKind, NoteEvent};
use crate::disk_sample::DiskSample;
use crate::live::ChannelLive;
use crate::sampler_source::{SamplerData, Zone, length_multiplier};

/// Frames between updates of the pitch and filter modulation.
pub const MOD_FRAMES: usize = 16;

/// One playing note.
#[derive(Clone, Debug)]
struct Voice {
    active: bool,
    voice: u64,
    key: u8,
    velocity: f32,
    /// Read position in sample frames.
    position: f64,
    /// Frames to advance per output frame, before the pitch envelope; negative while a ping-pong loop runs back.
    step: f64,
    /// The part of the sample the note plays.
    region: Zone,
    /// The loop, when the note loops.
    looping: Option<Zone>,
    amp: Adsr,
    filter_env: Adsr,
    pitch_env: Adsr,
    filters: [Svf; 2],
    /// Frames played, for the fade-in and the modulation schedule.
    elapsed: u64,
    /// When the voice started, for stealing the oldest.
    age: u64,
    /// The pitch envelope's speed factor, updated with the modulation.
    pitch: f64,
}

/// Frames of the window a streaming voice keeps behind its head.
const WINDOW: usize = 16_384;

/// A voice's stream, for a sampler that keeps its sample on disk ([`crate::disk_sample`]): its reader, the head it
/// plays from memory, and the window of frames streamed after the head.
#[derive(Debug)]
struct DiskVoice {
    reader: StreamReader,
    /// The head the voice reads first, by its index in the sample's heads.
    head: Option<usize>,
    /// Frames streamed after the head, per channel.
    window: [Vec<f32>; 2],
    /// The sample frame the window starts at.
    window_start: u64,
    /// How many frames of the window are filled.
    filled: usize,
}

impl DiskVoice {
    /// Starts reading at `position`: from the head that covers it, with the stream picking up where the head ends.
    ///
    /// RT-safe.
    fn start(&mut self, disk: &DiskSample, position: f64) {
        let at = position.max(0.0) as u64;
        self.head = disk.heads.partition_point(|(start, _)| *start <= at).checked_sub(1);
        let end = match self.head.and_then(|i| disk.heads.get(i)) {
            Some((start, data)) if at < start + data.frames() as u64 => start + data.frames() as u64,
            _ => at,
        };
        self.window_start = end;
        self.filled = 0;
        self.reader.seek(end);
    }

    /// Makes sure the window covers the frames around `position`: drops what the voice has passed and reads on.
    ///
    /// RT-safe. Complexity: O(w) in the window, now and then.
    fn fill(&mut self, position: f64) {
        let passed = (position.floor() as i64 - 2).max(0) as u64;
        if passed > self.window_start + (WINDOW / 2) as u64 {
            let drop = ((passed - self.window_start) as usize).min(self.filled);
            for channel in &mut self.window {
                channel.copy_within(drop..self.filled, 0);
            }
            self.window_start += drop as u64;
            self.filled -= drop;
        }
        let reach = position.floor() as i64 + 3;
        let end = (self.window_start + self.filled as u64) as i64;
        if reach >= end && reach >= self.window_start as i64 && self.filled < WINDOW {
            let [left, right] = &mut self.window;
            self.filled += self.reader.read(&mut [&mut left[self.filled..], &mut right[self.filled..]]);
        }
    }

    /// Sample frame `k` of channel `channel`: from the head, or the window; silence when the disk hasn't caught up.
    fn at(&self, disk: &DiskSample, channel: usize, k: i64) -> f32 {
        let Ok(k) = u64::try_from(k) else { return 0.0 };
        if let Some((start, data)) = self.head.and_then(|i| disk.heads.get(i))
            && k >= *start
            && k < start + data.frames() as u64
        {
            return data.channels.get(channel).and_then(|c| c.get((k - start) as usize)).copied().unwrap_or(0.0);
        }
        if k >= self.window_start && k < self.window_start + self.filled as u64 {
            return self.window[channel.min(1)].get((k - self.window_start) as usize).copied().unwrap_or(0.0);
        }
        0.0
    }

    /// Channel `channel` at fractional `position`, by Hermite interpolation like [`read`].
    fn read(&self, disk: &DiskSample, channel: usize, position: f64) -> f32 {
        let index = position.floor();
        let t = (position - index) as f32;
        let i = index as i64;
        hermite(self.at(disk, channel, i - 1), self.at(disk, channel, i), self.at(disk, channel, i + 1), self.at(disk, channel, i + 2), t)
    }
}

/// A sampler channel's processor.
#[derive(Debug)]
pub struct SamplerNode {
    key: u64,
    sample: Option<Arc<AudioData>>,
    /// The sample streamed from disk, when the sampler keeps it there.
    disk: Option<Arc<DiskSample>>,
    /// Each voice's stream, when the sample streams; by voice index. Graph processors must be `Sync` and stream
    /// readers are not, so they sit in a mutex that is never locked: [`Mutex::get_mut`] borrows them without a lock.
    streams: Mutex<Vec<Option<DiskVoice>>>,
    zones: Arc<[Zone]>,
    params: SamplerParams,
    /// Sample frames per output frame at the sample's own pitch: the sample's rate over the project's.
    rate_ratio: f64,
    /// The declick fades, in frames.
    fade_in: f64,
    fade_out: f64,
    filter_mode: Option<SvfMode>,
    filter_q: f64,
    voices: Vec<Voice>,
    arp: Arp,
    /// The song tick the arpeggiator runs on while the transport is stopped.
    free_tick: f64,
    live: Arc<ChannelLive>,
    pan_law: PanLaw,
    rate: SampleRate,
    started: u64,
    volume: f32,
    pan: f32,
}

impl SamplerNode {
    /// A sampler for the channel with event key `key`, playing `data` with `params`.
    ///
    /// Allocates its voices.
    pub fn new(key: u64, data: SamplerData, params: &SamplerParams, live: Arc<ChannelLive>, pan_law: PanLaw, rate: SampleRate) -> Self {
        let filter_mode = match params.filter.kind {
            FilterKind::Off => None,
            FilterKind::LowPass => Some(SvfMode::LowPass),
            FilterKind::HighPass => Some(SvfMode::HighPass),
            FilterKind::BandPass => Some(SvfMode::BandPass),
            FilterKind::Notch => Some(SvfMode::Notch),
        };
        // Resonance 0 to 1 spans Q from a Butterworth's 0.707 up to about 20.
        let filter_q = core::f64::consts::FRAC_1_SQRT_2 * 2f64.powf(f64::from(params.filter.resonance.clamp(0.0, 1.0)) * 4.8);
        let cutoff = f64::from(params.filter.cutoff.clamp(20.0, 20_000.0));
        let voice = Voice {
            active: false,
            voice: 0,
            key: 0,
            velocity: 0.0,
            position: 0.0,
            step: 1.0,
            region: (0.0, 0.0),
            looping: None,
            amp: Adsr::new(params.envelope, rate),
            filter_env: Adsr::new(params.filter_envelope.adsr, rate),
            pitch_env: Adsr::new(params.pitch_envelope.adsr, rate),
            filters: [Svf::new(cutoff, filter_q, rate); 2],
            elapsed: 0,
            age: 0,
            pitch: 1.0,
        };
        let hz = f64::from(rate.hz());
        let own_rate = data.sample.as_ref().map(|s| s.rate).or_else(|| data.disk.as_ref().map(|d| d.rate));
        let rate_ratio = own_rate.map_or(1.0, |r| f64::from(r.hz()) / hz);
        let (fade_in, fade_out) = params.declick.times();
        let volume = live.volume.get();
        let pan = live.pan.get();
        let polyphony = usize::from(params.polyphony.max(1));
        // A streaming sampler opens a reader per voice here, off the audio thread.
        let streams = (0..polyphony)
            .map(|_| {
                let disk = data.disk.as_ref()?;
                let config = StreamConfig { chunk_frames: 2048, chunks: 16 };
                let reader = disk.thread.open(&disk.path, config).map_err(|err| tracing::warn!(%err, "a sampler voice can't stream")).ok()?;
                Some(DiskVoice { reader, head: None, window: [vec![0.0; WINDOW], vec![0.0; WINDOW]], window_start: 0, filled: 0 })
            })
            .collect();
        Self {
            key,
            sample: data.sample,
            disk: data.disk,
            streams: Mutex::new(streams),
            zones: data.zones,
            params: params.clone(),
            rate_ratio,
            fade_in: fade_in * hz,
            fade_out: fade_out * hz,
            filter_mode,
            filter_q,
            voices: vec![voice; polyphony],
            arp: Arp::new(params.arp),
            free_tick: 0.0,
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
        let start_tick = if context.playing { context.tick } else { self.free_tick };
        let arp = self.arp.enabled();
        if arp {
            self.arp.begin_block(start_tick, context.ticks_per_frame, frames);
        }
        let mut events = context.events_for(self.key).peekable();
        let mut cursor = 0;
        loop {
            let event_at = events.peek().map(|e| (e.frame as usize).min(frames));
            let arp_at = if arp { self.arp.due() } else { None };
            match (event_at, arp_at) {
                (event, Some(at)) if event.is_none_or(|e| at < e) => {
                    self.render(io, cursor, at);
                    cursor = at;
                    if let Some((_, action)) = self.arp.fire() {
                        self.arp_action(action, context);
                    }
                }
                (Some(at), _) => {
                    self.render(io, cursor, at);
                    cursor = at;
                    if let Some(event) = events.next() {
                        self.apply(event, at, context);
                    }
                }
                _ => break,
            }
        }
        self.render(io, cursor, frames);
        self.finish(io, context);
        self.free_tick = start_tick + frames as f64 * context.ticks_per_frame;
    }

    fn apply(&mut self, event: &NoteEvent, frame: usize, context: &BlockContext) {
        if self.arp.enabled() {
            match event.kind {
                EventKind::AllOff => {
                    if let Some(voice) = self.arp.clear() {
                        self.release(voice);
                    }
                    self.voices.iter_mut().for_each(|v| v.amp.gate_off());
                }
                EventKind::Off => self.arp.release(event.key),
                EventKind::On => self.arp.press(event.key, event.velocity, self.arp.tick_at(frame)),
            }
            return;
        }
        match event.kind {
            EventKind::AllOff => self.voices.iter_mut().for_each(|v| v.amp.gate_off()),
            EventKind::Off => self.release(event.voice),
            EventKind::On => self.note_on(event.key, event.velocity, event.voice, context),
        }
    }

    fn arp_action(&mut self, action: ArpAction, context: &BlockContext) {
        match action {
            ArpAction::On { key, velocity, voice } => self.note_on(key, velocity, voice, context),
            ArpAction::Off { voice } => self.release(voice),
        }
    }

    /// Releases the note `voice`; a one-shot plays to the end of its sample, and only stopping the transport cuts it.
    fn release(&mut self, voice: u64) {
        if self.params.one_shot && self.params.loop_mode == LoopMode::Off {
            return;
        }
        if let Some(v) = self.voices.iter_mut().find(|v| v.active && v.voice == voice) {
            v.amp.gate_off();
            v.filter_env.gate_off();
            v.pitch_env.gate_off();
        }
    }

    /// Starts a voice for `key`.
    fn note_on(&mut self, key: u8, velocity: f32, voice_id: u64, context: &BlockContext) {
        let (frames, own_rate) = match (&self.sample, &self.disk) {
            (Some(sample), _) => (sample.frames() as u64, sample.rate),
            (None, Some(disk)) => (disk.frames, disk.rate),
            (None, None) => return,
        };
        let params = &self.params;
        if !(params.key_range.0..=params.key_range.1).contains(&key) {
            return;
        }
        let total = frames;
        let frames = frames as f64;
        if frames < 1.0 {
            return;
        }
        let cents = f64::from(context.params.get(param_key(self.key, Param::Pitch)).unwrap_or_else(|| self.live.pitch_cents.get()));
        let (region, position, semitones, looping) = if self.zones.is_empty() {
            let region = (f64::from(params.start.clamp(0.0, 1.0)) * frames, f64::from(params.end.clamp(0.0, 1.0)) * frames);
            if region.1 - region.0 < 1.0 {
                return;
            }
            let position = region.0 + f64::from(params.start_offset.clamp(0.0, 1.0)) * (region.1 - region.0);
            let (loop_start, loop_end) = ((f64::from(params.loop_start) * frames).max(region.0), (f64::from(params.loop_end) * frames).min(region.1));
            let looping =
                (params.loop_mode != LoopMode::Off && loop_end - loop_start >= 1.0 && position < loop_end).then_some((loop_start, loop_end));
            (region, position, f64::from(key) - f64::from(params.root_key) + cents / 100.0, looping)
        } else {
            // Slices and regions play at their own pitch, one per key from the root key up.
            let Some(zone) = key.checked_sub(params.root_key).and_then(|i| self.zones.get(usize::from(i))).copied() else { return };
            (zone, zone.0, cents / 100.0, None)
        };
        let bpm = context.ticks_per_frame * f64::from(self.rate.hz()) * 60.0 / PPQ as f64;
        let speed = 2f64.powf(semitones / 12.0) * self.rate_ratio * resample_speed(total, own_rate, params, bpm);
        // Take a free voice, or steal the oldest.
        let index = self.voices.iter().position(|v| !v.active).or_else(|| self.voices.iter().enumerate().min_by_key(|(_, v)| v.age).map(|(i, _)| i));
        let streams = self.streams.get_mut().unwrap_or_else(PoisonError::into_inner);
        if let (Some(disk), Some(stream)) = (&self.disk, index.and_then(|i| streams.get_mut(i)).and_then(Option::as_mut)) {
            stream.start(disk, position);
        }
        let Some(voice) = index.and_then(|i| self.voices.get_mut(i)) else { return };
        self.started += 1;
        voice.active = true;
        voice.voice = voice_id;
        voice.key = key;
        voice.velocity = velocity;
        voice.position = position;
        voice.step = speed;
        voice.region = region;
        voice.looping = looping;
        voice.age = self.started;
        voice.elapsed = 0;
        voice.pitch = 1.0;
        for (env, adsr) in [
            (&mut voice.amp, params.envelope),
            (&mut voice.filter_env, params.filter_envelope.adsr),
            (&mut voice.pitch_env, params.pitch_envelope.adsr),
        ] {
            env.set_params(adsr, self.rate);
            env.reset();
            env.gate_on();
        }
        // The cutoff follows the key; an envelope moves it from there.
        let tracking = f64::from(params.filter.key_tracking) * (f64::from(key) - 60.0) / 12.0;
        let hz = (f64::from(params.filter.cutoff.clamp(20.0, 20_000.0)) * 2f64.powf(tracking)).clamp(20.0, 20_000.0);
        for filter in &mut voice.filters {
            filter.reset();
            filter.set(hz, self.filter_q, self.rate);
        }
    }

    /// Adds the voices into the output from frame `from` up to `to`.
    fn render(&mut self, io: &mut NodeIo<'_>, from: usize, to: usize) {
        if from >= to {
            return;
        }
        let channels = match (&self.sample, &self.disk) {
            (Some(sample), _) => sample.channel_count(),
            (None, Some(disk)) => disk.channels,
            (None, None) => return,
        };
        if channels == 0 {
            return;
        }
        let stereo = channels >= 2;
        let memory = self.sample.as_ref().map(|s| (s.channels.first(), s.channels.get(1)));
        let Some((left, right)) = io.output.stereo_mut() else { return };
        let params = &self.params;
        let (pitch_mod, filter_mod) = (params.pitch_envelope.enabled, params.filter_envelope.enabled && self.filter_mode.is_some());
        let cutoff = f64::from(params.filter.cutoff.clamp(20.0, 20_000.0));
        let streams = self.streams.get_mut().unwrap_or_else(PoisonError::into_inner);
        for (index, voice) in self.voices.iter_mut().enumerate().filter(|(_, v)| v.active) {
            let mut stream = streams.get_mut(index).and_then(Option::as_mut);
            let tracking = f64::from(params.filter.key_tracking) * (f64::from(voice.key) - 60.0) / 12.0;
            for f in from..to {
                if voice.amp.is_idle() {
                    voice.active = false;
                    break;
                }
                // Modulation moves every MOD_FRAMES frames of the voice's life, wherever the blocks fall.
                if voice.elapsed % MOD_FRAMES as u64 == 0 && (pitch_mod || filter_mod) {
                    if pitch_mod {
                        voice.pitch = 2f64.powf(f64::from(voice.pitch_env.level() * params.pitch_envelope.amount) / 12.0);
                    }
                    if filter_mod {
                        let octaves = f64::from(voice.filter_env.level() * params.filter_envelope.amount);
                        let hz = (cutoff * 2f64.powf(octaves + tracking)).clamp(20.0, 20_000.0);
                        voice.filters.iter_mut().for_each(|filter| filter.set(hz, self.filter_q, self.rate));
                    }
                }
                if pitch_mod {
                    voice.pitch_env.next_level();
                }
                if filter_mod {
                    voice.filter_env.next_level();
                }
                let step = voice.step * voice.pitch;
                let mut gain = voice.amp.next_level() * voice.velocity;
                if self.fade_in > 0.0 && (voice.elapsed as f64) < self.fade_in {
                    gain *= (voice.elapsed as f64 / self.fade_in) as f32;
                }
                if self.fade_out > 0.0 && voice.looping.is_none() {
                    let remaining = (voice.region.1 - voice.position) / step.abs().max(1e-9);
                    if remaining < self.fade_out {
                        gain *= (remaining / self.fade_out).max(0.0) as f32;
                    }
                }
                let (mut l, mut r) = match (memory, &self.disk, stream.as_deref_mut()) {
                    (Some((first, second)), _, _) => {
                        (first.map_or(0.0, |c| read(c, voice.position)), second.map_or(0.0, |c| read(c, voice.position)))
                    }
                    (None, Some(disk), Some(stream)) => {
                        stream.fill(voice.position);
                        let r = if stereo { stream.read(disk, 1, voice.position) } else { 0.0 };
                        (stream.read(disk, 0, voice.position), r)
                    }
                    _ => (0.0, 0.0),
                };
                if let Some(mode) = self.filter_mode {
                    l = voice.filters[0].process_sample(l, mode);
                    if stereo {
                        r = voice.filters[1].process_sample(r, mode);
                    }
                }
                left[f] += l * gain;
                right[f] += r * gain;
                voice.elapsed += 1;
                let before = voice.position;
                if !advance(voice, step, params.loop_mode) {
                    voice.active = false;
                    break;
                }
                // A loop jumped back: the stream starts again from the loop's head.
                if voice.position < before
                    && let (Some(disk), Some(stream)) = (&self.disk, stream.as_deref_mut())
                {
                    stream.start(disk, voice.position);
                }
            }
        }
        // A mono sample's voices were summed on the left; the pan stage spreads them.
        if !stereo {
            right[from..to].copy_from_slice(&left[from..to]);
        }
    }

    /// Applies the channel's mute, volume and pan to the block, ramping from the last block's values.
    fn finish(&mut self, io: &mut NodeIo<'_>, context: &BlockContext) {
        let muted = self.live.muted.load(Ordering::Relaxed);
        let automated = |param, live: f32| context.params.get(param_key(self.key, param)).unwrap_or(live);
        let volume = if muted { 0.0 } else { automated(Param::Volume, self.live.volume.get()) };
        let pan = automated(Param::Pan, self.live.pan.get()).clamp(-1.0, 1.0);
        let channels = self.sample.as_ref().map(|s| s.channel_count()).or_else(|| self.disk.as_ref().map(|d| d.channels));
        let mono = channels.is_none_or(|c| c < 2);
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

/// How much faster Resample mode reads a sample `frames` long at `rate`: the pitch shift, over how much longer it
/// plays.
fn resample_speed(frames: u64, rate: SampleRate, params: &SamplerParams, bpm: f64) -> f64 {
    if params.stretch.mode != SamplerStretchMode::Resample || params.stretch.is_identity() {
        return 1.0;
    }
    2f64.powf(f64::from(params.stretch.pitch_cents) / 1200.0) / length_multiplier(frames, rate, params, bpm)
}

/// Moves `voice` on by `step` frames, looping as it should. Returns false when the note has run out of sample.
fn advance(voice: &mut Voice, step: f64, mode: LoopMode) -> bool {
    voice.position += if voice.step < 0.0 { -step.abs() } else { step.abs() };
    match voice.looping {
        Some((start, end)) if voice.step > 0.0 && voice.position >= end => {
            if mode == LoopMode::PingPong {
                voice.position = (2.0 * end - voice.position).max(start);
                voice.step = -voice.step;
            } else {
                voice.position = start + (voice.position - end) % (end - start);
            }
            true
        }
        Some((start, end)) if voice.step < 0.0 && voice.position < start => {
            voice.position = (2.0 * start - voice.position).min(end);
            voice.step = -voice.step;
            true
        }
        Some(_) => true,
        None => voice.position < voice.region.1 && voice.position >= voice.region.0,
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

    fn voice(looping: Option<Zone>) -> Voice {
        let rate = SampleRate::HZ_48000;
        let adsr = bodo_kit_dsp::envelope::AdsrParams::default();
        Voice {
            active: true,
            voice: 0,
            key: 60,
            velocity: 1.0,
            position: 0.0,
            step: 1.0,
            region: (0.0, 100.0),
            looping,
            amp: Adsr::new(adsr, rate),
            filter_env: Adsr::new(adsr, rate),
            pitch_env: Adsr::new(adsr, rate),
            filters: [Svf::new(1000.0, 0.7, rate); 2],
            elapsed: 0,
            age: 0,
            pitch: 1.0,
        }
    }

    #[test]
    fn forward_loops_wrap_and_ping_pong_turns_at_both_ends() {
        let mut v = voice(Some((10.0, 20.0)));
        v.position = 19.5;
        assert!(advance(&mut v, 1.0, LoopMode::Forward));
        assert!((v.position - 10.5).abs() < 1e-9);
        let mut v = voice(Some((10.0, 20.0)));
        v.position = 19.5;
        advance(&mut v, 1.0, LoopMode::PingPong);
        assert!((v.position - 19.5).abs() < 1e-9 && v.step < 0.0, "turned back at the end");
        v.position = 10.2;
        advance(&mut v, 1.0, LoopMode::PingPong);
        assert!((v.position - 10.8).abs() < 1e-9 && v.step > 0.0, "and forward again at the start");
        let mut v = voice(None);
        v.position = 99.5;
        assert!(!advance(&mut v, 1.0, LoopMode::Off), "past the end of the region the note is over");
    }
}
