//! A mixer insert or the master, in the stage order of ADR 0009 and project.md section 9.3, as three graph nodes:
//!
//! 1. [`StripInput`]: the input sum (done by the graph, latency-aligned and scaled by send levels), plus the hardware
//!    input of an armed insert, then reverse polarity and swap left/right. Its output is the PreFx tap.
//! 2. [`StripFx`]: FX slots 1 to 10 in order, the 3-band strip EQ, then stereo separation. Its output is the PreFader
//!    tap. Each slot has a sidechain input port when its sidechain route exists; the graph lines it up with the slot.
//! 3. [`StripOutput`]: pan, the fader smoothed per sample, mute (and solo silencing) with a 5 ms ramp, the post-fader
//!    point (meters, the PostFader record tap), then the negative track-latency delay. Its output feeds the master,
//!    the routes that tap PostFader and the direct hardware output.
//!
//! Splitting the strip lets the graph run the stages of different inserts in parallel and lets routes and sidechains
//! leave each insert at their tap. With every control at its default, the strip passes audio through bit for bit,
//! which null tests rely on.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError};

use bodo_kit_core::queue::{Consumer, Producer};
use bodo_kit_core::{SampleRate, rt_safe};
use bodo_kit_dsp::delay::DelayLine;
use bodo_kit_dsp::filter::{Biquad, BiquadCoeffs, BiquadKind};
use bodo_kit_dsp::gain::{db_to_gain, ramp_gain};
use bodo_kit_dsp::pan::StereoPan;
use bodo_kit_dsp::stereo::{invert_polarity, stereo_separation, swap_channels};
use bodo_kit_graph::NodeIo;
use bodo_model::mixer::{FX_SLOTS, RecordTap};

use crate::automation::{Param, param_key};
use crate::context::BlockContext;
use crate::fx::{Effect, EffectBlock, EffectInfo};
use crate::live::{NO_PORT, SlotLive, StripLive};
use crate::record::{CaptureSlot, StripCapture};

/// The length of the mute ramp, so muting never clicks.
const MUTE_RAMP_SECONDS: f64 = 0.005;

/// The first stage of a strip: input, polarity and swap.
#[derive(Debug)]
pub struct StripInput {
    live: Arc<StripLive>,
    capture: StripCapture,
}

impl StripInput {
    /// The input stage of the insert whose settings are `live`.
    pub fn new(live: Arc<StripLive>) -> Self {
        Self { live, capture: StripCapture::default() }
    }

    /// The same stage, recording the Input and PreFx taps through captures that arrive on `receiver`.
    pub fn with_capture(self, receiver: bodo_kit_core::handoff::Receiver<CaptureSlot>) -> Self {
        Self { capture: StripCapture::new(receiver), ..self }
    }

    /// Processes one block.
    ///
    /// RT-safe. Complexity: O(n) in the frames.
    #[rt_safe]
    pub fn process(&mut self, io: &mut NodeIo<'_>, context: &BlockContext) {
        self.capture.update();
        match io.inputs.first() {
            Some(input) => io.output.copy_from(input),
            None => io.output.clear(),
        }
        let Some((left, right)) = io.output.stereo_mut() else { return };
        // 0: an armed insert monitors its hardware input.
        if self.live.armed.load(Ordering::Relaxed)
            && let Some((from_left, from_right)) = self.live.input_channels()
        {
            for (out, input) in [(&mut *left, context.input(from_left)), (&mut *right, context.input(from_right))] {
                for (o, i) in out.iter_mut().zip(input) {
                    *o += *i;
                }
            }
        }
        self.capture.offer(RecordTap::Input, left, right, context);
        // 1-2: polarity and swap, before the FX, like a console preamp.
        if self.live.polarity_invert.load(Ordering::Relaxed) {
            invert_polarity(left);
            invert_polarity(right);
        }
        if self.live.swap_lr.load(Ordering::Relaxed) {
            swap_channels(left, right);
        }
        self.capture.offer(RecordTap::PreFx, left, right, context);
    }
}

/// An effect loaded in a slot, as the FX stage runs it.
pub struct SlotState {
    effect: Option<Box<dyn Effect>>,
    info: Option<&'static EffectInfo>,
    live: Option<Arc<SlotLive>>,
    /// This block's parameter values, in the effect's order.
    values: Box<[f32]>,
    /// The automation keys of the parameters, in the same order.
    param_keys: Box<[u64]>,
    mix_key: u64,
    enabled_key: u64,
    /// The dry path's delay by the slot's latency, one line per side; `None` for effects that never report any.
    dry: Option<[DelayLine; 2]>,
}

impl std::fmt::Debug for SlotState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlotState").field("effect", &self.info.map(|i| i.uid)).finish_non_exhaustive()
    }
}

impl Default for SlotState {
    fn default() -> Self {
        Self { effect: None, info: None, live: None, values: Box::new([]), param_keys: Box::new([]), mix_key: 0, enabled_key: 0, dry: None }
    }
}

impl SlotState {
    /// A slot running `effect`, of kind `info`, with settings `live` and automation keys for its parameters, mix and
    /// on switch.
    ///
    /// Allocates the dry delay for the effect's largest latency at `rate`.
    pub fn new(effect: Box<dyn Effect>, info: &'static EffectInfo, live: Arc<SlotLive>, keys: SlotKeys, rate: SampleRate) -> Self {
        let max = (info.max_latency)(rate) as usize;
        let dry = (max > 0).then(|| [DelayLine::new(max), DelayLine::new(max)]);
        Self {
            values: info.defaults().into_boxed_slice(),
            effect: Some(effect),
            info: Some(info),
            live: Some(live),
            param_keys: keys.params,
            mix_key: keys.mix,
            enabled_key: keys.enabled,
            dry,
        }
    }
}

/// The automation keys of a slot.
#[derive(Debug)]
pub struct SlotKeys {
    /// Each parameter's key, in the effect's order.
    pub params: Box<[u64]>,
    /// The dry/wet mix's key.
    pub mix: u64,
    /// The on switch's key.
    pub enabled: u64,
}

/// A change to one slot, sent while the strip runs; the old state comes back the same way, to be dropped off the
/// audio thread.
#[derive(Debug)]
pub struct SlotPatch {
    /// The slot, 0 to 9.
    pub index: usize,
    /// What the slot runs from now on.
    pub state: SlotState,
}

/// The queues an FX stage takes slot changes from and returns old slots on.
#[derive(Debug)]
pub struct SlotQueues {
    /// New slot states.
    pub patches: Consumer<Box<SlotPatch>>,
    /// Replaced slot states, for the control thread to drop.
    pub garbage: Producer<Box<SlotPatch>>,
}

/// The second stage of a strip: slots, EQ and stereo separation.
#[derive(Debug)]
pub struct StripFx {
    /// The insert's key, which its automated parameters are keyed by.
    key: u64,
    live: Arc<StripLive>,
    slots: [SlotState; FX_SLOTS],
    /// Slot changes. A mutex that is never locked: processors must be `Sync` and queue ends aren't, but the stage
    /// always has them to itself ([`Mutex::get_mut`]).
    queues: Mutex<Option<SlotQueues>>,
    /// The EQ: low shelf, peak and high shelf for each side.
    eq: [[Biquad; 3]; 2],
    eq_version: u32,
    eq_active: bool,
    /// The dry signal of the slot running, one buffer per side.
    dry: [Box<[f32]>; 2],
    capture: StripCapture,
}

impl StripFx {
    /// The FX stage of the insert with graph key `key`, its settings in `live`, its slots running `slots` from the
    /// start, and later changes arriving on `queues`. Blocks are at most `max_frames` long.
    ///
    /// Allocates.
    pub fn new(key: u64, live: Arc<StripLive>, slots: [SlotState; FX_SLOTS], queues: SlotQueues, max_frames: usize) -> Self {
        let identity = Biquad::new(BiquadCoeffs::IDENTITY);
        Self {
            key,
            live,
            slots,
            queues: Mutex::new(Some(queues)),
            eq: [[identity; 3]; 2],
            eq_version: u32::MAX,
            eq_active: false,
            dry: [vec![0.0; max_frames].into_boxed_slice(), vec![0.0; max_frames].into_boxed_slice()],
            capture: StripCapture::default(),
        }
    }

    /// The same stage, recording the PostFx tap through captures that arrive on `receiver`.
    pub fn with_capture(self, receiver: bodo_kit_core::handoff::Receiver<CaptureSlot>) -> Self {
        Self { capture: StripCapture::new(receiver), ..self }
    }

    /// Swaps in the slot changes that arrived, handing the replaced slots back, as many as the return queue has room
    /// for.
    ///
    /// RT-safe.
    fn take_patches(&mut self) {
        let Some(queues) = self.queues.get_mut().unwrap_or_else(PoisonError::into_inner).as_mut() else { return };
        while queues.garbage.free_slots() > 0 {
            let Some(mut patch) = queues.patches.pop() else { break };
            if let Some(slot) = self.slots.get_mut(patch.index) {
                core::mem::swap(slot, &mut patch.state);
            }
            if let Err(full) = queues.garbage.push(patch) {
                // Unreachable: room was checked. Keep the old state rather than free it here.
                core::mem::forget(full);
            }
        }
    }

    /// Redesigns the EQ when its settings changed.
    ///
    /// RT-safe.
    fn update_eq(&mut self, rate: SampleRate) {
        let version = self.live.eq.version.load(Ordering::Acquire);
        if version == self.eq_version {
            return;
        }
        self.eq_version = version;
        let [low, mid, high] = self.live.eq.get();
        let active = low[1] != 0.0 || mid[1] != 0.0 || high[1] != 0.0;
        let design = |kind, band: [f32; 3]| BiquadCoeffs::design(kind, f64::from(band[0].max(1.0)), f64::from(band[2].max(0.05)), rate);
        let coeffs = [
            design(BiquadKind::LowShelf { gain_db: f64::from(low[1]) }, low),
            design(BiquadKind::Peak { gain_db: f64::from(mid[1]) }, mid),
            design(BiquadKind::HighShelf { gain_db: f64::from(high[1]) }, high),
        ];
        for side in &mut self.eq {
            for (filter, c) in side.iter_mut().zip(coeffs) {
                if !self.eq_active {
                    filter.reset();
                }
                filter.set_coeffs(c);
            }
        }
        self.eq_active = active;
    }

    /// Processes one block.
    ///
    /// RT-safe. Complexity: O(n) in the frames, plus the effects' own work.
    #[rt_safe]
    pub fn process(&mut self, io: &mut NodeIo<'_>, context: &BlockContext) {
        self.capture.update();
        self.take_patches();
        self.update_eq(context.rate);
        match io.inputs.first() {
            Some(input) => io.output.copy_from(input),
            None => io.output.clear(),
        }
        let Some((left, right)) = io.output.stereo_mut() else { return };
        let frames = left.len().min(right.len());
        // 3: the slots, in order.
        let [dry_left, dry_right] = &mut self.dry;
        for slot in &mut self.slots {
            let (Some(effect), Some(info), Some(live)) = (slot.effect.as_mut(), slot.info, slot.live.as_ref()) else { continue };
            let enabled = context.params.get(slot.enabled_key).map_or_else(|| live.enabled.load(Ordering::Relaxed), |v| v >= 0.5);
            let mix = context.params.get(slot.mix_key).unwrap_or_else(|| live.mix.get()).clamp(0.0, 1.0);
            for (i, spec) in info.params.iter().enumerate() {
                let automated = slot.param_keys.get(i).and_then(|&key| context.params.get(key)).map(|n| spec.from_normalized(n));
                let value = automated.unwrap_or_else(|| live.params.get(i).map_or(spec.default, |cell| spec.clamp(cell.get())));
                if let Some(v) = slot.values.get_mut(i) {
                    *v = value;
                }
            }
            let latency = live.latency.load(Ordering::Relaxed) as usize;
            let (Some(dry_l), Some(dry_r)) = (dry_left.get_mut(..frames), dry_right.get_mut(..frames)) else { continue };
            // The dry path is delayed by the slot's latency, so partial mixes never comb-filter and a disabled slot
            // keeps its timing.
            let needs_dry = slot.dry.is_some() || !enabled || mix < 1.0;
            if needs_dry {
                dry_l.copy_from_slice(&left[..frames]);
                dry_r.copy_from_slice(&right[..frames]);
                if let Some([line_l, line_r]) = slot.dry.as_mut() {
                    line_l.process(dry_l, latency);
                    line_r.process(dry_r, latency);
                }
            }
            if enabled {
                let port = live.sidechain_port.load(Ordering::Relaxed);
                let sidechain = if port == NO_PORT {
                    None
                } else {
                    io.inputs.get(port as usize).filter(|b| b.channels() >= 2).map(|b| (b.channel(0), b.channel(1)))
                };
                let mut block = EffectBlock {
                    left: &mut left[..frames],
                    right: &mut right[..frames],
                    sidechain,
                    params: &slot.values,
                    rate: context.rate,
                    tempo: context.tempo,
                };
                effect.process(&mut block);
                if mix < 1.0 {
                    for ((wet_l, wet_r), (d_l, d_r)) in left.iter_mut().zip(right.iter_mut()).zip(dry_l.iter().zip(dry_r.iter())) {
                        *wet_l = (1.0 - mix) * *d_l + mix * *wet_l;
                        *wet_r = (1.0 - mix) * *d_r + mix * *wet_r;
                    }
                }
            } else {
                left[..frames].copy_from_slice(dry_l);
                right[..frames].copy_from_slice(dry_r);
            }
        }
        // 4: the strip EQ, skipped entirely when flat so the strip stays bit-exact.
        if self.eq_active {
            let [eq_left, eq_right] = &mut self.eq;
            for filter in eq_left {
                filter.process(left);
            }
            for filter in eq_right {
                filter.process(right);
            }
        }
        self.capture.offer(RecordTap::PostFx, left, right, context);
        // 5: stereo separation.
        let automated = |param, live: f32| context.params.get(param_key(self.key, param)).unwrap_or(live);
        stereo_separation(left, right, automated(Param::StereoSep, self.live.stereo_sep.get()).clamp(-1.0, 1.0));
    }
}

/// The last stage of a strip: pan, fader, mute, meters and the negative latency delay.
#[derive(Debug)]
pub struct StripOutput {
    /// The insert's key, which its automated parameters are keyed by.
    key: u64,
    live: Arc<StripLive>,
    fader: f32,
    mute: f32,
    pan: f32,
    /// Whether to measure the correlation, as only the master does.
    master: bool,
    /// The delay for a negative track latency, and how long it is.
    negative: Option<([DelayLine; 2], usize)>,
    capture: StripCapture,
}

impl StripOutput {
    /// The output stage of the insert with graph key `key`, its settings in `live`, starting where they are now.
    /// `delay` frames of negative track latency delay its output; `master` measures correlation.
    ///
    /// Allocates the delay.
    pub fn new(key: u64, live: Arc<StripLive>, delay: usize, master: bool) -> Self {
        let fader = db_to_gain(live.volume_db.get());
        let mute = if live.muted.load(Ordering::Relaxed) || live.silenced.load(Ordering::Relaxed) { 0.0 } else { 1.0 };
        let pan = live.pan.get();
        let negative = (delay > 0).then(|| ([DelayLine::new(delay), DelayLine::new(delay)], delay));
        Self { key, live, fader, mute, pan, master, negative, capture: StripCapture::default() }
    }

    /// The same stage, recording the PostFader tap through captures that arrive on `receiver`.
    pub fn with_capture(self, receiver: bodo_kit_core::handoff::Receiver<CaptureSlot>) -> Self {
        Self { capture: StripCapture::new(receiver), ..self }
    }

    /// Processes one block.
    ///
    /// RT-safe. Complexity: O(n) in the frames.
    #[rt_safe]
    pub fn process(&mut self, io: &mut NodeIo<'_>, context: &BlockContext) {
        self.capture.update();
        match io.inputs.first() {
            Some(input) => io.output.copy_from(input),
            None => io.output.clear(),
        }
        let Some((left, right)) = io.output.stereo_mut() else { return };
        // Automation, where it plays, stands in for the live settings.
        let automated = |param, live: f32| context.params.get(param_key(self.key, param)).unwrap_or(live);
        // 6: pan.
        let pan = automated(Param::Pan, self.live.pan.get()).clamp(-1.0, 1.0);
        apply_pan(left, right, self.pan, pan);
        self.pan = pan;
        // 7-8: the fader and the mute ramp, folded into one gain ramp; a strip outside the solo set mutes the same way.
        let fader = db_to_gain(automated(Param::Volume, self.live.volume_db.get()));
        let silent = self.live.muted.load(Ordering::Relaxed) || self.live.silenced.load(Ordering::Relaxed);
        let mute = step_toward(self.mute, if silent { 0.0 } else { 1.0 }, io.frames, context.rate);
        ramp_gain(left, self.fader * self.mute, fader * mute);
        ramp_gain(right, self.fader * self.mute, fader * mute);
        self.fader = fader;
        self.mute = mute;
        // 9: the post-fader point.
        self.live.meter.measure(left, right);
        if self.master {
            self.live.meter.measure_correlation(left, right);
        }
        self.capture.offer(RecordTap::PostFader, left, right, context);
        // 10: a negative track latency delays this strip.
        if let Some(([line_l, line_r], delay)) = self.negative.as_mut() {
            line_l.process(left, *delay);
            line_r.process(right, *delay);
        }
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
