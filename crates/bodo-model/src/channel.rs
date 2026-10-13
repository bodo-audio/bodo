//! Channels of the channel rack (project.md sections 8.3 and 10.2).

use std::sync::Arc;

use bodo_kit_core::Seconds;
use bodo_kit_dsp::envelope::AdsrParams;
use imbl::Vector;

use crate::automation::AutomationLane;
use crate::color::Color;
use crate::id::{ChannelId, InsertId, PluginInstanceId, SampleId};

/// A channel: a sound source in the channel rack.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    /// The channel's id.
    pub id: ChannelId,
    /// Its name.
    pub name: String,
    /// Its color.
    pub color: Color,
    /// What kind of source it is.
    pub kind: ChannelKind,
    /// The mixer insert it plays into, or `None` for the master.
    #[serde(default)]
    pub target: Option<InsertId>,
    /// Volume as a linear gain; 1.0 is unity.
    #[serde(default = "unity")]
    pub volume: f32,
    /// Pan, -1 (left) to 1 (right).
    #[serde(default)]
    pub pan: f32,
    /// Pitch shift in cents.
    #[serde(default)]
    pub pitch_cents: f32,
    /// How late every second step plays, 0 to 1 (project.md section 10.2).
    #[serde(default)]
    pub swing: f32,
    /// Whether the channel is muted.
    #[serde(default)]
    pub muted: bool,
    /// A MIDI input that plays the channel.
    #[serde(default)]
    pub midi_in: Option<MidiBinding>,
    /// The channel rack group it belongs to, by name; `None` leaves it unsorted (project.md section 10.2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

fn unity() -> f32 {
    1.0
}

/// What a channel plays.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum ChannelKind {
    /// The built-in sampler.
    Sampler(Arc<SamplerParams>),
    /// An instrument plugin.
    Instrument {
        /// The plugin.
        plugin: PluginRef,
    },
    /// An audio clip channel, whose clips play a sample on the playlist.
    AudioClip {
        /// The sample.
        sample: SampleId,
        /// How the sample follows tempo changes.
        #[serde(default)]
        stretch: StretchMode,
        /// The sample's own tempo in BPM, which Resample and Stretch match to the project's. Bodo records the project's
        /// tempo here when the channel is made, unless the sample's tempo is known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tempo: Option<f64>,
    },
    /// An automation clip channel.
    Automation(Arc<AutomationLane>),
    /// A layer: one trigger plays every child channel.
    Layer {
        /// The children.
        children: Vector<ChannelId>,
    },
    /// Sends MIDI to an external port.
    MidiOut {
        /// The port's name.
        port: String,
        /// The MIDI channel, 0 to 15.
        channel: u8,
    },
}

/// How audio follows the project tempo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StretchMode {
    /// Plays at its own speed.
    #[default]
    None,
    /// Changes speed and pitch together, like tape.
    Resample,
    /// Stretches time and keeps the pitch.
    Stretch,
}

/// How a sampler loops its sample.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LoopMode {
    /// Plays once.
    #[default]
    Off,
    /// Loops forward between the loop points.
    Forward,
    /// Loops back and forth between the loop points.
    PingPong,
}

/// Settings of the built-in sampler (project.md section 10.2).
///
/// Positions (start, end, loop points) are fractions of the sample as the sampler plays it: after the precomputed
/// effects, the reverse and any time stretch.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SamplerParams {
    /// The sample it plays.
    pub sample: Option<SampleId>,
    /// The key that plays the sample at its own pitch; with slices or regions, the key of the first one.
    pub root_key: u8,
    /// The lowest and highest keys it responds to.
    pub key_range: (u8, u8),
    /// The volume envelope.
    pub envelope: AdsrParams,
    /// Where the played part of the sample starts, as a fraction of the sample.
    pub start: f32,
    /// Where the played part of the sample ends, as a fraction of the sample.
    pub end: f32,
    /// Where each note starts inside the played part, as a fraction of it.
    pub start_offset: f32,
    /// How it loops.
    pub loop_mode: LoopMode,
    /// The loop start, as a fraction of the sample.
    pub loop_start: f32,
    /// The loop end, as a fraction of the sample.
    pub loop_end: f32,
    /// Plays the sample backwards.
    pub reverse: bool,
    /// Plays the whole sample on every note, however short the note is, as drum one-shots should. A looping sample
    /// always stops with its note.
    pub one_shot: bool,
    /// Streams the sample from disk rather than loading it into memory, where the playback allows it.
    pub keep_on_disk: bool,
    /// Converts the sample to the project's rate when it loads. Off, it plays at its own rate, read faster or slower.
    pub resample: bool,
    /// Plays the regions marked in the file, one per key from the root key up.
    pub load_regions: bool,
    /// Plays the slices between the markers in the file, one per key from the root key up.
    pub load_slices: bool,
    /// The most voices it plays at once.
    pub polyphony: u16,
    /// How far the channel's pitch knob reaches, in semitones either way.
    pub pitch_range: f32,
    /// Short fades that keep notes from clicking.
    pub declick: Declick,
    /// Time stretching.
    pub stretch: SamplerStretch,
    /// Effects rendered into a copy of the sample.
    pub effects: SampleEffects,
    /// The filter every voice goes through.
    pub filter: SamplerFilter,
    /// The envelope that moves the filter's cutoff.
    pub filter_envelope: ModEnvelope,
    /// The envelope that moves the pitch.
    pub pitch_envelope: ModEnvelope,
    /// The arpeggiator.
    pub arp: Arpeggiator,
}

impl Default for SamplerParams {
    fn default() -> Self {
        Self {
            sample: None,
            root_key: 60,
            key_range: (0, 127),
            envelope: AdsrParams { attack: Seconds(0.0), decay: Seconds(0.0), sustain: 1.0, release: Seconds(0.01) },
            start: 0.0,
            end: 1.0,
            start_offset: 0.0,
            loop_mode: LoopMode::Off,
            loop_start: 0.0,
            loop_end: 1.0,
            reverse: false,
            one_shot: true,
            keep_on_disk: false,
            resample: true,
            load_regions: false,
            load_slices: false,
            polyphony: 32,
            pitch_range: 2.0,
            declick: Declick::Off,
            stretch: SamplerStretch::default(),
            effects: SampleEffects::default(),
            filter: SamplerFilter::default(),
            filter_envelope: ModEnvelope::default(),
            pitch_envelope: ModEnvelope::default(),
            arp: Arpeggiator::default(),
        }
    }
}

/// Short fades at a note's start and end that keep it from clicking.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Declick {
    /// No fades.
    #[default]
    Off,
    /// Keeps the attack sharp and fades out over 2 ms, for drums.
    Transient,
    /// Fades in over 1 ms and out over 3 ms.
    Generic,
    /// Fades in and out over 10 ms, for pads and loops.
    Smooth,
}

impl Declick {
    /// Every mode, in the menu's order.
    pub const ALL: [Self; 4] = [Self::Off, Self::Transient, Self::Generic, Self::Smooth];

    /// The fade-in and fade-out times in seconds.
    pub fn times(self) -> (f64, f64) {
        match self {
            Self::Off => (0.0, 0.0),
            Self::Transient => (0.0, 0.002),
            Self::Generic => (0.001, 0.003),
            Self::Smooth => (0.010, 0.010),
        }
    }
}

/// How the sampler changes a sample's speed and length.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SamplerStretchMode {
    /// Like tape: a longer sample also plays lower.
    #[default]
    Resample,
    /// Changes length and pitch apart, with a time stretcher.
    Stretch,
}

/// The sampler's time stretching.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SamplerStretch {
    /// Resample or stretch.
    pub mode: SamplerStretchMode,
    /// A pitch shift in cents. In Resample mode it also changes the speed.
    pub pitch_cents: f32,
    /// How many times longer the sample plays: 2 is twice as long.
    pub multiplier: f32,
    /// Fits the played part to this many beats at the project's tempo, in place of the multiplier.
    pub time_beats: Option<f64>,
}

impl Default for SamplerStretch {
    fn default() -> Self {
        Self { mode: SamplerStretchMode::Resample, pitch_cents: 0.0, multiplier: 1.0, time_beats: None }
    }
}

impl SamplerStretch {
    /// Whether it leaves the sample as it is.
    pub fn is_identity(&self) -> bool {
        self.pitch_cents == 0.0 && (self.multiplier - 1.0).abs() < 1e-6 && self.time_beats.is_none()
    }
}

/// Effects rendered once into a copy of the sample (project.md section 10.2). The file itself never changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SampleEffects {
    /// Cuts silence below -60 dB from both ends.
    pub trim: bool,
    /// Removes any DC offset.
    pub remove_dc: bool,
    /// Scales the sample so its peak is at 0 dBFS.
    pub normalize: bool,
    /// Inverts the waveform.
    pub reverse_polarity: bool,
    /// Swaps left and right.
    pub swap_stereo: bool,
    /// Moves the sound from left to right over its length.
    pub fade_stereo: bool,
    /// A fade-in at the start, in seconds.
    pub fade_in: f32,
    /// A fade-out at the end, in seconds.
    pub fade_out: f32,
    /// Crossfades the loop's end into the audio before its start, as a fraction of the loop, up to 0.5.
    pub crossfade: f32,
}

impl SampleEffects {
    /// Whether it leaves the sample as it is.
    pub fn is_identity(&self) -> bool {
        *self == Self::default()
    }
}

/// The sampler filter's response.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FilterKind {
    /// No filter.
    #[default]
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

impl FilterKind {
    /// Every kind, in the menu's order.
    pub const ALL: [Self; 5] = [Self::Off, Self::LowPass, Self::HighPass, Self::BandPass, Self::Notch];
}

/// The filter every sampler voice goes through.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SamplerFilter {
    /// The response.
    pub kind: FilterKind,
    /// The cutoff in Hz, 20 to 20 000.
    pub cutoff: f32,
    /// Resonance, 0 to 1.
    pub resonance: f32,
    /// How far the cutoff follows the key, 0 to 1: at 1 it moves an octave per octave from C4.
    pub key_tracking: f32,
}

impl Default for SamplerFilter {
    fn default() -> Self {
        Self { kind: FilterKind::Off, cutoff: 20_000.0, resonance: 0.0, key_tracking: 0.0 }
    }
}

/// An envelope that moves a parameter: the filter's cutoff in octaves, or the pitch in semitones.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ModEnvelope {
    /// Whether it moves anything.
    pub enabled: bool,
    /// Its shape.
    pub adsr: AdsrParams,
    /// How far it moves the parameter at full level: octaves for the filter, semitones for the pitch.
    pub amount: f32,
}

impl Default for ModEnvelope {
    fn default() -> Self {
        Self { enabled: false, adsr: AdsrParams { attack: Seconds(0.0), decay: Seconds(0.3), sustain: 0.0, release: Seconds(0.1) }, amount: 0.0 }
    }
}

/// The order an arpeggio plays the held keys in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ArpDirection {
    /// Lowest to highest.
    #[default]
    Up,
    /// Highest to lowest.
    Down,
    /// Up, then down, without repeating the ends.
    UpDown,
    /// Down, then up, without repeating the ends.
    DownUp,
    /// A random key each step.
    Random,
    /// In the order the keys were pressed.
    AsPlayed,
}

impl ArpDirection {
    /// Every direction, in the menu's order.
    pub const ALL: [Self; 6] = [Self::Up, Self::Down, Self::UpDown, Self::DownUp, Self::Random, Self::AsPlayed];
}

/// The sampler's arpeggiator: held keys play one after another, in time with the song.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Arpeggiator {
    /// Whether it plays.
    pub enabled: bool,
    /// The order.
    pub direction: ArpDirection,
    /// Ticks from one note to the next; a step is 240.
    pub rate: i64,
    /// How long each note lasts, as a fraction of the rate.
    pub gate: f32,
    /// How many octaves up the pattern climbs, 1 to 4.
    pub octaves: u8,
    /// How many times each note plays before the next, 1 to 8.
    pub repeat: u8,
}

impl Default for Arpeggiator {
    fn default() -> Self {
        Self { enabled: false, direction: ArpDirection::Up, rate: 240, gate: 0.5, octaves: 1, repeat: 1 }
    }
}

/// A plugin used by a channel or an FX slot.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginRef {
    /// The instance's id.
    pub instance: PluginInstanceId,
    /// Its format.
    pub format: PluginFormat,
    /// The format's unique id for the plugin, such as a CLAP id.
    pub uid: String,
    /// The plugin's saved state. Project files keep it outside the JSON, in `plugins/<instance>.state`.
    #[serde(skip)]
    pub state: Arc<[u8]>,
    /// Where the plugin runs.
    #[serde(default)]
    pub sandbox: SandboxMode,
}

/// A plugin format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PluginFormat {
    /// A Bodo Kit plugin linked into Bodo, such as a stock plugin.
    Native,
    /// CLAP.
    Clap,
    /// VST3.
    Vst3,
    /// Audio Unit.
    Au,
    /// LV2.
    Lv2,
}

/// Where a plugin instance runs (project.md section 7.3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SandboxMode {
    /// In Bodo's process.
    InProcess,
    /// In the bridge process all third-party plugins share. The default.
    #[default]
    SharedBridge,
    /// In a bridge process per vendor.
    PerVendor,
    /// In a bridge process of its own.
    PerPlugin,
}

/// A MIDI input bound to a channel.
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MidiBinding {
    /// The input port's name.
    pub port: String,
    /// The MIDI channel it listens to, 0 to 15, or `None` for all.
    pub channel: Option<u8>,
}
