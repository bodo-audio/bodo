//! Automation lanes and what they can control (project.md section 10.5).

use bodo_kit_core::Tick;
use imbl::Vector;

use crate::id::{ChannelId, InsertId, PluginInstanceId};
use crate::mixer::RouteKey;

/// How the value moves from a point to the next one.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", tag = "type", content = "tension")]
pub enum CurveShape {
    /// Holds the value until the next point.
    Hold,
    /// A straight line.
    #[default]
    Linear,
    /// An exponential curve; the tension bends it, -1 to 1.
    Exp(f32),
    /// An S curve; the tension sets its steepness, -1 to 1.
    SCurve(f32),
    /// A staircase with this many steps.
    Stairs(u8),
    /// Half a sine wave.
    Sine,
    /// A square pulse.
    Pulse,
}

/// A point of an automation lane.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoPoint {
    /// Where the point is.
    pub at: Tick,
    /// The value, normalized to 0..=1.
    pub value: f32,
    /// The curve from this point to the next.
    #[serde(default)]
    pub curve: CurveShape,
}

/// A mixer insert parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InsertParam {
    /// The fader.
    Volume,
    /// Pan.
    Pan,
    /// Stereo separation.
    StereoSep,
}

/// An FX slot parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SlotParam {
    /// Dry/wet mix.
    Mix,
    /// On or off.
    Enabled,
}

/// A channel parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChannelParam {
    /// Volume.
    Volume,
    /// Pan.
    Pan,
    /// Pitch.
    Pitch,
    /// A MIDI control change, 0 to 127, sent to the channel's instrument.
    Cc(u8),
    /// MIDI pitch bend.
    PitchBend,
    /// MIDI channel pressure (aftertouch).
    Pressure,
}

/// Something automation can control.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ParamTarget {
    /// A parameter of a mixer insert.
    Insert(InsertId, InsertParam),
    /// A parameter of an FX slot: the insert, the slot index 0 to 9, and the parameter.
    Slot(InsertId, u8, SlotParam),
    /// A plugin parameter, by its CLAP or VST3 id.
    Plugin(PluginInstanceId, u32),
    /// A parameter of a channel.
    Channel(ChannelId, ChannelParam),
    /// The send level of a route.
    Route(RouteKey),
    /// The project tempo.
    Tempo,
}

/// A target and its points over time.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationLane {
    /// What the lane controls.
    pub target: ParamTarget,
    /// The points, sorted by time.
    pub points: Vector<AutoPoint>,
    /// The value before the first point, normalized to 0..=1.
    pub default: f32,
}
