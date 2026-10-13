//! The mixer: inserts, the master, FX slots and routes (project.md section 9, ADR 0009).

use std::sync::Arc;

use bodo_kit_core::Frames;
use imbl::{OrdMap, Vector};

use crate::channel::PluginRef;
use crate::color::Color;
use crate::id::{ChannelId, InsertId};

/// Every insert has exactly this many FX slots; the type enforces it.
pub const FX_SLOTS: usize = 10;

/// The mixer.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mixer {
    /// The master, always the last node of the graph.
    pub master: Arc<MixerInsert>,
    /// Every other insert.
    pub inserts: OrdMap<InsertId, Arc<MixerInsert>>,
    /// Display order: insert 1, 2, 3 and so on.
    pub order: Vector<InsertId>,
    /// Routes between channels and inserts.
    #[serde(with = "route_list")]
    pub routes: OrdMap<RouteKey, Route>,
}

impl Mixer {
    /// A mixer with a master and `inserts` empty inserts named "Insert 1" and up.
    pub fn with_inserts(master: InsertId, inserts: impl IntoIterator<Item = InsertId>) -> Self {
        let mut map = OrdMap::new();
        let mut order = Vector::new();
        for (index, id) in inserts.into_iter().enumerate() {
            map.insert(id, Arc::new(MixerInsert::new(id, format!("Insert {}", index + 1))));
            order.push_back(id);
        }
        Self { master: Arc::new(MixerInsert::new(master, "Master")), inserts: map, order, routes: OrdMap::new() }
    }

    /// The insert with `id`, including the master.
    pub fn insert(&self, id: InsertId) -> Option<&Arc<MixerInsert>> {
        if self.master.id == id { Some(&self.master) } else { self.inserts.get(&id) }
    }
}

/// Where a route starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", tag = "type", content = "id")]
pub enum RouteSource {
    /// A channel of the channel rack.
    Channel(ChannelId),
    /// A mixer insert.
    Insert(InsertId),
}

/// Identifies a route by its two ends; there is at most one route between them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteKey {
    /// The source.
    pub from: RouteSource,
    /// The insert it goes to. Routes never target the master; inserts reach it through `to_master`.
    pub to: InsertId,
}

/// A route's settings.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Route {
    /// The send level as a linear gain.
    pub send_level: f32,
    /// Where on the source strip the signal is taken.
    #[serde(default)]
    pub tap: SendTap,
    /// Audio, or sidechain only.
    #[serde(default)]
    pub kind: RouteKind,
}

/// Where a send takes its signal (ADR 0009).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SendTap {
    /// After polarity and swap, before the FX.
    PreFx,
    /// After the FX, EQ and stereo separation, before pan and the fader.
    PreFader,
    /// After the fader and mute. The default.
    #[default]
    PostFader,
}

/// Whether a route carries audio or only feeds sidechains.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RouteKind {
    /// Summed into the target's input.
    #[default]
    Audio,
    /// Feeds only plugin sidechain inputs of the target.
    Sidechain,
}

/// A mixer insert, or the master.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MixerInsert {
    /// The insert's id.
    pub id: InsertId,
    /// Its name.
    pub name: String,
    /// Its color.
    #[serde(default = "gray")]
    pub color: Color,
    /// The name of its icon in the mixer, such as `drum` or `microphone`; none by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// The ten FX slots, in processing order.
    #[serde(default)]
    pub slots: [FxSlot; FX_SLOTS],
    /// The built-in 3-band EQ, after slot 10.
    #[serde(default)]
    pub eq: StripEq,
    /// The fader in dB, -inf to +6.
    #[serde(default)]
    pub volume_db: f32,
    /// Pan, -1 to 1.
    #[serde(default)]
    pub pan: f32,
    /// Stereo separation: -1 wider, 0 unchanged, +1 mono.
    #[serde(default)]
    pub stereo_sep: f32,
    /// Inverts polarity, before the FX.
    #[serde(default)]
    pub polarity_invert: bool,
    /// Swaps left and right, before the FX.
    #[serde(default)]
    pub swap_lr: bool,
    /// Manual track latency: positive declares latency, negative delays this strip.
    #[serde(default)]
    pub latency_offset: Frames,
    /// When delay compensation is bypassed.
    #[serde(default)]
    pub pdc_bypass: PdcBypass,
    /// Whether the insert is muted.
    #[serde(default)]
    pub muted: bool,
    /// Whether the insert is soloed.
    #[serde(default)]
    pub solo: bool,
    /// Whether it feeds the master. Ignored on the master itself.
    #[serde(default = "yes")]
    pub to_master: bool,
    /// A hardware input for recording and monitoring.
    #[serde(default)]
    pub input: Option<HwPort>,
    /// A direct hardware output, such as a cue mix.
    #[serde(default)]
    pub output: Option<HwPort>,
    /// Whether disk recording is armed.
    #[serde(default)]
    pub armed: bool,
    /// What disk recording captures.
    #[serde(default)]
    pub record_tap: RecordTap,
    /// The format recordings are written in.
    #[serde(default)]
    pub record_format: RecordFormat,
}

fn gray() -> Color {
    Color::GRAY
}

fn yes() -> bool {
    true
}

impl MixerInsert {
    /// An empty insert at unity gain that feeds the master.
    pub fn new(id: InsertId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            color: Color::GRAY,
            icon: None,
            slots: Default::default(),
            eq: StripEq::default(),
            volume_db: 0.0,
            pan: 0.0,
            stereo_sep: 0.0,
            polarity_invert: false,
            swap_lr: false,
            latency_offset: Frames::ZERO,
            pdc_bypass: PdcBypass::Never,
            muted: false,
            solo: false,
            to_master: true,
            input: None,
            output: None,
            armed: false,
            record_tap: RecordTap::PostFader,
            record_format: RecordFormat::Float32,
        }
    }
}

/// An FX slot.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FxSlot {
    /// The plugin, if any.
    pub plugin: Option<PluginRef>,
    /// Whether the slot processes. A disabled slot keeps reporting its latency.
    pub enabled: bool,
    /// Dry/wet mix, 0 to 1.
    pub mix: f32,
    /// The route feeding this slot's sidechain input. It must be a sidechain route into this insert.
    pub sidechain: Option<RouteSource>,
    /// The values of a native effect's parameters, by their stable ids; a parameter that isn't listed is at its
    /// default. Plugins of other formats keep their settings in their own state instead.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<ParamValue>,
}

impl Default for FxSlot {
    fn default() -> Self {
        Self { plugin: None, enabled: true, mix: 1.0, sidechain: None, params: Vec::new() }
    }
}

impl FxSlot {
    /// The value of parameter `id`, if it was set.
    ///
    /// Complexity: O(log n) in the parameters set.
    pub fn param(&self, id: u32) -> Option<f32> {
        self.params.binary_search_by_key(&id, |p| p.id).ok().and_then(|i| self.params.get(i)).map(|p| p.value)
    }

    /// Sets parameter `id` to `value`, keeping the list sorted by id.
    ///
    /// Complexity: O(n) in the parameters set.
    pub fn set_param(&mut self, id: u32, value: f32) {
        match self.params.binary_search_by_key(&id, |p| p.id) {
            Ok(i) => {
                if let Some(p) = self.params.get_mut(i) {
                    p.value = value;
                }
            }
            Err(i) => self.params.insert(i, ParamValue { id, value }),
        }
    }
}

/// The value of one effect parameter.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParamValue {
    /// The parameter's stable id.
    pub id: u32,
    /// Its value, in the parameter's own unit.
    pub value: f32,
}

/// One band of the strip EQ.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EqBand {
    /// Frequency in Hz.
    pub frequency: f32,
    /// Gain in dB.
    pub gain_db: f32,
    /// Quality or shelf slope.
    pub q: f32,
}

/// The built-in 3-band EQ: low shelf, peaking mid, high shelf.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StripEq {
    /// The low shelf.
    pub low: EqBand,
    /// The peaking mid band.
    pub mid: EqBand,
    /// The high shelf.
    pub high: EqBand,
}

impl Default for StripEq {
    fn default() -> Self {
        let band = |frequency| EqBand { frequency, gain_db: 0.0, q: core::f32::consts::FRAC_1_SQRT_2 };
        Self { low: band(120.0), mid: band(1200.0), high: band(8000.0) }
    }
}

/// When delay compensation is bypassed, for low-latency monitoring.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PdcBypass {
    /// Never.
    #[default]
    Never,
    /// Always.
    Always,
    /// While recording.
    WhileRecording,
}

/// What an armed insert records.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecordTap {
    /// The summed input.
    Input,
    /// Before the FX.
    PreFx,
    /// After the FX and EQ.
    PostFx,
    /// After the fader.
    #[default]
    PostFader,
}

/// The file format of recordings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecordFormat {
    /// 32-bit float WAV.
    #[default]
    Float32,
    /// 24-bit WAV.
    Int24,
    /// 24-bit FLAC, lossless and about half the size.
    Flac,
    /// 32-bit float Wave64, for takes longer than WAV's 4 GB.
    W64,
}

/// Hardware channels of the audio device, by index.
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HwPort {
    /// The device channels: one for mono, two for stereo.
    pub channels: Vec<u16>,
}

/// Serializes the route map as a list, since JSON object keys must be strings.
mod route_list {
    use imbl::OrdMap;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::{Route, RouteKey, RouteKind, RouteSource, SendTap};
    use crate::id::InsertId;

    #[derive(Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Entry {
        from: RouteSource,
        to: InsertId,
        send_level: f32,
        #[serde(default)]
        tap: SendTap,
        #[serde(default)]
        kind: RouteKind,
    }

    pub(super) fn serialize<S: Serializer>(routes: &OrdMap<RouteKey, Route>, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(routes.iter().map(|(key, route)| Entry {
            from: key.from,
            to: key.to,
            send_level: route.send_level,
            tap: route.tap,
            kind: route.kind,
        }))
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<OrdMap<RouteKey, Route>, D::Error> {
        let entries = Vec::<Entry>::deserialize(deserializer)?;
        Ok(entries.into_iter().map(|e| (RouteKey { from: e.from, to: e.to }, Route { send_level: e.send_level, tap: e.tap, kind: e.kind })).collect())
    }
}
