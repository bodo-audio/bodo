//! The project invariants (project.md section 8.4), checked after every transaction in debug builds and in CI.

use std::collections::{HashMap, HashSet};

use bodo_kit_core::Tick;
use bodo_kit_graph::{DagError, DagNode, IncrementalDag};

use crate::channel::ChannelKind;
use crate::id::{ChannelId, InsertId};
use crate::mixer::{FX_SLOTS, RouteKind, RouteSource};
use crate::pattern::PatternLen;
use crate::playlist::ClipSource;
use crate::project::Project;

/// One broken invariant. The messages name entities by id, so agents can act on them.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Violation {
    /// An order list doesn't name every entity of its map exactly once.
    #[error("{list} doesn't list every entry exactly once")]
    Order {
        /// The order list.
        list: &'static str,
    },
    /// Something refers to an entity that doesn't exist.
    #[error("{owner} refers to {target}, which doesn't exist")]
    Dangling {
        /// What holds the reference.
        owner: String,
        /// The missing entity.
        target: String,
    },
    /// A sequence that must be sorted isn't.
    #[error("{owner} is not sorted")]
    Unsorted {
        /// The sequence.
        owner: String,
    },
    /// A length that must be positive isn't.
    #[error("{owner} has a length that isn't positive")]
    NonPositiveLength {
        /// What has the length.
        owner: String,
    },
    /// A clip-mode pattern has notes for another channel.
    #[error("{pattern} is bound to {channel} but has notes for {other}")]
    BindingBroken {
        /// The pattern.
        pattern: String,
        /// Its bound channel.
        channel: String,
        /// The other channel.
        other: String,
    },
    /// Two notes of a pattern share an id, or an id isn't below the pattern's next id.
    #[error("{pattern} has a duplicate or out-of-range note id {note}")]
    BadNoteId {
        /// The pattern.
        pattern: String,
        /// The note id.
        note: u32,
    },
    /// A MIDI key above 127.
    #[error("{owner} has key {key}, above 127")]
    KeyOutOfRange {
        /// What has the key.
        owner: String,
        /// The key.
        key: u8,
    },
    /// Two clips share an id.
    #[error("clip {clip} appears more than once")]
    DuplicateClip {
        /// The clip.
        clip: String,
    },
    /// A clip plays a channel of the wrong kind.
    #[error("clip {clip} plays {source_channel}, which isn't the right kind of channel")]
    WrongClipSource {
        /// The clip.
        clip: String,
        /// The channel it plays.
        source_channel: String,
    },
    /// A route goes from an insert to itself, or targets the master.
    #[error("the route from {from} to {to} is not allowed")]
    BadRoute {
        /// The route's source.
        from: String,
        /// The route's target.
        to: String,
    },
    /// The routing between inserts has a cycle.
    #[error("the routing has a cycle through {}", .path.join(" -> "))]
    Cycle {
        /// The inserts on the cycle, in order.
        path: Vec<String>,
    },
    /// An FX slot takes its sidechain from a source that has no sidechain route into the insert.
    #[error("slot {slot} of {insert} takes its sidechain from {source_node}, which has no sidechain route to it")]
    SidechainWithoutRoute {
        /// The insert.
        insert: String,
        /// The slot, 1 to 10.
        slot: usize,
        /// The sidechain source.
        source_node: String,
    },
    /// A pattern has two automation lanes for the same target.
    #[error("{pattern} has two automation lanes for the same target")]
    DuplicateLane {
        /// The pattern.
        pattern: String,
    },
    /// A layer channel contains itself, directly or through other layers.
    #[error("layer {channel} contains itself")]
    LayerCycle {
        /// The layer.
        channel: String,
    },
}

/// The broken invariants of a project.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the project breaks {} invariant(s); the first: {}", .0.len(), .0.first().map(ToString::to_string).unwrap_or_default())]
pub struct ValidationError(pub Vec<Violation>);

impl Project {
    /// Checks every invariant and reports all the broken ones.
    ///
    /// Complexity: O(N log N) in the total number of entities, notes and clips.
    pub fn validate(&self) -> Result<(), ValidationError> {
        let mut out = Vec::new();
        self.check_orders(&mut out);
        self.check_channels(&mut out);
        self.check_patterns(&mut out);
        self.check_tracks(&mut out);
        self.check_mixer(&mut out);
        let markers_sorted = self.markers.iter().zip(self.markers.iter().skip(1)).all(|(a, b)| a.at <= b.at);
        if !markers_sorted {
            out.push(Violation::Unsorted { owner: String::from("markers") });
        }
        if out.is_empty() { Ok(()) } else { Err(ValidationError(out)) }
    }

    fn check_orders(&self, out: &mut Vec<Violation>) {
        fn exact<K: Ord + Copy>(order: impl Iterator<Item = K>, keys: impl Iterator<Item = K>) -> bool {
            let mut listed: Vec<K> = order.collect();
            let count = listed.len();
            listed.sort_unstable();
            listed.dedup();
            let keys: Vec<K> = keys.collect();
            listed.len() == count && listed == keys
        }
        if !exact(self.rack_order.iter().copied(), self.channels.keys().copied()) {
            out.push(Violation::Order { list: "rackOrder" });
        }
        if !exact(self.track_order.iter().copied(), self.tracks.keys().copied()) {
            out.push(Violation::Order { list: "trackOrder" });
        }
        if !exact(self.mixer.order.iter().copied(), self.mixer.inserts.keys().copied()) || self.mixer.inserts.contains_key(&self.mixer.master.id) {
            out.push(Violation::Order { list: "mixer.order" });
        }
    }

    fn insert_exists(&self, id: InsertId) -> bool {
        self.mixer.insert(id).is_some()
    }

    fn check_channels(&self, out: &mut Vec<Violation>) {
        for (id, channel) in &self.channels {
            if let Some(target) = channel.target
                && !self.insert_exists(target)
            {
                out.push(Violation::Dangling { owner: id.to_string(), target: target.to_string() });
            }
            match &channel.kind {
                ChannelKind::Sampler(params) => {
                    if let Some(sample) = params.sample
                        && !self.samples.contains_key(&sample)
                    {
                        out.push(Violation::Dangling { owner: id.to_string(), target: sample.to_string() });
                    }
                    if params.root_key > 127 || params.key_range.1 > 127 {
                        out.push(Violation::KeyOutOfRange { owner: id.to_string(), key: params.root_key.max(params.key_range.1) });
                    }
                }
                ChannelKind::AudioClip { sample, .. } => {
                    if !self.samples.contains_key(sample) {
                        out.push(Violation::Dangling { owner: id.to_string(), target: sample.to_string() });
                    }
                }
                ChannelKind::Automation(lane) => {
                    if !lane.points.iter().zip(lane.points.iter().skip(1)).all(|(a, b)| a.at <= b.at) {
                        out.push(Violation::Unsorted { owner: format!("{id} automation") });
                    }
                }
                ChannelKind::Layer { children } => {
                    for child in children {
                        if !self.channels.contains_key(child) {
                            out.push(Violation::Dangling { owner: id.to_string(), target: child.to_string() });
                        }
                    }
                    if self.layer_reaches(*id, *id, &mut HashSet::new()) {
                        out.push(Violation::LayerCycle { channel: id.to_string() });
                    }
                }
                ChannelKind::Instrument { .. } | ChannelKind::MidiOut { .. } => {}
            }
        }
    }

    /// Whether the layers under `from` contain `target`.
    fn layer_reaches(&self, from: ChannelId, target: ChannelId, seen: &mut HashSet<ChannelId>) -> bool {
        let Some(ChannelKind::Layer { children }) = self.channels.get(&from).map(|c| &c.kind) else { return false };
        children.iter().any(|&child| child == target || (seen.insert(child) && self.layer_reaches(child, target, seen)))
    }

    fn check_patterns(&self, out: &mut Vec<Violation>) {
        for (id, pattern) in &self.patterns {
            let name = id.to_string();
            if let PatternLen::Fixed(len) = pattern.length
                && len <= Tick::ZERO
            {
                out.push(Violation::NonPositiveLength { owner: name.clone() });
            }
            if let Some(binding) = pattern.binding {
                if !self.channels.contains_key(&binding) {
                    out.push(Violation::Dangling { owner: name.clone(), target: binding.to_string() });
                }
                for channel in pattern.notes.keys().filter(|&&c| c != binding) {
                    out.push(Violation::BindingBroken { pattern: name.clone(), channel: binding.to_string(), other: channel.to_string() });
                }
            }
            let mut note_ids = HashSet::new();
            for (channel, seq) in &pattern.notes {
                if !self.channels.contains_key(channel) {
                    out.push(Violation::Dangling { owner: name.clone(), target: channel.to_string() });
                }
                if !seq.is_well_formed() {
                    out.push(Violation::Unsorted { owner: format!("{name} notes of {channel}") });
                }
                for note in seq.notes() {
                    if !note_ids.insert(note.id) || note.id.0 >= pattern.next_note_id {
                        out.push(Violation::BadNoteId { pattern: name.clone(), note: note.id.0 });
                    }
                    if note.key > 127 {
                        out.push(Violation::KeyOutOfRange { owner: format!("{name} note {}", note.id.0), key: note.key });
                    }
                }
            }
            let mut targets = HashSet::new();
            for lane in &pattern.automation {
                if !targets.insert(lane.target) {
                    out.push(Violation::DuplicateLane { pattern: name.clone() });
                }
            }
        }
    }

    fn check_tracks(&self, out: &mut Vec<Violation>) {
        let mut clip_ids = HashSet::new();
        for (id, track) in &self.tracks {
            if !track.clips.iter().zip(track.clips.iter().skip(1)).all(|(a, b)| a.start <= b.start) {
                out.push(Violation::Unsorted { owner: format!("{id} clips") });
            }
            for clip in &track.clips {
                if !clip_ids.insert(clip.id) {
                    out.push(Violation::DuplicateClip { clip: clip.id.to_string() });
                }
                if clip.len <= Tick::ZERO {
                    out.push(Violation::NonPositiveLength { owner: clip.id.to_string() });
                }
                match clip.source {
                    ClipSource::Pattern(pattern) => {
                        if !self.patterns.contains_key(&pattern) {
                            out.push(Violation::Dangling { owner: clip.id.to_string(), target: pattern.to_string() });
                        }
                    }
                    ClipSource::Audio(channel) | ClipSource::Automation(channel) => match self.channels.get(&channel).map(|c| &c.kind) {
                        None => out.push(Violation::Dangling { owner: clip.id.to_string(), target: channel.to_string() }),
                        Some(ChannelKind::AudioClip { .. }) if matches!(clip.source, ClipSource::Audio(_)) => {}
                        Some(ChannelKind::Automation(_)) if matches!(clip.source, ClipSource::Automation(_)) => {}
                        Some(_) => out.push(Violation::WrongClipSource { clip: clip.id.to_string(), source_channel: channel.to_string() }),
                    },
                }
            }
        }
    }

    fn check_mixer(&self, out: &mut Vec<Violation>) {
        let mixer = &self.mixer;
        let master = mixer.master.id;
        let mut dag = IncrementalDag::new();
        let mut nodes: HashMap<InsertId, DagNode> = HashMap::new();
        let mut names: HashMap<DagNode, InsertId> = HashMap::new();
        for &id in mixer.inserts.keys() {
            let node = dag.add_node();
            nodes.insert(id, node);
            names.insert(node, id);
        }
        for key in mixer.routes.keys() {
            let to_ok = mixer.inserts.contains_key(&key.to);
            let from_ok = match key.from {
                RouteSource::Channel(channel) => self.channels.contains_key(&channel),
                RouteSource::Insert(insert) => mixer.inserts.contains_key(&insert),
            };
            let from_name = match key.from {
                RouteSource::Channel(c) => c.to_string(),
                RouteSource::Insert(i) => i.to_string(),
            };
            if key.to == master || key.from == RouteSource::Insert(key.to) {
                out.push(Violation::BadRoute { from: from_name, to: key.to.to_string() });
                continue;
            }
            if !to_ok {
                out.push(Violation::Dangling { owner: format!("route from {from_name}"), target: key.to.to_string() });
                continue;
            }
            if !from_ok {
                out.push(Violation::Dangling { owner: format!("route to {}", key.to), target: from_name });
                continue;
            }
            // Sidechain routes count too: the source must still be rendered before its consumer.
            if let (RouteSource::Insert(from), Some(&to)) = (key.from, nodes.get(&key.to))
                && let Some(&from) = nodes.get(&from)
                && let Err(DagError::Cycle { path }) = dag.add_edge(from, to)
            {
                let path = path.iter().filter_map(|n| names.get(n)).map(ToString::to_string).collect();
                out.push(Violation::Cycle { path });
            }
        }
        for insert in mixer.inserts.values().chain(std::iter::once(&mixer.master)) {
            for (index, slot) in insert.slots.iter().enumerate().take(FX_SLOTS) {
                let Some(source) = slot.sidechain else { continue };
                let has_route =
                    mixer.routes.get(&crate::mixer::RouteKey { from: source, to: insert.id }).is_some_and(|route| route.kind == RouteKind::Sidechain);
                if !has_route {
                    let source_node = match source {
                        RouteSource::Channel(c) => c.to_string(),
                        RouteSource::Insert(i) => i.to_string(),
                    };
                    out.push(Violation::SidechainWithoutRoute { insert: insert.id.to_string(), slot: index + 1, source_node });
                }
            }
        }
    }
}
