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

/// Settings of the built-in sampler.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SamplerParams {
    /// The sample it plays.
    pub sample: Option<SampleId>,
    /// The key that plays the sample at its own pitch.
    pub root_key: u8,
    /// The lowest and highest keys it responds to.
    pub key_range: (u8, u8),
    /// The amplitude envelope.
    pub envelope: AdsrParams,
    /// Where playback starts, as a fraction of the sample.
    pub start: f32,
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
    /// Streams the sample from disk rather than loading it into memory.
    pub keep_on_disk: bool,
    /// The most voices it plays at once.
    pub polyphony: u16,
}

impl Default for SamplerParams {
    fn default() -> Self {
        Self {
            sample: None,
            root_key: 60,
            key_range: (0, 127),
            envelope: AdsrParams { attack: Seconds(0.0), decay: Seconds(0.0), sustain: 1.0, release: Seconds(0.01) },
            start: 0.0,
            loop_mode: LoopMode::Off,
            loop_start: 0.0,
            loop_end: 1.0,
            reverse: false,
            one_shot: true,
            keep_on_disk: false,
            polyphony: 32,
        }
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
