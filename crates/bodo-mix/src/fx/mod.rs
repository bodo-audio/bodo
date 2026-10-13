//! Native effects for the mixer's FX slots (project.md sections 9.1 to 9.3).
//!
//! An [`Effect`] processes a stereo block in place on the audio thread, with an optional sidechain input. Each kind of
//! effect is described once in the [`catalog`]: its stable uid, name and category, its parameters with stable integer
//! ids, units, ranges, defaults and tags ([`ParamSpec`]), and how much latency it reports for a set of parameter
//! values. A slot of the model holds a `PluginRef` with `PluginFormat::Native` and the uid, and the slot's parameter
//! values by id.
//!
//! Parameters reach a running effect as plain values in their own units ([`EffectBlock::params`]), in the order of
//! the effect's [`EffectInfo::params`]. Automation drives them through normalized values (0 to 1), which
//! [`ParamSpec::from_normalized`] maps to the parameter's range.

use bodo_kit_core::{SampleRate, rt_safe};

mod delay;
mod ducker;
mod filter;
mod latency;
mod utility;

/// A native effect instance.
///
/// Instances are created on the control thread ([`EffectInfo::create`]), which may allocate, and run on the audio
/// thread, which must not.
pub trait Effect: Send + Sync + 'static {
    /// Processes one block in place.
    ///
    /// Runs on the audio thread or an RT worker, so it must be RT-safe.
    fn process(&mut self, block: &mut EffectBlock<'_>);

    /// Forgets every sound in flight, as after a jump of the transport.
    ///
    /// Must be RT-safe.
    fn reset(&mut self);
}

/// What an effect sees when it runs.
#[derive(Debug)]
pub struct EffectBlock<'a> {
    /// The left channel, processed in place.
    pub left: &'a mut [f32],
    /// The right channel, processed in place.
    pub right: &'a mut [f32],
    /// The sidechain input, left and right, when the slot has one.
    pub sidechain: Option<(&'a [f32], &'a [f32])>,
    /// The parameter values for this block, in the order of [`EffectInfo::params`], in their own units.
    pub params: &'a [f32],
    /// The sample rate.
    pub rate: SampleRate,
    /// The tempo at the start of the block, in BPM.
    pub tempo: f64,
}

impl EffectBlock<'_> {
    /// Parameter `index` of [`EffectInfo::params`], or `default` when the block has fewer values.
    pub fn param(&self, index: usize, default: f32) -> f32 {
        self.params.get(index).copied().unwrap_or(default)
    }
}

/// What a parameter's value measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Unit {
    /// A plain number.
    None,
    /// Decibels.
    Decibels,
    /// Hertz.
    Hertz,
    /// Milliseconds.
    Milliseconds,
    /// Percent.
    Percent,
    /// A pan position, -1 (left) to 1 (right).
    Pan,
    /// Samples.
    Samples,
}

/// How a parameter's values are spread over a knob or an automation lane.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum ParamKind {
    /// Any value in the range, spread evenly.
    Linear,
    /// Any value in the range, spread geometrically, as frequencies and times are heard. The range must be positive.
    Logarithmic,
    /// Whole numbers in the range.
    Integer,
    /// Off (0) or on (1).
    Toggle,
    /// One of a list of choices, by index from 0.
    Choice {
        /// The choices' names.
        choices: &'static [&'static str],
    },
}

/// A parameter of a native effect.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParamSpec {
    /// The stable id. It never changes and is never reused for another parameter of the same effect.
    pub id: u32,
    /// The name.
    pub name: &'static str,
    /// The unit.
    pub unit: Unit,
    /// The smallest value.
    pub min: f32,
    /// The largest value.
    pub max: f32,
    /// The value a new instance starts with.
    pub default: f32,
    /// How the values spread.
    pub kind: ParamKind,
    /// Words that tell an assistant what the parameter does, such as `brightness` or `space`.
    pub tags: &'static [&'static str],
}

impl ParamSpec {
    /// `value` limited to the parameter's range, and rounded where the parameter takes whole numbers.
    pub fn clamp(&self, value: f32) -> f32 {
        let value = if value.is_finite() { value.clamp(self.min, self.max) } else { self.default };
        match self.kind {
            ParamKind::Integer | ParamKind::Toggle | ParamKind::Choice { .. } => value.round(),
            ParamKind::Linear | ParamKind::Logarithmic => value,
        }
    }

    /// The value at `n` along the parameter's travel, 0 to 1.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn from_normalized(&self, n: f32) -> f32 {
        let n = n.clamp(0.0, 1.0);
        let value = match self.kind {
            ParamKind::Logarithmic if self.min > 0.0 => self.min * (self.max / self.min).powf(n),
            _ => self.min + (self.max - self.min) * n,
        };
        self.clamp(value)
    }

    /// Where `value` sits along the parameter's travel, 0 to 1; the inverse of [`ParamSpec::from_normalized`].
    pub fn to_normalized(&self, value: f32) -> f32 {
        let value = self.clamp(value);
        let n = match self.kind {
            ParamKind::Logarithmic if self.min > 0.0 => (value / self.min).ln() / (self.max / self.min).ln(),
            _ if self.max > self.min => (value - self.min) / (self.max - self.min),
            _ => 0.0,
        };
        n.clamp(0.0, 1.0)
    }

    /// The value written for people, such as `-6.0 dB` or `1/8 dotted`.
    pub fn display(&self, value: f32) -> String {
        let value = self.clamp(value);
        match (self.kind, self.unit) {
            (ParamKind::Toggle, _) => String::from(if value >= 0.5 { "on" } else { "off" }),
            (ParamKind::Choice { choices }, _) => choices.get(value as usize).map_or_else(|| value.to_string(), |c| (*c).to_owned()),
            (_, Unit::Decibels) => format!("{value:.1} dB"),
            (_, Unit::Hertz) if value >= 1000.0 => format!("{:.2} kHz", value / 1000.0),
            (_, Unit::Hertz) => format!("{value:.0} Hz"),
            (_, Unit::Milliseconds) => format!("{value:.1} ms"),
            (_, Unit::Percent) => format!("{value:.0}%"),
            (_, Unit::Pan) if value.abs() < 0.005 => String::from("center"),
            (_, Unit::Pan) => format!("{:.0}% {}", value.abs() * 100.0, if value < 0.0 { "left" } else { "right" }),
            (_, Unit::Samples) => format!("{value:.0} samples"),
            (_, Unit::None) => format!("{value:.2}"),
        }
    }
}

/// What kind of effect it is, for browsing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Category {
    /// Gain, pan, width and other utilities.
    Utility,
    /// Filters and EQs.
    Filter,
    /// Delays and echoes.
    Delay,
    /// Compressors, duckers and other dynamics.
    Dynamics,
}

/// A kind of native effect.
#[derive(Clone, Copy, Debug)]
pub struct EffectInfo {
    /// The stable uid stored in projects, such as `bodo.delay`.
    pub uid: &'static str,
    /// The name.
    pub name: &'static str,
    /// What it does, in a sentence.
    pub description: &'static str,
    /// Its category.
    pub category: Category,
    /// Whether it listens to a sidechain input.
    pub sidechain: bool,
    /// Whether browsers should leave it out, as for test effects.
    pub hidden: bool,
    /// Its parameters.
    pub params: &'static [ParamSpec],
    /// Makes an instance for `rate`, with room for blocks of `max_frames`. Runs on the control thread.
    pub create: fn(SampleRate, usize) -> Box<dyn Effect>,
    /// The latency an instance reports with parameter values `params`, in frames at `rate`.
    pub latency: fn(&[f32], SampleRate) -> u32,
    /// The most latency it can report at `rate`, which sizes the slot's dry delay.
    pub max_latency: fn(SampleRate) -> u32,
}

impl EffectInfo {
    /// The index of parameter `id` in [`EffectInfo::params`].
    pub fn index_of(&self, id: u32) -> Option<usize> {
        self.params.iter().position(|p| p.id == id)
    }

    /// The parameter called `name`, ignoring case.
    pub fn param_named(&self, name: &str) -> Option<&ParamSpec> {
        self.params.iter().find(|p| p.name.eq_ignore_ascii_case(name.trim()))
    }

    /// The default value of every parameter, in order.
    pub fn defaults(&self) -> Vec<f32> {
        self.params.iter().map(|p| p.default).collect()
    }
}

impl serde::Serialize for EffectInfo {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("EffectInfo", 6)?;
        s.serialize_field("uid", self.uid)?;
        s.serialize_field("name", self.name)?;
        s.serialize_field("description", self.description)?;
        s.serialize_field("category", &self.category)?;
        s.serialize_field("sidechain", &self.sidechain)?;
        s.serialize_field("params", self.params)?;
        s.end()
    }
}

/// No latency, whatever the settings.
fn no_latency(_: &[f32], _: SampleRate) -> u32 {
    0
}

/// No latency, whatever the rate.
fn no_max_latency(_: SampleRate) -> u32 {
    0
}

/// Every native effect, in browsing order.
pub fn catalog() -> &'static [EffectInfo] {
    &CATALOG
}

static CATALOG: [EffectInfo; 5] = [utility::INFO, filter::INFO, delay::INFO, ducker::INFO, latency::INFO];

/// The native effect with `uid`.
///
/// Complexity: O(k) in the catalog's effects.
pub fn find(uid: &str) -> Option<&'static EffectInfo> {
    CATALOG.iter().find(|info| info.uid == uid)
}

/// Moves `current` toward `target` sample by sample over a block, writing each step through `f`. Effects use it to
/// glide parameters that would click if they jumped.
///
/// RT-safe.
#[rt_safe]
pub(crate) fn glide(current: f32, target: f32, frames: usize, mut f: impl FnMut(usize, f32)) {
    if current == target || frames == 0 {
        for i in 0..frames {
            f(i, target);
        }
        return;
    }
    let step = (target - current) / frames as f32;
    for i in 0..frames {
        f(i, current + step * (i + 1) as f32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_effect_has_unique_ids_and_sane_defaults() {
        let mut uids = std::collections::HashSet::new();
        for info in catalog() {
            assert!(uids.insert(info.uid), "{} twice", info.uid);
            let mut ids = std::collections::HashSet::new();
            for param in info.params {
                assert!(ids.insert(param.id), "{}: id {} twice", info.uid, param.id);
                assert!(param.min <= param.default && param.default <= param.max, "{}: {} default out of range", info.uid, param.name);
                assert!((param.from_normalized(param.to_normalized(param.default)) - param.default).abs() < 1e-3 * param.max.abs().max(1.0));
            }
        }
        assert_eq!(find("bodo.delay").map(|i| i.name), Some("Delay"));
        assert!(find("nope").is_none());
    }

    #[test]
    fn values_map_to_and_from_the_travel() {
        let cutoff = ParamSpec {
            id: 0,
            name: "Cutoff",
            unit: Unit::Hertz,
            min: 20.0,
            max: 20_000.0,
            default: 1000.0,
            kind: ParamKind::Logarithmic,
            tags: &[],
        };
        assert!((cutoff.from_normalized(0.5) - 632.46).abs() < 0.1, "the geometric middle");
        assert_eq!(cutoff.display(1500.0), "1.50 kHz");
        let toggle = ParamSpec { kind: ParamKind::Toggle, min: 0.0, max: 1.0, default: 0.0, ..cutoff };
        assert_eq!(toggle.from_normalized(0.7), 1.0);
        assert_eq!(toggle.display(1.0), "on");
    }
}
