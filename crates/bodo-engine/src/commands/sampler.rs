//! `sampler.*`: the built-in sampler's settings, everything its window shows (project.md section 10.2).
//!
//! One command sets any of them: fields left out keep their value, and nested groups (the envelopes, the filter, the
//! stretch, the precomputed effects, the arpeggiator) take only the fields given. Knob drags merge into one undo step
//! per channel.

use std::sync::Arc;

use bodo_kit_core::Seconds;
use bodo_kit_dsp::envelope::AdsrParams;
use bodo_model::channel::{ArpDirection, ChannelKind, Declick, FilterKind, LoopMode, ModEnvelope, SamplerParams, SamplerStretchMode};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{by_channel, in_range};
use crate::error::CommandError;
use crate::params::{ChannelRef, Key, Length};
use crate::registry::{Effects, Registry};
use crate::session::Ctx;

/// How the sample loops.
#[derive(Clone, Copy, Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
enum Loop {
    /// Plays once.
    Off,
    /// Loops forward between the loop points.
    Forward,
    /// Loops back and forth between the loop points.
    PingPong,
}

impl From<Loop> for LoopMode {
    fn from(mode: Loop) -> Self {
        match mode {
            Loop::Off => LoopMode::Off,
            Loop::Forward => LoopMode::Forward,
            Loop::PingPong => LoopMode::PingPong,
        }
    }
}

/// Short fades against clicks.
#[derive(Clone, Copy, Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
enum DeclickMode {
    /// None.
    Off,
    /// A sharp attack and a 2 ms fade-out, for drums.
    Transient,
    /// 1 ms in, 3 ms out.
    Generic,
    /// 10 ms in and out, for pads and loops.
    Smooth,
}

impl From<DeclickMode> for Declick {
    fn from(mode: DeclickMode) -> Self {
        match mode {
            DeclickMode::Off => Declick::Off,
            DeclickMode::Transient => Declick::Transient,
            DeclickMode::Generic => Declick::Generic,
            DeclickMode::Smooth => Declick::Smooth,
        }
    }
}

/// How the sampler changes a sample's length.
#[derive(Clone, Copy, Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
enum StretchKind {
    /// Like tape: longer is lower.
    Resample,
    /// Length and pitch apart.
    Stretch,
}

impl From<StretchKind> for SamplerStretchMode {
    fn from(mode: StretchKind) -> Self {
        match mode {
            StretchKind::Resample => SamplerStretchMode::Resample,
            StretchKind::Stretch => SamplerStretchMode::Stretch,
        }
    }
}

/// The filter's response.
#[derive(Clone, Copy, Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
enum Filter {
    /// No filter.
    Off,
    /// Low pass.
    LowPass,
    /// High pass.
    HighPass,
    /// Band pass.
    BandPass,
    /// Notch.
    Notch,
}

impl From<Filter> for FilterKind {
    fn from(kind: Filter) -> Self {
        match kind {
            Filter::Off => FilterKind::Off,
            Filter::LowPass => FilterKind::LowPass,
            Filter::HighPass => FilterKind::HighPass,
            Filter::BandPass => FilterKind::BandPass,
            Filter::Notch => FilterKind::Notch,
        }
    }
}

/// The order of an arpeggio.
#[derive(Clone, Copy, Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
enum Direction {
    /// Lowest to highest.
    Up,
    /// Highest to lowest.
    Down,
    /// Up then down.
    UpDown,
    /// Down then up.
    DownUp,
    /// A random key each step.
    Random,
    /// In the order the keys were pressed.
    AsPlayed,
}

impl From<Direction> for ArpDirection {
    fn from(direction: Direction) -> Self {
        match direction {
            Direction::Up => ArpDirection::Up,
            Direction::Down => ArpDirection::Down,
            Direction::UpDown => ArpDirection::UpDown,
            Direction::DownUp => ArpDirection::DownUp,
            Direction::Random => ArpDirection::Random,
            Direction::AsPlayed => ArpDirection::AsPlayed,
        }
    }
}

/// Envelope times in seconds and a sustain level; fields left out keep their value.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Envelope {
    /// Whether a filter or pitch envelope moves anything; the volume envelope is always on.
    enabled: Option<bool>,
    /// Seconds to rise to full level, 0 to 10.
    attack: Option<f64>,
    /// Seconds to fall to the sustain level, 0 to 10.
    decay: Option<f64>,
    /// The level held while the note is held, 0 to 1.
    sustain: Option<f32>,
    /// Seconds to fall to silence after the note ends, 0 to 10.
    release: Option<f64>,
    /// How far a filter envelope moves the cutoff (octaves, -8 to 8) or a pitch envelope the pitch (semitones, -48
    /// to 48). The volume envelope has none.
    amount: Option<f32>,
}

impl Envelope {
    fn apply(&self, adsr: &mut AdsrParams) -> Result<(), CommandError> {
        for (name, value, slot) in
            [("attack", self.attack, &mut adsr.attack), ("decay", self.decay, &mut adsr.decay), ("release", self.release, &mut adsr.release)]
        {
            if let Some(value) = value {
                in_range(name, value, 0.0..=10.0)?;
                *slot = Seconds(value);
            }
        }
        if let Some(sustain) = self.sustain {
            in_range("sustain", f64::from(sustain), 0.0..=1.0)?;
            adsr.sustain = sustain;
        }
        Ok(())
    }

    fn apply_mod(&self, envelope: &mut ModEnvelope, range: f32) -> Result<(), CommandError> {
        self.apply(&mut envelope.adsr)?;
        if let Some(enabled) = self.enabled {
            envelope.enabled = enabled;
        }
        if let Some(amount) = self.amount {
            in_range("amount", f64::from(amount), -f64::from(range)..=f64::from(range))?;
            envelope.amount = amount;
        }
        Ok(())
    }
}

/// Time stretching; fields left out keep their value.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Stretch {
    /// Resample (tape) or stretch (length and pitch apart).
    mode: Option<StretchKind>,
    /// Pitch shift in cents, -2400 to 2400.
    pitch_cents: Option<f32>,
    /// How many times longer the sample plays, 0.05 to 20.
    multiplier: Option<f32>,
    /// Fits the played part to this many beats at the project's tempo; 0 goes back to the multiplier.
    time_beats: Option<f64>,
}

/// Precomputed effects; fields left out keep their value.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SampleFx {
    /// Cuts silence below -60 dB from both ends.
    trim: Option<bool>,
    /// Removes DC offset.
    remove_dc: Option<bool>,
    /// Brings the peak to 0 dBFS.
    normalize: Option<bool>,
    /// Inverts the waveform.
    reverse_polarity: Option<bool>,
    /// Swaps left and right.
    swap_stereo: Option<bool>,
    /// Moves the sound from left to right over its length.
    fade_stereo: Option<bool>,
    /// Fade-in in seconds, 0 to 10.
    fade_in: Option<f32>,
    /// Fade-out in seconds, 0 to 10.
    fade_out: Option<f32>,
    /// Loop crossfade, 0 to 0.5 of the loop.
    crossfade: Option<f32>,
}

/// The filter; fields left out keep their value.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FilterSettings {
    /// The response.
    kind: Option<Filter>,
    /// Cutoff in Hz, 20 to 20 000.
    cutoff: Option<f32>,
    /// Resonance, 0 to 1.
    resonance: Option<f32>,
    /// How far the cutoff follows the key, 0 to 1.
    key_tracking: Option<f32>,
}

/// The arpeggiator; fields left out keep their value.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Arp {
    /// Whether it plays.
    enabled: Option<bool>,
    /// The order.
    direction: Option<Direction>,
    /// Time from one note to the next, such as "1 step" or 120 (ticks).
    rate: Option<Length>,
    /// How long each note lasts, as a fraction of the rate, 0.01 to 1.
    gate: Option<f32>,
    /// Octaves the pattern climbs, 1 to 4.
    octaves: Option<u8>,
    /// Times each note plays before the next, 1 to 8.
    repeat: Option<u8>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Set {
    /// The sampler channel; the selected channel by default.
    channel: Option<ChannelRef>,
    /// The key that plays the sample at its own pitch, such as "C4" or 60.
    root_key: Option<Key>,
    /// The lowest key it plays.
    low_key: Option<Key>,
    /// The highest key it plays.
    high_key: Option<Key>,
    /// Where the played part starts, 0 to 1 of the sample.
    start: Option<f32>,
    /// Where the played part ends, 0 to 1 of the sample.
    end: Option<f32>,
    /// Where notes start inside the played part, 0 to 1 of it.
    start_offset: Option<f32>,
    /// How the sample loops.
    loop_mode: Option<Loop>,
    /// The loop start, 0 to 1 of the sample.
    loop_start: Option<f32>,
    /// The loop end, 0 to 1 of the sample.
    loop_end: Option<f32>,
    /// Plays the sample backwards.
    reverse: Option<bool>,
    /// Plays the whole sample on every note however short the note.
    one_shot: Option<bool>,
    /// Streams the sample from disk where the playback allows it.
    keep_on_disk: Option<bool>,
    /// Converts the sample to the project's rate when it loads.
    resample: Option<bool>,
    /// Plays the file's regions, one per key from the root key.
    load_regions: Option<bool>,
    /// Plays the slices between the file's markers, one per key from the root key.
    load_slices: Option<bool>,
    /// The most voices at once, 1 to 256.
    polyphony: Option<u16>,
    /// How far the pitch knob reaches, 1 to 48 semitones.
    pitch_range: Option<f32>,
    /// Fades against clicks.
    declick: Option<DeclickMode>,
    /// The volume envelope.
    envelope: Option<Envelope>,
    /// The filter.
    filter: Option<FilterSettings>,
    /// The filter envelope.
    filter_envelope: Option<Envelope>,
    /// The pitch envelope.
    pitch_envelope: Option<Envelope>,
    /// Time stretching.
    stretch: Option<Stretch>,
    /// Effects rendered into a copy of the sample.
    effects: Option<SampleFx>,
    /// The arpeggiator.
    arp: Option<Arp>,
}

/// A fraction from 0 to 1.
fn fraction(name: &str, value: Option<f32>, slot: &mut f32) -> Result<(), CommandError> {
    if let Some(value) = value {
        in_range(name, f64::from(value), 0.0..=1.0)?;
        *slot = value;
    }
    Ok(())
}

fn set_flag(value: Option<bool>, slot: &mut bool) {
    if let Some(value) = value {
        *slot = value;
    }
}

/// `params` with the settings of `p` applied.
fn apply(ctx: &Ctx<'_>, mut params: SamplerParams, p: &Set) -> Result<SamplerParams, CommandError> {
    if let Some(key) = &p.root_key {
        params.root_key = key.resolve()?;
    }
    if let Some(key) = &p.low_key {
        params.key_range.0 = key.resolve()?;
    }
    if let Some(key) = &p.high_key {
        params.key_range.1 = key.resolve()?;
    }
    if params.key_range.0 > params.key_range.1 {
        return Err(CommandError::invalid("the lowest key must not be above the highest"));
    }
    fraction("start", p.start, &mut params.start)?;
    fraction("end", p.end, &mut params.end)?;
    fraction("startOffset", p.start_offset, &mut params.start_offset)?;
    fraction("loopStart", p.loop_start, &mut params.loop_start)?;
    fraction("loopEnd", p.loop_end, &mut params.loop_end)?;
    if params.start >= params.end {
        return Err(CommandError::invalid("the start must come before the end"));
    }
    if params.loop_start >= params.loop_end {
        return Err(CommandError::invalid("the loop start must come before the loop end"));
    }
    if let Some(mode) = p.loop_mode {
        params.loop_mode = mode.into();
    }
    set_flag(p.reverse, &mut params.reverse);
    set_flag(p.one_shot, &mut params.one_shot);
    set_flag(p.keep_on_disk, &mut params.keep_on_disk);
    set_flag(p.resample, &mut params.resample);
    set_flag(p.load_regions, &mut params.load_regions);
    set_flag(p.load_slices, &mut params.load_slices);
    if let Some(polyphony) = p.polyphony {
        in_range("polyphony", f64::from(polyphony), 1.0..=256.0)?;
        params.polyphony = polyphony;
    }
    if let Some(range) = p.pitch_range {
        in_range("pitchRange", f64::from(range), 1.0..=48.0)?;
        params.pitch_range = range;
    }
    if let Some(declick) = p.declick {
        params.declick = declick.into();
    }
    if let Some(envelope) = &p.envelope {
        envelope.apply(&mut params.envelope)?;
    }
    if let Some(envelope) = &p.filter_envelope {
        envelope.apply_mod(&mut params.filter_envelope, 8.0)?;
    }
    if let Some(envelope) = &p.pitch_envelope {
        envelope.apply_mod(&mut params.pitch_envelope, 48.0)?;
    }
    if let Some(filter) = &p.filter {
        if let Some(kind) = filter.kind {
            params.filter.kind = kind.into();
        }
        if let Some(cutoff) = filter.cutoff {
            in_range("cutoff", f64::from(cutoff), 20.0..=20_000.0)?;
            params.filter.cutoff = cutoff;
        }
        fraction("resonance", filter.resonance, &mut params.filter.resonance)?;
        fraction("keyTracking", filter.key_tracking, &mut params.filter.key_tracking)?;
    }
    if let Some(stretch) = &p.stretch {
        if let Some(mode) = stretch.mode {
            params.stretch.mode = mode.into();
        }
        if let Some(cents) = stretch.pitch_cents {
            in_range("pitchCents", f64::from(cents), -2400.0..=2400.0)?;
            params.stretch.pitch_cents = cents;
        }
        if let Some(multiplier) = stretch.multiplier {
            in_range("multiplier", f64::from(multiplier), 0.05..=20.0)?;
            params.stretch.multiplier = multiplier;
        }
        if let Some(beats) = stretch.time_beats {
            in_range("timeBeats", beats, 0.0..=1024.0)?;
            params.stretch.time_beats = (beats > 0.0).then_some(beats);
        }
    }
    if let Some(fx) = &p.effects {
        let effects = &mut params.effects;
        set_flag(fx.trim, &mut effects.trim);
        set_flag(fx.remove_dc, &mut effects.remove_dc);
        set_flag(fx.normalize, &mut effects.normalize);
        set_flag(fx.reverse_polarity, &mut effects.reverse_polarity);
        set_flag(fx.swap_stereo, &mut effects.swap_stereo);
        set_flag(fx.fade_stereo, &mut effects.fade_stereo);
        for (name, value, slot) in [("fadeIn", fx.fade_in, &mut effects.fade_in), ("fadeOut", fx.fade_out, &mut effects.fade_out)] {
            if let Some(value) = value {
                in_range(name, f64::from(value), 0.0..=10.0)?;
                *slot = value;
            }
        }
        if let Some(crossfade) = fx.crossfade {
            in_range("crossfade", f64::from(crossfade), 0.0..=0.5)?;
            effects.crossfade = crossfade;
        }
    }
    if let Some(arp) = &p.arp {
        set_flag(arp.enabled, &mut params.arp.enabled);
        if let Some(direction) = arp.direction {
            params.arp.direction = direction.into();
        }
        if let Some(rate) = &arp.rate {
            let ticks = rate.resolve(&ctx.project, bodo_kit_core::Tick::ZERO)?.0;
            in_range("rate", ticks as f64, 1.0..=15_360.0)?;
            params.arp.rate = ticks;
        }
        if let Some(gate) = arp.gate {
            in_range("gate", f64::from(gate), 0.01..=1.0)?;
            params.arp.gate = gate;
        }
        if let Some(octaves) = arp.octaves {
            in_range("octaves", f64::from(octaves), 1.0..=4.0)?;
            params.arp.octaves = octaves;
        }
        if let Some(repeat) = arp.repeat {
            in_range("repeat", f64::from(repeat), 1.0..=8.0)?;
            params.arp.repeat = repeat;
        }
    }
    Ok(params)
}

fn set(ctx: &mut Ctx<'_>, p: Set) -> Result<Value, CommandError> {
    let id = ctx.channel(p.channel.as_ref())?;
    let channel = ctx.project.channels.get(&id).cloned().ok_or_else(|| CommandError::not_found("channel", &id.to_string()))?;
    let ChannelKind::Sampler(params) = &channel.kind else {
        return Err(CommandError::rejected(format!("{} is not a sampler channel", channel.name)));
    };
    let params = apply(ctx, (**params).clone(), &p)?;
    let result = serde_json::to_value(&params).unwrap_or(Value::Null);
    ctx.edit_channel(id, |c| c.kind = ChannelKind::Sampler(Arc::new(params)))?;
    ctx.set_label(format!("Edit sampler {}", channel.name));
    Ok(json!({ "channel": id, "sampler": result }))
}

pub(super) fn register(r: &mut Registry) {
    r.edit(
        "sampler.set",
        "Edit sampler",
        "Sets any of a sampler channel's settings: the played part and loop, content options, declicking, envelopes, filter, stretch, precomputed effects and arpeggiator.",
        Effects::UNDOABLE,
        set,
    )
    .example(json!({
        "channel": "Kick",
        "end": 0.75,
        "loopMode": "pingPong",
        "loopStart": 0.25,
        "loopEnd": 0.5,
        "oneShot": false,
        "declick": "smooth",
        "filter": { "kind": "lowPass", "cutoff": 800, "resonance": 0.3 },
        "filterEnvelope": { "enabled": true, "decay": 0.4, "amount": 3 },
        "effects": { "normalize": true, "fadeOut": 0.05 },
        "arp": { "enabled": true, "direction": "upDown", "rate": "1 step" }
    }))
    .merge(by_channel);
}
