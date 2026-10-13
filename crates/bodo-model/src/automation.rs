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

impl CurveShape {
    /// How far from one point's value to the next the curve is, a fraction `u` (0 to 1) of the way along.
    ///
    /// - Hold keeps the first value until the next point.
    /// - Exp bends the line: its tension (-1 to 1) sets how far, with positive values rising late.
    /// - SCurve eases in and out, steeper as its tension rises from -1 to 1.
    /// - Stairs climbs in that many equal steps; Sine follows half a cosine; Pulse jumps halfway.
    ///
    /// RT-safe.
    pub fn ease(self, u: f64) -> f64 {
        let u = u.clamp(0.0, 1.0);
        match self {
            Self::Hold => 0.0,
            Self::Linear => u,
            Self::Exp(tension) => {
                let s = 6.0 * f64::from(tension.clamp(-1.0, 1.0));
                if s.abs() < 1e-3 { u } else { (s * u).exp_m1() / s.exp_m1() }
            }
            Self::SCurve(tension) => {
                let p = 1.0 + 2.0 * (f64::from(tension.clamp(-1.0, 1.0)) + 1.0);
                let (a, b) = (u.powf(p), (1.0 - u).powf(p));
                if a + b > 0.0 { a / (a + b) } else { u }
            }
            Self::Stairs(steps) => {
                let n = f64::from(steps.max(1));
                (u * n).floor() / n
            }
            Self::Sine => 0.5 - 0.5 * (core::f64::consts::PI * u).cos(),
            Self::Pulse => {
                if u < 0.5 {
                    0.0
                } else {
                    1.0
                }
            }
        }
    }

    /// The value between `from` and `to` a fraction `u` of the way along a segment with this curve.
    ///
    /// RT-safe.
    pub fn between(self, from: f32, to: f32, u: f64) -> f32 {
        from + (to - from) * self.ease(u) as f32
    }
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

/// Where a normalized automation value of a volume is unity gain (0 dB).
pub const UNITY: f32 = 0.8;

/// The gain a normalized volume value `n` stands for: `(n / 0.8)^3`, so 0 is silence, 0.8 is unity (0 dB) and 1 is
/// about +5.8 dB. A cubic law spreads the useful range over the whole travel, like a fader.
pub fn volume_gain(n: f32) -> f32 {
    (n.clamp(0.0, 1.0) / UNITY).powi(3)
}

/// The normalized value of the volume gain `gain`; the inverse of [`volume_gain`].
pub fn volume_normalized(gain: f32) -> f32 {
    (UNITY * gain.max(0.0).cbrt()).clamp(0.0, 1.0)
}

/// The quietest fader setting, standing for minus infinity, which JSON can't hold.
const SILENT_DB: f32 = -144.0;
/// The tempo range a tempo lane covers.
const TEMPO_RANGE: (f32, f32) = (10.0, 999.0);
/// The pitch range a channel pitch lane covers, in cents either way.
const PITCH_CENTS: f32 = 2400.0;

impl ParamTarget {
    /// The real value the normalized automation value `n` (0 to 1) stands for:
    /// - insert volume in dB and channel volume as a linear gain, through [`volume_gain`] (channel volume tops out at
    ///   2, insert volume at +6 dB, and 0 is -144 dB, standing for silence);
    /// - pan and stereo separation from -1 to 1;
    /// - channel pitch from -2400 to 2400 cents;
    /// - a route's send level from 0 to 2;
    /// - an FX slot's on switch as 0 or 1 (on from 0.5);
    /// - tempo from 10 to 999 BPM;
    /// - everything else (slot mix, plugin parameters, MIDI CC, pitch bend, pressure) as the normalized value itself.
    pub fn value_of(&self, n: f32) -> f32 {
        let n = if n.is_finite() { n.clamp(0.0, 1.0) } else { 0.0 };
        let bipolar = 2.0 * n - 1.0;
        match self {
            Self::Insert(_, InsertParam::Volume) => {
                let gain = volume_gain(n);
                if gain <= 0.0 { SILENT_DB } else { (20.0 * gain.log10()).clamp(SILENT_DB, 6.0) }
            }
            Self::Insert(_, InsertParam::Pan | InsertParam::StereoSep) | Self::Channel(_, ChannelParam::Pan) => bipolar,
            Self::Channel(_, ChannelParam::Volume) => volume_gain(n).min(2.0),
            Self::Channel(_, ChannelParam::Pitch) => bipolar * PITCH_CENTS,
            Self::Route(_) => 2.0 * n,
            Self::Slot(_, _, SlotParam::Enabled) => {
                if n >= 0.5 {
                    1.0
                } else {
                    0.0
                }
            }
            Self::Tempo => TEMPO_RANGE.0 + n * (TEMPO_RANGE.1 - TEMPO_RANGE.0),
            Self::Slot(_, _, SlotParam::Mix)
            | Self::Plugin(..)
            | Self::Channel(_, ChannelParam::Cc(_) | ChannelParam::PitchBend | ChannelParam::Pressure) => n,
        }
    }

    /// The normalized value (0 to 1) of the real value `value`; the inverse of [`ParamTarget::value_of`].
    pub fn normalized_of(&self, value: f32) -> f32 {
        let value = if value.is_finite() { value } else { 0.0 };
        let n = match self {
            Self::Insert(_, InsertParam::Volume) => {
                if value <= SILENT_DB {
                    0.0
                } else {
                    volume_normalized(10f32.powf(value / 20.0))
                }
            }
            Self::Insert(_, InsertParam::Pan | InsertParam::StereoSep) | Self::Channel(_, ChannelParam::Pan) => (value + 1.0) / 2.0,
            Self::Channel(_, ChannelParam::Volume) => volume_normalized(value),
            Self::Channel(_, ChannelParam::Pitch) => (value / PITCH_CENTS + 1.0) / 2.0,
            Self::Route(_) => value / 2.0,
            Self::Slot(_, _, SlotParam::Enabled) => {
                if value >= 0.5 {
                    1.0
                } else {
                    0.0
                }
            }
            Self::Tempo => (value - TEMPO_RANGE.0) / (TEMPO_RANGE.1 - TEMPO_RANGE.0),
            Self::Slot(_, _, SlotParam::Mix)
            | Self::Plugin(..)
            | Self::Channel(_, ChannelParam::Cc(_) | ChannelParam::PitchBend | ChannelParam::Pressure) => value,
        };
        n.clamp(0.0, 1.0)
    }
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

impl AutomationLane {
    /// The normalized value (0 to 1) at `local` ticks from the lane's start: the default before the first point, the
    /// last point's value after the last.
    ///
    /// Complexity: O(log n) in points.
    pub fn normalized_at(&self, local: f64) -> f32 {
        let index = self
            .points
            .binary_search_by(|p| if (p.at.0 as f64) <= local { core::cmp::Ordering::Less } else { core::cmp::Ordering::Greater })
            .unwrap_or_else(|i| i);
        match index.checked_sub(1).and_then(|i| self.points.get(i)) {
            None => self.default,
            Some(a) => match self.points.get(index) {
                None => a.value,
                Some(b) => {
                    let span = (b.at.0 - a.at.0) as f64;
                    let u = if span > 0.0 { (local - a.at.0 as f64) / span } else { 1.0 };
                    a.curve.between(a.value, b.value, u)
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_map_both_ways() {
        let insert = ParamTarget::Insert(InsertId::from_u128(1), InsertParam::Volume);
        assert!((insert.value_of(0.8)).abs() < 1e-5, "0.8 is 0 dB");
        assert!((insert.value_of(1.0) - 5.81).abs() < 0.01, "1 is about +6 dB");
        assert_eq!(insert.value_of(0.0), -144.0);
        let channel = ChannelId::from_u128(2);
        let targets = [
            insert,
            ParamTarget::Insert(InsertId::from_u128(1), InsertParam::Pan),
            ParamTarget::Channel(channel, ChannelParam::Volume),
            ParamTarget::Channel(channel, ChannelParam::Pitch),
            ParamTarget::Channel(channel, ChannelParam::Cc(74)),
            ParamTarget::Tempo,
        ];
        for target in targets {
            for n in [0.1f32, 0.25, 0.5, 0.8, 0.95] {
                let back = target.normalized_of(target.value_of(n));
                assert!((back - n).abs() < 1e-4, "{target:?}: {n} -> {} -> {back}", target.value_of(n));
            }
        }
        assert_eq!(ParamTarget::Channel(channel, ChannelParam::Volume).value_of(0.8), 1.0);
        assert_eq!(ParamTarget::Channel(channel, ChannelParam::Pan).value_of(0.5), 0.0);
        assert_eq!(ParamTarget::Channel(channel, ChannelParam::Pitch).value_of(1.0), 2400.0);
    }

    #[test]
    fn curves_shape_the_segment() {
        assert_eq!(CurveShape::Linear.between(0.0, 1.0, 0.25), 0.25);
        assert_eq!(CurveShape::Hold.between(0.2, 1.0, 0.9), 0.2);
        assert_eq!(CurveShape::Pulse.between(0.0, 1.0, 0.49), 0.0);
        assert_eq!(CurveShape::Pulse.between(0.0, 1.0, 0.5), 1.0);
        assert_eq!(CurveShape::Stairs(4).between(0.0, 1.0, 0.6), 0.5);
        assert!((CurveShape::Sine.between(0.0, 1.0, 0.5) - 0.5).abs() < 1e-6);
        assert!(CurveShape::Exp(0.5).between(0.0, 1.0, 0.5) < 0.5, "positive tension rises late");
        assert!(CurveShape::Exp(-0.5).between(0.0, 1.0, 0.5) > 0.5);
        assert!((CurveShape::Exp(0.0).between(0.0, 1.0, 0.3) - 0.3).abs() < 1e-6);
        assert!((CurveShape::SCurve(-1.0).between(0.0, 1.0, 0.3) - 0.3).abs() < 1e-6, "the gentlest S curve is a line");
        assert!(CurveShape::SCurve(1.0).between(0.0, 1.0, 0.25) < 0.25);
        for curve in [CurveShape::Linear, CurveShape::Exp(0.7), CurveShape::SCurve(0.3), CurveShape::Sine] {
            assert_eq!(curve.between(0.2, 0.9, 0.0), 0.2);
            assert!((curve.between(0.2, 0.9, 1.0) - 0.9).abs() < 1e-6, "{curve:?}");
        }
    }

    #[test]
    fn lanes_hold_their_ends_and_follow_their_curves() {
        let point = |at, value, curve| AutoPoint { at: Tick(at), value, curve };
        let lane = AutomationLane {
            target: ParamTarget::Tempo,
            points: Vector::from(vec![point(100, 0.2, CurveShape::Linear), point(300, 0.6, CurveShape::Hold), point(400, 1.0, CurveShape::Linear)]),
            default: 0.5,
        };
        assert_eq!(lane.normalized_at(0.0), 0.5, "the default before the first point");
        assert_eq!(lane.normalized_at(100.0), 0.2);
        assert!((lane.normalized_at(200.0) - 0.4).abs() < 1e-6);
        assert_eq!(lane.normalized_at(350.0), 0.6, "hold");
        assert_eq!(lane.normalized_at(1000.0), 1.0, "the last value after the last point");
    }
}
