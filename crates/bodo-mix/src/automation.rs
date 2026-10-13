//! Automation playback (project.md section 10.5).
//!
//! The control thread flattens every automation clip on an unmuted track, and every lane of every pattern, into an
//! [`AutomationIndex`] that travels with the song index. Each lane's points are sorted, in ticks from where the lane
//! starts: a clip's start (less its offset), or the start of each pass of a pattern.
//!
//! Once per block the audio thread evaluates the lanes active at the block's first frame, O(log n) per lane, and
//! writes the real values into [`ParamValues`], a table the audio thread owns. Processors read their automated
//! parameters there and fall back to their live settings, so automation never races the control thread's writes,
//! and their per-block gain ramps smooth each step. A value holds after its lane ends; stopping, seeking or a loop
//! wrap clears the table, so the project's own values come back until a lane plays again.
//!
//! Each lane is read as far back as its parameter's audio is late through delay compensation ([`LaneDelays`]), so
//! automation lines up with what is heard (project.md section 9.4).
//!
//! Played now: channel volume, pan and pitch (pitch when a note starts); insert volume, pan and stereo separation;
//! FX slot mix and on switch; native effect parameters; and route send levels. MIDI CC, pitch bend, pressure,
//! plugins hosted from other formats and tempo are stored and edited but not yet played.

use bodo_kit_core::{SampleRate, Seconds, rt_safe};
use bodo_model::automation::{AutomationLane, ChannelParam, CurveShape, InsertParam, ParamTarget, SlotParam};
use bodo_model::channel::ChannelKind;
use bodo_model::pattern::Pattern;
use bodo_model::playlist::{ClipOffset, ClipSource};
use bodo_model::{PatternId, Project};

use crate::keys;
use crate::sequence::{PlayMode, Transport};

/// The most parameters automation drives at once. More are dropped and counted.
pub const MAX_PARAMS: usize = 1024;

/// A parameter of a processor that automation can drive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Param {
    /// A channel's volume (linear gain) or an insert's fader (dB).
    Volume,
    /// Pan, -1 to 1.
    Pan,
    /// A channel's pitch in cents.
    Pitch,
    /// An insert's stereo separation, -1 to 1.
    StereoSep,
}

/// The key of parameter `param` of the processor with key `node`.
pub fn param_key(node: u64, param: Param) -> u64 {
    let tag: u64 = match param {
        Param::Volume => 1,
        Param::Pan => 2,
        Param::Pitch => 3,
        Param::StereoSep => 4,
    };
    node.rotate_left(17) ^ tag.wrapping_mul(0xD6E8_FEB8_6659_FD93)
}

/// The parameter key a lane drives, if automation plays that target yet.
fn playable_key(target: &ParamTarget) -> Option<u64> {
    match target {
        ParamTarget::Channel(id, ChannelParam::Volume) => Some(param_key(keys::channel(*id), Param::Volume)),
        ParamTarget::Channel(id, ChannelParam::Pan) => Some(param_key(keys::channel(*id), Param::Pan)),
        ParamTarget::Channel(id, ChannelParam::Pitch) => Some(param_key(keys::channel(*id), Param::Pitch)),
        ParamTarget::Insert(id, InsertParam::Volume) => Some(param_key(keys::insert(*id), Param::Volume)),
        ParamTarget::Insert(id, InsertParam::Pan) => Some(param_key(keys::insert(*id), Param::Pan)),
        ParamTarget::Insert(id, InsertParam::StereoSep) => Some(param_key(keys::insert(*id), Param::StereoSep)),
        ParamTarget::Slot(id, index, SlotParam::Mix) => Some(keys::slot_param(*id, *index, 0)),
        ParamTarget::Slot(id, index, SlotParam::Enabled) => Some(keys::slot_param(*id, *index, 1)),
        ParamTarget::Plugin(instance, param) => Some(keys::plugin_param(*instance, *param)),
        ParamTarget::Route(key) => Some(keys::route_level(*key)),
        _ => None,
    }
}

/// How late each automated parameter's audio is, so its lane is read that much earlier (project.md section 9.4:
/// automation for node n is read at `t - in(n)`). Sorted by parameter key; parameters left out have no delay.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LaneDelays {
    entries: Vec<(u64, u32)>,
    /// The distinct delays, smallest first, zero included.
    distinct: Vec<u32>,
}

impl LaneDelays {
    /// Delays from `(parameter key, frames)` pairs.
    ///
    /// Allocates. Complexity: O(n log n).
    pub fn new(mut entries: Vec<(u64, u32)>) -> Self {
        entries.retain(|&(_, d)| d > 0);
        entries.sort_unstable();
        entries.dedup_by_key(|e| e.0);
        let mut distinct: Vec<u32> = std::iter::once(0).chain(entries.iter().map(|&(_, d)| d)).collect();
        distinct.sort_unstable();
        distinct.dedup();
        Self { entries, distinct }
    }

    /// The delay of the parameter with `key`, in frames.
    ///
    /// RT-safe. Complexity: O(log n).
    #[rt_safe]
    pub fn of(&self, key: u64) -> u32 {
        self.entries.binary_search_by_key(&key, |&(k, _)| k).ok().and_then(|i| self.entries.get(i)).map_or(0, |&(_, d)| d)
    }
}

/// The values automation set, by parameter key. The audio thread owns it; processors read it in place of their live
/// settings.
#[derive(Debug)]
pub struct ParamValues {
    /// Sorted by key; capacity [`MAX_PARAMS`].
    entries: Vec<(u64, f32)>,
    dropped: u64,
}

impl Default for ParamValues {
    fn default() -> Self {
        Self::new()
    }
}

impl ParamValues {
    /// An empty table with room for [`MAX_PARAMS`] values.
    ///
    /// Allocates.
    pub fn new() -> Self {
        Self { entries: Vec::with_capacity(MAX_PARAMS), dropped: 0 }
    }

    /// The value automation set for `key`, if any.
    ///
    /// RT-safe. Complexity: O(log n) in values set.
    #[rt_safe]
    pub fn get(&self, key: u64) -> Option<f32> {
        self.entries.binary_search_by_key(&key, |&(k, _)| k).ok().and_then(|i| self.entries.get(i)).map(|&(_, v)| v)
    }

    /// Sets the value of `key`. When the table is full a new key is dropped and counted.
    ///
    /// RT-safe. Complexity: O(log n) to update, O(n) the first time a key is set.
    #[rt_safe]
    pub fn set(&mut self, key: u64, value: f32) {
        match self.entries.binary_search_by_key(&key, |&(k, _)| k) {
            Ok(i) => {
                if let Some(entry) = self.entries.get_mut(i) {
                    entry.1 = value;
                }
            }
            Err(i) if self.entries.len() < self.entries.capacity() => self.entries.insert(i, (key, value)),
            Err(_) => self.dropped += 1,
        }
    }

    /// Forgets every value, so processors use their live settings again.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// How many values are set.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no value is set.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Values dropped because the table was full, since the engine started.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }
}

/// A point of a flattened lane, in ticks from the lane's start.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Point {
    at: i64,
    value: f32,
    curve: CurveShape,
}

/// A lane that drives a played parameter.
#[derive(Clone, Debug, PartialEq)]
struct Lane {
    key: u64,
    target: ParamTarget,
    default: f32,
    points: Vec<Point>,
}

impl Lane {
    fn of(lane: &AutomationLane) -> Option<Self> {
        let key = playable_key(&lane.target)?;
        let mut points: Vec<Point> = lane.points.iter().map(|p| Point { at: p.at.0, value: p.value, curve: p.curve }).collect();
        points.sort_by_key(|p| p.at);
        Some(Self { key, target: lane.target, default: lane.default, points })
    }

    /// The real value at `local` ticks from the lane's start.
    ///
    /// RT-safe. Complexity: O(log n) in points.
    fn value_at(&self, local: f64) -> f32 {
        let index = self.points.partition_point(|p| (p.at as f64) <= local);
        let normalized = match index.checked_sub(1).and_then(|i| self.points.get(i)) {
            None => self.default,
            Some(a) => match self.points.get(index) {
                None => a.value,
                Some(b) => {
                    let span = (b.at - a.at) as f64;
                    let u = if span > 0.0 { (local - a.at as f64) / span } else { 1.0 };
                    a.curve.between(a.value, b.value, u)
                }
            },
        };
        self.target.value_of(normalized)
    }
}

/// Where lanes play on the playlist: a clip of an automation channel, or a clip of a pattern with lanes.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Placement {
    start: i64,
    end: i64,
    /// The tick where the lanes' time 0 falls.
    origin: i64,
    /// For pattern lanes, the pattern's length: the lanes repeat with it. Zero for automation clips.
    repeat: i64,
    first: u32,
    count: u32,
}

/// The lanes of a pattern, for Pattern mode.
#[derive(Clone, Copy, Debug, PartialEq)]
struct PatternLanes {
    key: u64,
    length: i64,
    first: u32,
    count: u32,
}

/// Every lane that plays, flattened for the audio thread.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AutomationIndex {
    lanes: Vec<Lane>,
    /// Sorted by start.
    placements: Vec<Placement>,
    /// The longest placement, which bounds how far back a lookup must search.
    max_len: i64,
    /// Sorted by key.
    patterns: Vec<PatternLanes>,
}

impl AutomationIndex {
    /// Flattens `project`'s automation clips and pattern lanes. `length` is how long each pattern loops, the same
    /// length the sequencer repeats its notes by.
    ///
    /// Allocates. Complexity: O(L + P log P) in lane points and placements.
    pub fn build(project: &Project, length: impl Fn(&Pattern) -> i64) -> Self {
        let mut index = Self::default();
        let mut pattern_lanes: Vec<(PatternId, PatternLanes)> = Vec::new();
        for (id, pattern) in &project.patterns {
            let first = index.lanes.len() as u32;
            index.lanes.extend(pattern.automation.iter().filter_map(|lane| Lane::of(lane)));
            let count = index.lanes.len() as u32 - first;
            if count > 0 {
                pattern_lanes.push((*id, PatternLanes { key: keys::pattern(*id), length: length(pattern).max(1), first, count }));
            }
        }
        index.patterns = pattern_lanes.iter().map(|(_, lanes)| *lanes).collect();
        index.patterns.sort_by_key(|p| p.key);
        let mut channel_lanes: std::collections::HashMap<bodo_model::ChannelId, (u32, u32)> = std::collections::HashMap::new();
        for (id, channel) in &project.channels {
            if let ChannelKind::Automation(lane) = &channel.kind
                && let Some(lane) = Lane::of(lane)
            {
                channel_lanes.insert(*id, (index.lanes.len() as u32, 1));
                index.lanes.push(lane);
            }
        }
        let solo = bodo_model::playlist::any_solo(project.tracks.values());
        for track in project.track_order.iter().filter_map(|id| project.tracks.get(id)).filter(|t| t.plays(solo)) {
            for clip in track.clips.iter().filter(|c| !c.muted) {
                let offset = match clip.offset {
                    ClipOffset::Ticks(t) => t.0,
                    ClipOffset::Frames(_) => 0,
                };
                let (repeat, lanes) = match clip.source {
                    ClipSource::Automation(channel) => (0, channel_lanes.get(&channel).copied()),
                    ClipSource::Pattern(pattern) => match pattern_lanes.iter().find(|(id, _)| *id == pattern) {
                        Some((_, lanes)) => (lanes.length, Some((lanes.first, lanes.count))),
                        None => (0, None),
                    },
                    ClipSource::Audio(_) => (0, None),
                };
                if let Some((first, count)) = lanes {
                    index.placements.push(Placement { start: clip.start.0, end: clip.end().0, origin: clip.start.0 - offset, repeat, first, count });
                }
            }
        }
        index.placements.sort_by_key(|p| p.start);
        index.max_len = index.placements.iter().map(|p| p.end - p.start).max().unwrap_or(0);
        index
    }

    /// Whether nothing is automated.
    pub fn is_empty(&self) -> bool {
        self.placements.is_empty() && self.patterns.is_empty()
    }

    fn set_lanes(&self, first: u32, count: u32, local: f64, values: &mut ParamValues, wanted: &impl Fn(u64) -> bool) {
        let range = first as usize..(first + count) as usize;
        for lane in self.lanes.get(range).unwrap_or(&[]) {
            if wanted(lane.key) {
                values.set(lane.key, lane.value_at(local));
            }
        }
    }

    /// Sets the values of the lanes active at `tick` in `mode` into `values`.
    ///
    /// RT-safe. Complexity: O(log P + k log n) in placements, active lanes and their points.
    #[rt_safe]
    pub fn evaluate(&self, mode: PlayMode, tick: f64, values: &mut ParamValues) {
        self.evaluate_where(mode, tick, values, &|_| true);
    }

    /// Like [`AutomationIndex::evaluate`], for the lanes whose keys `wanted` accepts.
    ///
    /// RT-safe. Complexity: O(log P + k log n) in placements, active lanes and their points.
    #[rt_safe]
    pub fn evaluate_where(&self, mode: PlayMode, tick: f64, values: &mut ParamValues, wanted: &impl Fn(u64) -> bool) {
        match mode {
            PlayMode::Song => {
                let to = self.placements.partition_point(|p| (p.start as f64) <= tick);
                let from = self.placements[..to].partition_point(|p| (p.start as f64) <= tick - self.max_len as f64);
                // In start order, so where two placements drive one parameter the later one wins.
                for placement in self.placements.get(from..to).unwrap_or(&[]) {
                    if (placement.end as f64) <= tick {
                        continue;
                    }
                    let mut local = tick - placement.origin as f64;
                    if placement.repeat > 0 {
                        local = local.rem_euclid(placement.repeat as f64);
                    }
                    self.set_lanes(placement.first, placement.count, local, values, wanted);
                }
            }
            PlayMode::Pattern(id) => {
                let key = keys::pattern(id);
                if let Ok(i) = self.patterns.binary_search_by_key(&key, |p| p.key)
                    && let Some(lanes) = self.patterns.get(i)
                {
                    self.set_lanes(lanes.first, lanes.count, tick.rem_euclid(lanes.length as f64), values, wanted);
                }
            }
        }
    }
}

/// Sets the automated values for the block that starts at the transport's frame. Each lane is read as far back as
/// its parameter's audio is late, by `delays`, so automation lines up with delay-compensated audio.
///
/// RT-safe. Complexity: O(d log m) in distinct delays and tempo points, plus [`AutomationIndex::evaluate`] per delay.
#[rt_safe]
pub fn apply(
    index: &AutomationIndex,
    tempo: &bodo_kit_core::TempoMap,
    transport: &Transport,
    rate: SampleRate,
    values: &mut ParamValues,
    delays: &LaneDelays,
) {
    if index.is_empty() {
        return;
    }
    let tick_at = |delay: u32| tempo.ticks_at(Seconds((transport.frame - i64::from(delay)).max(0) as f64 / f64::from(rate.hz())));
    if delays.entries.is_empty() {
        index.evaluate(transport.mode, tick_at(0), values);
        return;
    }
    for &delay in &delays.distinct {
        index.evaluate_where(transport.mode, tick_at(delay), values, &|key| delays.of(key) == delay);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_set_get_and_clear() {
        let mut values = ParamValues::new();
        values.set(5, 0.5);
        values.set(2, 0.2);
        values.set(5, 0.7);
        assert_eq!((values.get(5), values.get(2), values.get(3)), (Some(0.7), Some(0.2), None));
        assert_eq!(values.len(), 2);
        values.clear();
        assert!(values.is_empty());
    }
}
