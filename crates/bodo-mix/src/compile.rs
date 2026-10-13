//! Turning a project snapshot into the mix graph (project.md section 9.4, ADR 0025).
//!
//! **Shape.** Sampler and audio clip channels feed the input stage of their target insert, or of the master. Every
//! insert is three nodes, input, FX and output stages ([`crate::nodes::strip`]), chained in that order. Its output
//! stage feeds the master when `to_master` is on; audio routes leave the source at their tap (an insert's input
//! stage for PreFx, its FX stage for PreFader, its output stage for PostFader) and enter the target's input stage
//! scaled by their send level. A sidechain route enters the target's FX stage at a port of its own for each slot that
//! listens to it, `pre_k` frames into the stage's latency, so delay compensation lines it up with that slot.
//!
//! **Latency.** The FX stage reports the latency of its slots, disabled ones included, plus a positive track latency.
//! A negative track latency is a real delay at the end of the output stage that compensation doesn't see. Edges out of
//! an insert whose delay compensation is bypassed (always, or while recording) aren't compensated.
//!
//! **Solo.** The audible set is the soloed inserts, everything that feeds them and everything they feed, over audio
//! edges; the strips outside it keep running, silenced.
//!
//! **Updates.** Most edits never touch the graph: faders, pans, mutes, EQ, effect parameters and send levels go to the
//! running processors through shared atomics and edge gains, and effects are swapped into slots through queues.
//! Only a change of shape, of latency, or of a channel's sampler settings or sample compiles a new plan, and
//! processors whose settings didn't change carry over into it.
//!
//! **Crossfades.** A recompiled plan ramps every gain over 5 ms: new edges fade in, and edges and nodes the edit
//! removed stay one more plan, fading out, before a later recompile drops them ([`Compiler::expire_ghosts`]).

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use bodo_kit_audio::AudioData;
use bodo_kit_core::queue::{Consumer, Producer, spsc};
use bodo_kit_core::{MAX_BLOCK_FRAMES, SampleRate};
use bodo_kit_graph::{EdgeDesc, EdgeGains, EdgeKey, GainHandle, GraphDesc, LatencyReport, Layout, NodeDesc, NodeKey, NodeTimes, Plan, compile};
use bodo_model::automation::ParamTarget;
use bodo_model::channel::{ChannelKind, PluginFormat, SamplerParams};
use bodo_model::mixer::{FX_SLOTS, FxSlot, MixerInsert, PdcBypass, RouteKey, RouteKind, RouteSource};
use bodo_model::{ChannelId, InsertId, PluginInstanceId, Project, SampleRoots};

use crate::MixError;
use crate::audio_clips::stretch_keys;
use crate::automation::{LaneDelays, Param, param_key};
use crate::disk_sample::DiskCache;
use crate::fx::{EffectInfo, find};
use crate::keys;
use crate::live::{ChannelLive, NO_PORT, SlotLive, StripLive};
use crate::midi_out::{MidiOutNode, MidiOutTap};
use crate::nodes::strip::SlotKeys;
use crate::nodes::{AudioClipNode, MixNode, SamplerNode, SlotPatch, SlotQueues, SlotState, StripFx, StripInput, StripOutput};
use crate::record::{CaptureLinks, CaptureStage};
use crate::sample::SampleCache;
use crate::sampler_source::{DerivedCache, SamplerData};
use crate::stretch::StretchCache;

/// How long edges and nodes an edit removed stay in the graph, fading out, before a recompile drops them.
const GHOST_TIME: Duration = Duration::from_millis(40);
/// How many slot changes can wait for an FX stage at once.
const PATCH_QUEUE: usize = 32;
/// Gain ramps take at least this long, so topology changes and send-level changes never click.
const RAMP_SECONDS: f64 = 0.005;
/// Marks a tap that feeds device outputs; the low bits hold the channels.
pub(crate) const HARDWARE_TAP: u64 = 1 << 63;

/// What a node of the mix graph is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Role {
    /// A sampler or audio clip channel.
    Channel(ChannelId),
    /// An insert's input stage.
    Input(InsertId),
    /// An insert's FX stage.
    Fx(InsertId),
    /// An insert's output stage.
    Output(InsertId),
}

/// A signal that enters the master, which a stem export writes to its own file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StemSource {
    /// The insert, or the channel playing straight into the master.
    pub source: RouteSource,
    /// Its name.
    pub name: String,
    /// The graph edge it enters the master on, whose compensation delay the stem gets too.
    pub(crate) edge: EdgeKey,
}

/// The mix graph's shape for a project, with whatever processors were asked for.
pub(crate) struct Topology<P> {
    pub(crate) desc: GraphDesc<P>,
    /// The sidechain port of each slot that has one: insert, slot index, port.
    pub(crate) sidechain_ports: Vec<(InsertId, usize, u32)>,
    /// The latency of each insert's slots.
    pub(crate) slot_latency: HashMap<InsertId, [u32; FX_SLOTS]>,
    /// Stem sources, by tap index, when stems were asked for.
    pub(crate) stems: Vec<StemSource>,
}

/// The native effect a slot runs, if it holds one that Bodo knows.
pub(crate) fn native(slot: &FxSlot) -> Option<(PluginInstanceId, &'static EffectInfo)> {
    let plugin = slot.plugin.as_ref()?;
    if plugin.format != PluginFormat::Native {
        return None;
    }
    find(&plugin.uid).map(|info| (plugin.instance, info))
}

/// The values of `info`'s parameters in `slot`, in order, each at its default unless the slot set it.
pub(crate) fn slot_values(info: &EffectInfo, slot: &FxSlot) -> Vec<f32> {
    info.params.iter().map(|p| slot.param(p.id).map_or(p.default, |v| p.clamp(v))).collect()
}

/// The latency of each slot of `insert` at `rate`.
pub(crate) fn slot_latencies(insert: &MixerInsert, rate: SampleRate) -> [u32; FX_SLOTS] {
    let mut out = [0u32; FX_SLOTS];
    for (latency, slot) in out.iter_mut().zip(&insert.slots) {
        if let Some((_, info)) = native(slot) {
            *latency = (info.latency)(&slot_values(info, slot), rate).min((info.max_latency)(rate));
        }
    }
    out
}

/// Whether delay compensation is bypassed for `insert` now.
fn bypassed(insert: &MixerInsert, recording: bool) -> bool {
    match insert.pdc_bypass {
        PdcBypass::Never => false,
        PdcBypass::Always => true,
        PdcBypass::WhileRecording => recording,
    }
}

/// Whether channel `id` has a node: samplers, audio clip channels and MIDI Out channels (silent, sending their notes
/// to a port) do.
fn has_node(project: &Project, id: ChannelId) -> bool {
    project.channels.get(&id).is_some_and(|c| matches!(c.kind, ChannelKind::Sampler(_) | ChannelKind::AudioClip { .. } | ChannelKind::MidiOut { .. }))
}

/// The tag of a tap feeding device outputs `channels`: one channel for a mono output, two for stereo.
fn hardware_tag(channels: &[u16]) -> Option<u64> {
    let code = |c: u16| u64::from(c) + 1;
    match channels {
        [mono] => Some(HARDWARE_TAP | code(*mono)),
        [left, right, ..] => Some(HARDWARE_TAP | code(*left) | code(*right) << 16),
        [] => None,
    }
}

/// The device output channels a hardware tap's `tag` feeds: left and right, the same for a mono output.
pub(crate) fn hardware_channels(tag: u64) -> Option<(usize, Option<usize>)> {
    if tag & HARDWARE_TAP == 0 {
        return None;
    }
    let left = (tag & 0xFFFF) as usize;
    let right = ((tag >> 16) & 0xFFFF) as usize;
    (left > 0).then(|| (left - 1, (right > 0).then(|| right - 1)))
}

/// The mix graph of `project`, with processors from `make`. `recording` decides the delay compensation bypassed while
/// recording; `stems` taps every signal entering the master.
///
/// Complexity: O(V + E) in channels, inserts and routes.
pub(crate) fn topology<P>(project: &Project, recording: bool, stems: bool, mut make: impl FnMut(Role) -> Option<P>) -> Topology<P> {
    let rate = project.settings.sample_rate;
    let master = project.mixer.master.id;
    let gain_ramp = (f64::from(rate.hz()) * RAMP_SECONDS).round() as u32;
    let mut desc = GraphDesc { nodes: Vec::new(), edges: Vec::new(), output: Some(keys::insert(master)), taps: Vec::new(), gain_ramp };
    let mut stem_sources = Vec::new();
    let edge = |key, from, to, port, port_offset, gain, compensate| EdgeDesc { key, from, to, port, port_offset, gain, compensate };
    for id in &project.rack_order {
        let Some(channel) = project.channels.get(id).filter(|_| has_node(project, *id)) else { continue };
        let key = keys::channel(*id);
        desc.nodes.push(NodeDesc { key, channels: 2, ports: 0, latency: 0, processor: make(Role::Channel(*id)) });
        let target = channel.target.filter(|t| project.mixer.insert(*t).is_some()).unwrap_or(master);
        let edge_key = keys::edge(key, keys::insert_input(target)) ^ 1;
        desc.edges.push(edge(edge_key, key, keys::insert_input(target), 0, 0, 1.0, true));
        if target == master && stems {
            stem_sources.push(StemSource { source: RouteSource::Channel(*id), name: channel.name.clone(), edge: edge_key });
        }
    }
    let mut sidechain_ports = Vec::new();
    let mut slot_latency = HashMap::new();
    let fed = if stems { fed_inserts(project) } else { HashSet::new() };
    let inserts = project.mixer.order.iter().copied().chain(std::iter::once(master));
    for id in inserts {
        let Some(insert) = project.mixer.insert(id) else { continue };
        let latencies = slot_latencies(insert, rate);
        // A port per slot that listens to an existing sidechain route, after the main input.
        let mut sidechains = Vec::new();
        for (k, slot) in insert.slots.iter().enumerate() {
            let Some(source) = slot.sidechain else { continue };
            if native(slot).is_none() {
                continue;
            }
            let route_key = RouteKey { from: source, to: id };
            let Some(route) = project.mixer.routes.get(&route_key).filter(|r| r.kind == RouteKind::Sidechain) else { continue };
            let from = match source {
                RouteSource::Channel(c) if has_node(project, c) => keys::channel(c),
                RouteSource::Insert(i) if project.mixer.insert(i).is_some() && i != id => keys::route_source(source, route.tap),
                _ => continue,
            };
            let pre: u32 = latencies.iter().take(k).sum();
            let port = sidechains.len() + 1;
            sidechains.push((k, port, from, route.send_level, pre, route_key));
        }
        let declared = u32::try_from(insert.latency_offset.0.max(0)).unwrap_or(u32::MAX);
        let fx_latency = latencies.iter().sum::<u32>().saturating_add(declared);
        let (input, fx, output) = (keys::insert_input(id), keys::insert_fx(id), keys::insert(id));
        desc.nodes.push(NodeDesc { key: input, channels: 2, ports: 1, latency: 0, processor: make(Role::Input(id)) });
        desc.nodes.push(NodeDesc { key: fx, channels: 2, ports: 1 + sidechains.len(), latency: fx_latency, processor: make(Role::Fx(id)) });
        desc.nodes.push(NodeDesc { key: output, channels: 2, ports: 1, latency: 0, processor: make(Role::Output(id)) });
        desc.edges.push(edge(keys::edge(input, fx), input, fx, 0, 0, 1.0, true));
        desc.edges.push(edge(keys::edge(fx, output), fx, output, 0, 0, 1.0, true));
        for (k, port, from, level, pre, route_key) in sidechains {
            desc.edges.push(edge(keys::route(route_key) ^ (0x5C00 + k as u64), from, fx, port, pre, level, true));
            sidechain_ports.push((id, k, port as u32));
        }
        if id != master && insert.to_master {
            let edge_key = keys::edge(output, keys::insert_input(master)) ^ 3;
            desc.edges.push(edge(edge_key, output, keys::insert_input(master), 0, 0, 1.0, !bypassed(insert, recording)));
            if stems && fed.contains(&id) {
                stem_sources.push(StemSource { source: RouteSource::Insert(id), name: insert.name.clone(), edge: edge_key });
            }
        }
        if let Some(tag) = insert.output.as_ref().and_then(|port| hardware_tag(&port.channels)) {
            desc.taps.push((output, tag));
        }
        slot_latency.insert(id, latencies);
    }
    for (key, route) in &project.mixer.routes {
        if route.kind != RouteKind::Audio || project.mixer.insert(key.to).is_none() || key.to == master {
            continue;
        }
        let (from, compensate) = match key.from {
            RouteSource::Channel(c) if has_node(project, c) => (keys::channel(c), true),
            RouteSource::Insert(i) if i != key.to => match project.mixer.insert(i) {
                Some(source) => (keys::route_source(key.from, route.tap), !bypassed(source, recording)),
                None => continue,
            },
            _ => continue,
        };
        desc.edges.push(edge(keys::route(*key), from, keys::insert_input(key.to), 0, 0, route.send_level, compensate));
    }
    if stems {
        desc.taps.extend(stem_sources.iter().enumerate().map(|(i, s)| (s.edge_source(&desc.edges), i as u64)));
    }
    Topology { desc, sidechain_ports, slot_latency, stems: stem_sources }
}

/// The inserts some channel's audio reaches, through targets and audio routes: the only ones that can sound.
///
/// Complexity: O(V + E) in channels, inserts and routes.
fn fed_inserts(project: &Project) -> HashSet<InsertId> {
    let master = project.mixer.master.id;
    let mut fed: HashSet<InsertId> = HashSet::new();
    let mut stack: Vec<InsertId> = project
        .rack_order
        .iter()
        .filter(|id| has_node(project, **id))
        .filter_map(|id| project.channels.get(id))
        .map(|c| c.target.filter(|t| project.mixer.insert(*t).is_some()).unwrap_or(master))
        .collect();
    stack.extend(project.mixer.routes.iter().filter_map(|(key, route)| match key.from {
        RouteSource::Channel(c) if route.kind == RouteKind::Audio && has_node(project, c) => Some(key.to),
        _ => None,
    }));
    while let Some(id) = stack.pop() {
        if fed.insert(id) {
            stack.extend(
                project
                    .mixer
                    .routes
                    .iter()
                    .filter_map(|(key, route)| (key.from == RouteSource::Insert(id) && route.kind == RouteKind::Audio).then_some(key.to)),
            );
        }
    }
    fed
}

impl StemSource {
    /// The node feeding this stem's edge.
    fn edge_source(&self, edges: &[EdgeDesc]) -> NodeKey {
        edges.iter().find(|e| e.key == self.edge).map_or(0, |e| e.from)
    }
}

/// What a sampler node was built from, to tell whether it must be rebuilt.
#[derive(Clone, Debug)]
struct SamplerSource {
    params: Arc<SamplerParams>,
    data: SamplerData,
}

impl SamplerSource {
    fn same_as(&self, other: &SamplerSource) -> bool {
        let same_sample = match (&self.data.sample, &other.data.sample) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        let same_disk = match (&self.data.disk, &other.data.disk) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        (Arc::ptr_eq(&self.params, &other.params) || self.params == other.params) && same_sample && same_disk && self.data.zones == other.data.zones
    }
}

/// What an audio clip node was built from: the sample and its time-stretched copies.
#[derive(Clone, Debug)]
struct AudioSource {
    sample: Option<Arc<AudioData>>,
    stretched: Vec<(u32, Arc<AudioData>)>,
}

impl AudioSource {
    fn same_as(&self, other: &AudioSource) -> bool {
        let same_sample = match (&self.sample, &other.sample) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        same_sample
            && self.stretched.len() == other.stretched.len()
            && self.stretched.iter().zip(&other.stretched).all(|((ka, a), (kb, b))| ka == kb && Arc::ptr_eq(a, b))
    }
}

/// An effect the control side has loaded into a slot.
#[derive(Debug)]
struct Loaded {
    instance: PluginInstanceId,
    info: &'static EffectInfo,
    live: Arc<SlotLive>,
}

/// The control side of an insert's FX stage: what its slots hold, and the queues to change them.
#[derive(Debug, Default)]
struct FxLink {
    loaded: [Option<Loaded>; FX_SLOTS],
    /// The queues of the running FX stage, once it is built.
    patches: Option<Producer<Box<SlotPatch>>>,
    garbage: Option<Consumer<Box<SlotPatch>>>,
    /// Changes waiting for room in the queue.
    pending: VecDeque<Box<SlotPatch>>,
}

/// The automation keys of slot `index` of `insert`, running an effect `info` that is plugin `instance`.
fn slot_keys(insert: InsertId, index: usize, instance: PluginInstanceId, info: &EffectInfo) -> SlotKeys {
    let index = index as u8;
    SlotKeys {
        params: info.params.iter().map(|p| keys::plugin_param(instance, p.id)).collect(),
        mix: keys::slot_param(insert, index, 0),
        enabled: keys::slot_param(insert, index, 1),
    }
}

impl FxLink {
    /// The slot state a new FX stage starts with for slot `index`: a fresh instance of what is loaded there.
    ///
    /// Allocates.
    fn fresh_state(&self, insert: InsertId, index: usize, rate: SampleRate, max_frames: usize) -> SlotState {
        match self.loaded.get(index).and_then(Option::as_ref) {
            Some(loaded) => SlotState::new(
                (loaded.info.create)(rate, max_frames),
                loaded.info,
                Arc::clone(&loaded.live),
                slot_keys(insert, index, loaded.instance, loaded.info),
                rate,
            ),
            None => SlotState::default(),
        }
    }

    /// Sends a slot change to the running stage, or keeps it until there is room.
    fn send(&mut self, patch: Box<SlotPatch>) {
        self.pending.push_back(patch);
        self.flush();
    }

    /// Drops the slots the stage handed back and sends what was waiting.
    fn flush(&mut self) {
        if let Some(garbage) = self.garbage.as_mut() {
            while garbage.pop().is_some() {}
        }
        let Some(patches) = self.patches.as_mut() else {
            self.pending.clear();
            return;
        };
        while let Some(patch) = self.pending.pop_front() {
            if let Err(full) = patches.push(patch) {
                self.pending.push_front(full.0);
                break;
            }
        }
    }
}

/// The shape of a compiled graph, kept to find what the next edit removes.
#[derive(Clone, Debug, Default, PartialEq)]
struct Shape {
    nodes: Vec<(NodeKey, usize, u32)>,
    edges: Vec<EdgeDesc>,
    taps: Vec<(NodeKey, u64)>,
}

impl Shape {
    fn of<P>(desc: &GraphDesc<P>) -> Self {
        Self { nodes: desc.nodes.iter().map(|n| (n.key, n.ports, n.latency)).collect(), edges: desc.edges.clone(), taps: desc.taps.clone() }
    }
}

/// Automation targets the audio thread needs with each plan: how late each parameter's audio is, and the routes whose
/// send levels are automated, with their edges' gains and the level to fall back to.
#[derive(Debug, Default)]
pub(crate) struct AutomationTargets {
    pub(crate) delays: LaneDelays,
    pub(crate) routes: Vec<(u64, GainHandle, f32)>,
}

/// A topology with the edges and nodes the last plan had and this one doesn't, kept silent for one more plan.
enum Ghosted<P> {
    /// Nothing was removed.
    Plain(GraphDesc<P>),
    /// The topology with ghosts, and without them in case the ghosts make it uncompilable.
    With { full: GraphDesc<P>, plain: Box<GraphDesc<P>> },
}

/// Adds to `desc` the nodes and edges of `previous` it no longer has: nodes keep their processors, edges fade to
/// silence and are left out of delay compensation.
fn with_ghosts(desc: GraphDesc<MixNode>, previous: &Shape) -> Ghosted<MixNode> {
    let nodes: HashSet<NodeKey> = desc.nodes.iter().map(|n| n.key).collect();
    let edges: HashSet<EdgeKey> = desc.edges.iter().map(|e| e.key).collect();
    let ghost_nodes: Vec<&(NodeKey, usize, u32)> = previous.nodes.iter().filter(|(key, ..)| !nodes.contains(key)).collect();
    let ghost_edges: Vec<&EdgeDesc> = previous.edges.iter().filter(|e| !edges.contains(&e.key)).collect();
    if ghost_nodes.is_empty() && ghost_edges.is_empty() {
        return Ghosted::Plain(desc);
    }
    let mut full =
        GraphDesc { nodes: Vec::new(), edges: desc.edges.clone(), output: desc.output, taps: desc.taps.clone(), gain_ramp: desc.gain_ramp };
    let mut plain =
        GraphDesc { nodes: Vec::with_capacity(desc.nodes.len()), edges: desc.edges, output: desc.output, taps: desc.taps, gain_ramp: desc.gain_ramp };
    // Processors can't be cloned, so the plain fallback keeps them and the ghosted one only adopts them: compiling it
    // succeeds unless the ghosts close a loop, and then the plain one runs.
    let ports: HashMap<NodeKey, usize> = desc.nodes.iter().map(|n| (n.key, n.ports)).chain(ghost_nodes.iter().map(|&&(k, p, _)| (k, p))).collect();
    for node in desc.nodes {
        full.nodes.push(NodeDesc { key: node.key, channels: node.channels, ports: node.ports, latency: node.latency, processor: None });
        plain.nodes.push(node);
    }
    full.nodes.extend(ghost_nodes.iter().map(|&&(key, ports, latency)| NodeDesc { key, channels: 2, ports, latency, processor: None }));
    for edge in ghost_edges {
        let fits = ports.get(&edge.to).is_some_and(|&p| edge.port < p) && ports.contains_key(&edge.from);
        if fits {
            full.edges.push(EdgeDesc { gain: 0.0, compensate: false, ..*edge });
        }
    }
    Ghosted::With { full, plain: Box::new(plain) }
}

/// The MIDI Out channels of `project`: each one's port and MIDI channel.
fn midi_out_channels(project: &Project) -> HashMap<ChannelId, (String, u8)> {
    project
        .channels
        .values()
        .filter_map(|c| match &c.kind {
            ChannelKind::MidiOut { port, channel } => Some((c.id, (port.clone(), *channel))),
            _ => None,
        })
        .collect()
}

/// What decides the MIDI Out nodes, flattened in rack order, for the graph's signature.
fn midi_out_signature<'a>(project: &'a Project, midi_out: &'a HashMap<ChannelId, (String, u8)>) -> impl Iterator<Item = u64> + 'a {
    project.rack_order.iter().filter_map(move |id| {
        let (port, channel) = midi_out.get(id)?;
        let port = port.bytes().fold(0xCBF2_9CE4_8422_2325u64, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01B3));
        Some(keys::channel(*id) ^ port ^ (u64::from(*channel) << 56))
    })
}

/// The parts of `project` that decide a channel node's processor: its sampler settings and sample, or its audio and
/// stretched copies. Two equal signatures need no new channel processors.
fn processor_signature(project: &Project, sources: &HashMap<ChannelId, SamplerSource>, audio: &HashMap<ChannelId, AudioSource>) -> Vec<u64> {
    let mut out = vec![project.settings.pan_law as u64];
    for id in &project.rack_order {
        if let Some(source) = sources.get(id) {
            out.push(keys::channel(*id));
            out.push(Arc::as_ptr(&source.params) as usize as u64);
            out.push(source.data.sample.as_ref().map_or(0, |s| Arc::as_ptr(s) as usize as u64));
            out.push(source.data.disk.as_ref().map_or(0, |d| Arc::as_ptr(d) as usize as u64));
        }
        if let Some(source) = audio.get(id) {
            out.push(keys::channel(*id) ^ 0xA0D1);
            out.push(source.sample.as_ref().map_or(0, |s| Arc::as_ptr(s) as usize as u64));
            out.extend(source.stretched.iter().flat_map(|(key, data)| [u64::from(*key), Arc::as_ptr(data) as usize as u64]));
        }
    }
    for insert in project.mixer.inserts.values().chain(std::iter::once(&project.mixer.master)) {
        out.push(keys::insert(insert.id) ^ insert.latency_offset.0.min(0).unsigned_abs());
    }
    out
}

/// The routes whose send levels automation drives.
fn automated_routes(project: &Project) -> HashSet<RouteKey> {
    let mut out = HashSet::new();
    let mut note = |target: &ParamTarget| {
        if let ParamTarget::Route(key) = target {
            out.insert(*key);
        }
    };
    for channel in project.channels.values() {
        if let ChannelKind::Automation(lane) = &channel.kind {
            note(&lane.target);
        }
    }
    for pattern in project.patterns.values() {
        for lane in &pattern.automation {
            note(&lane.target);
        }
    }
    out
}

/// Sets every route's edges to its send level, except routes automation drives.
fn set_gains(project: &Project, gains: &EdgeGains) {
    let automated = automated_routes(project);
    for (key, route) in &project.mixer.routes {
        if automated.contains(key) {
            continue;
        }
        match route.kind {
            RouteKind::Audio => {
                gains.set(keys::route(*key), route.send_level);
            }
            RouteKind::Sidechain => {
                for k in 0..FX_SLOTS as u64 {
                    gains.set(keys::route(*key) ^ (0x5C00 + k), route.send_level);
                }
            }
        }
    }
}

/// Keeps the graph and the live parameters in step with the project.
#[derive(Debug)]
pub(crate) struct Compiler {
    roots: SampleRoots,
    max_frames: usize,
    cache: SampleCache,
    layout: Option<Layout>,
    /// What the running plan's sampler and audio clip nodes were built from.
    samplers: HashMap<ChannelId, SamplerSource>,
    audio: HashMap<ChannelId, AudioSource>,
    stretch: StretchCache,
    /// Samples with the samplers' precomputed effects rendered in.
    derived: DerivedCache,
    /// Whether samplers may stream their samples from disk; offline renders load them fully.
    streaming: bool,
    /// Samples that samplers stream from disk.
    disk: DiskCache,
    /// The MIDI Out channels the running plan has nodes for: each one's port and MIDI channel.
    midi_out: HashMap<ChannelId, (String, u8)>,
    /// The control side's ends of the MIDI Out channels' queues.
    midi_tap: MidiOutTap,
    channels: HashMap<ChannelId, Arc<ChannelLive>>,
    strips: HashMap<InsertId, Arc<StripLive>>,
    links: HashMap<InsertId, FxLink>,
    /// Inserts the running plan has stages for, with the negative latency delay their output stage was built with.
    built: HashMap<InsertId, usize>,
    /// The control side's links to every strip stage's recording capture.
    captures: CaptureLinks,
    signature: Option<(Shape, Vec<u64>)>,
    gains: Option<EdgeGains>,
    times: Option<NodeTimes>,
    latency: LatencyReport,
    rate: SampleRate,
    /// The project of the last update, for recompiles the engine asks for.
    last: Option<Project>,
    /// Edges and nodes the last edit removed are in the running plan, fading out, since this time.
    ghosts_since: Option<Instant>,
    recording: bool,
    stems: bool,
    stem_sources: Vec<StemSource>,
    /// The automation targets of the plan last compiled, waiting to be sent with it.
    targets: Option<Box<AutomationTargets>>,
    /// The parameter delays of the running plan, kept to refresh the targets when only levels change.
    delays: LaneDelays,
    /// The routes automation drives, as last sent.
    sent_routes: Vec<(u64, f32)>,
}

impl Compiler {
    pub(crate) fn new(roots: SampleRoots, max_frames: usize, rate: SampleRate, stems: bool) -> Self {
        Self {
            roots,
            max_frames: max_frames.clamp(1, MAX_BLOCK_FRAMES),
            cache: SampleCache::new(),
            layout: None,
            samplers: HashMap::new(),
            audio: HashMap::new(),
            stretch: StretchCache::default(),
            derived: DerivedCache::default(),
            streaming: false,
            disk: DiskCache::default(),
            midi_out: HashMap::new(),
            midi_tap: MidiOutTap::default(),
            channels: HashMap::new(),
            strips: HashMap::new(),
            links: HashMap::new(),
            built: HashMap::new(),
            captures: CaptureLinks::default(),
            signature: None,
            gains: None,
            times: None,
            latency: LatencyReport::default(),
            rate,
            last: None,
            ghosts_since: None,
            recording: false,
            stems,
            stem_sources: Vec::new(),
            targets: None,
            delays: LaneDelays::default(),
            sent_routes: Vec::new(),
        }
    }

    /// Lets samplers that keep their sample on disk stream it, from the next update on. Real-time engines do; offline
    /// renders must never wait for the disk, so they load every sample fully.
    pub(crate) fn set_streaming(&mut self, streaming: bool) {
        self.streaming = streaming;
    }

    /// The control side's ends of the MIDI Out channels' queues.
    pub(crate) fn midi_out(&self) -> MidiOutTap {
        self.midi_tap.clone()
    }

    /// Resolves sample paths through `roots` from the next update on, and tries missing samples again.
    pub(crate) fn set_roots(&mut self, roots: SampleRoots) {
        self.roots = roots;
        self.cache.retry_failed();
    }

    /// The links to every strip stage's recording capture.
    pub(crate) fn captures_mut(&mut self) -> &mut CaptureLinks {
        &mut self.captures
    }

    /// What delay compensation decided for the running plan.
    pub(crate) fn latency(&self) -> &LatencyReport {
        &self.latency
    }

    /// The live settings of every insert, with their meters.
    pub(crate) fn strips(&self) -> &HashMap<InsertId, Arc<StripLive>> {
        &self.strips
    }

    /// How long each node of the running plan has spent processing.
    pub(crate) fn times(&self) -> Option<&NodeTimes> {
        self.times.as_ref()
    }

    /// The stems, by tap index, of an engine built for stems.
    pub(crate) fn stem_sources(&self) -> &[StemSource] {
        &self.stem_sources
    }

    /// The automation targets of the newest plan, once, for the audio thread.
    pub(crate) fn take_targets(&mut self) -> Option<Box<AutomationTargets>> {
        self.targets.take()
    }

    /// Recompiles, when it matters, with delay compensation bypassed or not for the inserts that bypass it while
    /// recording.
    pub(crate) fn set_recording(&mut self, recording: bool) -> Result<Option<Plan<MixNode>>, MixError> {
        if self.recording == recording {
            return Ok(None);
        }
        self.recording = recording;
        let Some(project) = self.last.take() else { return Ok(None) };
        let result = self.update(&project);
        self.last = Some(project);
        result
    }

    /// Recompiles without the edges and nodes the last edit removed, once they have faded out. Returns the new plan,
    /// if one was due.
    pub(crate) fn expire_ghosts(&mut self, now: Instant) -> Result<Option<Plan<MixNode>>, MixError> {
        let Some(since) = self.ghosts_since else { return Ok(None) };
        if now.duration_since(since) < GHOST_TIME {
            return Ok(None);
        }
        let Some(project) = self.last.take() else { return Ok(None) };
        let result = self.compile_plan(&project, None, false);
        self.last = Some(project);
        result.map(Some)
    }

    /// Drops the slots FX stages handed back and sends slot changes that were waiting.
    pub(crate) fn collect(&mut self) {
        for link in self.links.values_mut() {
            link.flush();
        }
    }

    /// Brings everything up to date with `project`. Returns a new plan when the graph changed.
    ///
    /// Allocates and may load samples. Complexity: O(V + E) plus sample loading and, on a recompile, the plan's
    /// buffers.
    pub(crate) fn update(&mut self, project: &Project) -> Result<Option<Plan<MixNode>>, MixError> {
        self.last = Some(project.clone());
        self.update_live(project);
        let sources = self.sampler_sources(project);
        let audio = self.audio_sources(project);
        let shape = topology(project, self.recording, self.stems, |_| None::<()>);
        let mut extra = processor_signature(project, &sources, &audio);
        extra.extend(midi_out_signature(project, &midi_out_channels(project)));
        let signature = (Shape::of(&shape.desc), extra);
        self.set_sidechain_ports(&shape.sidechain_ports);
        if self.signature.as_ref() == Some(&signature)
            && let Some(gains) = &self.gains
        {
            set_gains(project, gains);
            self.refresh_targets(project);
            return Ok(None);
        }
        let plan = self.compile_plan(project, Some((sources, audio)), true)?;
        self.signature = Some(signature);
        self.stretch.trim();
        self.derived.trim();
        self.disk.trim();
        self.cache.trim();
        Ok(Some(plan))
    }

    /// Builds the topology with processors and compiles it. `sources` are the channels' new sampler and audio
    /// sources, or `None` to keep the running ones. With `ghosts`, the edges and nodes the last plan had and this one
    /// doesn't stay one more plan, fading out.
    fn compile_plan(
        &mut self,
        project: &Project,
        sources: Option<(HashMap<ChannelId, SamplerSource>, HashMap<ChannelId, AudioSource>)>,
        ghosts: bool,
    ) -> Result<Plan<MixNode>, MixError> {
        let master = project.mixer.master.id;
        let (rate, max_frames, pan_law) = (self.rate, self.max_frames, project.settings.pan_law);
        let (new_samplers, new_audio) = sources.unwrap_or_else(|| (self.samplers.clone(), self.audio.clone()));
        let midi_out = midi_out_channels(project);
        let previous_shape = self.signature.as_ref().map(|(shape, _)| shape.clone());
        let mut built = HashMap::new();
        let topology = {
            let this = &mut *self;
            topology(project, this.recording, this.stems, |role| match role {
                Role::Channel(id) => {
                    let live = this.channels.get(&id).cloned().unwrap_or_default();
                    if let Some(source) = new_samplers.get(&id) {
                        let fresh = this.samplers.get(&id).is_none_or(|old| !old.same_as(source));
                        fresh.then(|| {
                            MixNode::Sampler(Box::new(SamplerNode::new(keys::channel(id), source.data.clone(), &source.params, live, pan_law, rate)))
                        })
                    } else if let Some(port) = midi_out.get(&id) {
                        // A silent node whose notes go to a MIDI port; a new port or MIDI channel opens a new queue.
                        let fresh = this.midi_out.get(&id) != Some(port);
                        fresh.then(|| MixNode::MidiOut(Box::new(MidiOutNode::new(keys::channel(id), port.1, this.midi_tap.open(id, &port.0)))))
                    } else {
                        let source = new_audio.get(&id)?;
                        let fresh = this.audio.get(&id).is_none_or(|old| !old.same_as(source));
                        fresh.then(|| {
                            MixNode::AudioClip(Box::new(AudioClipNode::new(
                                keys::channel(id),
                                source.sample.clone(),
                                source.stretched.clone(),
                                live,
                                pan_law,
                            )))
                        })
                    }
                }
                Role::Input(id) => {
                    if this.built.contains_key(&id) {
                        return None;
                    }
                    let live = this.strips.get(&id).cloned().unwrap_or_default();
                    Some(MixNode::StripInput(StripInput::new(live).with_capture(this.captures.link(id, CaptureStage::Input))))
                }
                Role::Fx(id) => {
                    if this.built.contains_key(&id) {
                        return None;
                    }
                    let live = this.strips.get(&id).cloned().unwrap_or_default();
                    let link = this.links.entry(id).or_default();
                    let (patches_tx, patches_rx) = spsc(PATCH_QUEUE);
                    let (garbage_tx, garbage_rx) = spsc(PATCH_QUEUE);
                    link.patches = Some(patches_tx);
                    link.garbage = Some(garbage_rx);
                    link.pending.clear();
                    let slots: [SlotState; FX_SLOTS] = std::array::from_fn(|index| link.fresh_state(id, index, rate, max_frames));
                    let stage = StripFx::new(keys::insert(id), live, slots, SlotQueues { patches: patches_rx, garbage: garbage_tx }, max_frames);
                    Some(MixNode::StripFx(Box::new(stage.with_capture(this.captures.link(id, CaptureStage::Fx)))))
                }
                Role::Output(id) => {
                    let insert = project.mixer.insert(id)?;
                    let delay = usize::try_from((-insert.latency_offset.0).max(0)).unwrap_or(0);
                    built.insert(id, delay);
                    if this.built.get(&id) == Some(&delay) {
                        return None;
                    }
                    let live = this.strips.get(&id).cloned().unwrap_or_default();
                    let stage = StripOutput::new(keys::insert(id), live, delay, id == master);
                    Some(MixNode::StripOutput(stage.with_capture(this.captures.link(id, CaptureStage::Output))))
                }
            })
        };
        let Topology { desc, sidechain_ports, slot_latency, stems } = topology;
        let shape = Shape::of(&desc);
        let ghosted = match (ghosts, previous_shape) {
            (true, Some(previous)) => with_ghosts(desc, &previous),
            _ => Ghosted::Plain(desc),
        };
        let (compiled, had_ghosts) = match ghosted {
            Ghosted::Plain(desc) => (compile(desc, self.max_frames, self.layout.as_ref())?, false),
            Ghosted::With { full, plain } => match compile(full, self.max_frames, self.layout.as_ref()) {
                Ok(compiled) => (compiled, true),
                Err(_) => (compile(*plain, self.max_frames, self.layout.as_ref())?, false),
            },
        };
        self.layout = Some(compiled.layout);
        self.gains = Some(compiled.gains);
        self.times = Some(compiled.times);
        self.latency = compiled.latency;
        self.stem_sources = stems;
        self.samplers = new_samplers;
        self.audio = new_audio;
        self.midi_tap.retain(|id| midi_out.contains_key(id));
        self.midi_out = midi_out;
        self.built = built;
        if let Some((_, extra)) = self.signature.take() {
            self.signature = Some((shape, extra));
        }
        let live_inserts: HashSet<InsertId> = self.built.keys().copied().collect();
        self.captures.retain(|id| live_inserts.contains(id));
        self.links.retain(|id, _| live_inserts.contains(id));
        self.ghosts_since = had_ghosts.then(Instant::now);
        self.set_sidechain_ports(&sidechain_ports);
        self.delays = self.lane_delays(project, &slot_latency);
        self.sent_routes.clear();
        self.refresh_targets(project);
        if self.targets.is_none() {
            self.targets = Some(Box::new(AutomationTargets { delays: self.delays.clone(), routes: Vec::new() }));
        }
        Ok(compiled.plan)
    }

    /// How late each automated parameter's audio is in the running plan.
    fn lane_delays(&self, project: &Project, slot_latency: &HashMap<InsertId, [u32; FX_SLOTS]>) -> LaneDelays {
        let node = |key: NodeKey| self.latency.nodes.get(&key).copied().unwrap_or_default();
        let mut entries = Vec::new();
        for insert in project.mixer.inserts.values().chain(std::iter::once(&project.mixer.master)) {
            let id = insert.id;
            let output = node(keys::insert(id)).input;
            entries.push((param_key(keys::insert(id), Param::Volume), output));
            entries.push((param_key(keys::insert(id), Param::Pan), output));
            let fx_in = node(keys::insert_fx(id)).input;
            let latencies = slot_latency.get(&id).copied().unwrap_or_default();
            entries.push((param_key(keys::insert(id), Param::StereoSep), fx_in + latencies.iter().sum::<u32>()));
            for (k, slot) in insert.slots.iter().enumerate() {
                let at = fx_in + latencies.iter().take(k).sum::<u32>();
                entries.push((keys::slot_param(id, k as u8, 0), at));
                entries.push((keys::slot_param(id, k as u8, 1), at));
                if let Some((instance, info)) = native(slot) {
                    entries.extend(info.params.iter().map(|p| (keys::plugin_param(instance, p.id), at)));
                }
            }
        }
        for (key, route) in &project.mixer.routes {
            let from = keys::route_source(key.from, route.tap);
            entries.push((keys::route_level(*key), node(from).output));
        }
        LaneDelays::new(entries)
    }

    /// Queues new automation targets for the audio thread when the automated routes or their levels changed.
    fn refresh_targets(&mut self, project: &Project) {
        let Some(gains) = &self.gains else { return };
        let automated = automated_routes(project);
        let mut routes: Vec<(u64, GainHandle, f32)> = Vec::new();
        for key in &automated {
            let Some(route) = project.mixer.routes.get(key) else { continue };
            let edge_keys: Vec<EdgeKey> = match route.kind {
                RouteKind::Audio => vec![keys::route(*key)],
                RouteKind::Sidechain => (0..FX_SLOTS as u64).map(|k| keys::route(*key) ^ (0x5C00 + k)).collect(),
            };
            for edge in edge_keys {
                if let Some(handle) = gains.handle(edge) {
                    routes.push((keys::route_level(*key), handle, route.send_level));
                }
            }
        }
        routes.sort_by_key(|(key, ..)| *key);
        let summary: Vec<(u64, f32)> = routes.iter().map(|(key, _, base)| (*key, *base)).collect();
        if summary != self.sent_routes || self.targets.is_some() {
            self.sent_routes = summary;
            self.targets = Some(Box::new(AutomationTargets { delays: self.delays.clone(), routes }));
        }
    }

    /// Points each loaded slot at its sidechain port, or at none.
    fn set_sidechain_ports(&self, ports: &[(InsertId, usize, u32)]) {
        for (id, link) in &self.links {
            for (k, loaded) in link.loaded.iter().enumerate() {
                if let Some(loaded) = loaded {
                    let port = ports.iter().find(|(i, slot, _)| i == id && *slot == k).map_or(NO_PORT, |&(.., p)| p);
                    loaded.live.sidechain_port.store(port, Ordering::Relaxed);
                }
            }
        }
    }

    /// Stores every channel's and insert's live settings, creating the shared cells of new ones, and loads the effects
    /// slots now hold into running FX stages.
    fn update_live(&mut self, project: &Project) {
        for (id, channel) in &project.channels {
            let live = self.channels.entry(*id).or_default();
            live.volume.set(channel.volume);
            live.pan.set(channel.pan);
            live.pitch_cents.set(channel.pitch_cents);
            live.muted.store(channel.muted, Ordering::Relaxed);
        }
        self.channels.retain(|id, _| project.channels.contains_key(id));
        let audible = audible_set(project);
        for insert in project.mixer.inserts.values().chain(std::iter::once(&project.mixer.master)) {
            let live = self.strips.entry(insert.id).or_default();
            live.volume_db.set(insert.volume_db);
            live.pan.set(insert.pan);
            live.stereo_sep.set(insert.stereo_sep);
            live.polarity_invert.store(insert.polarity_invert, Ordering::Relaxed);
            live.swap_lr.store(insert.swap_lr, Ordering::Relaxed);
            live.muted.store(insert.muted, Ordering::Relaxed);
            live.armed.store(insert.armed, Ordering::Relaxed);
            live.silenced.store(audible.as_ref().is_some_and(|set| !set.contains(&insert.id)), Ordering::Relaxed);
            live.set_input(insert.input.as_ref().map_or(&[], |port| port.channels.as_slice()));
            let band = |b: &bodo_model::mixer::EqBand| [b.frequency, b.gain_db, b.q];
            live.eq.set([band(&insert.eq.low), band(&insert.eq.mid), band(&insert.eq.high)]);
            self.update_slots(insert);
        }
        self.strips.retain(|id, _| project.mixer.insert(*id).is_some());
    }

    /// Brings `insert`'s loaded effects and their settings up to date, sending changed slots to its running FX stage.
    fn update_slots(&mut self, insert: &MixerInsert) {
        let (rate, max_frames) = (self.rate, self.max_frames);
        let link = self.links.entry(insert.id).or_default();
        for (index, slot) in insert.slots.iter().enumerate() {
            let wanted = native(slot);
            let current = link.loaded.get(index).and_then(Option::as_ref).map(|l| (l.instance, l.info.uid));
            if wanted.map(|(instance, info)| (instance, info.uid)) != current {
                let loaded = wanted.map(|(instance, info)| Loaded { instance, info, live: Arc::new(SlotLive::new(&slot_values(info, slot))) });
                if let Some(cell) = link.loaded.get_mut(index) {
                    *cell = loaded;
                }
                if link.patches.is_some() {
                    let state = link.fresh_state(insert.id, index, rate, max_frames);
                    link.send(Box::new(SlotPatch { index, state }));
                }
            }
            if let (Some(Some(loaded)), Some((_, info))) = (link.loaded.get(index), wanted) {
                let values = slot_values(info, slot);
                loaded.live.enabled.store(slot.enabled, Ordering::Relaxed);
                loaded.live.mix.set(slot.mix.clamp(0.0, 1.0));
                loaded.live.latency.store((info.latency)(&values, rate).min((info.max_latency)(rate)), Ordering::Relaxed);
                for (cell, value) in loaded.live.params.iter().zip(values) {
                    cell.set(value);
                }
            }
        }
    }

    /// The sampler channels and what each one plays, loading samples and rendering their precomputed effects on first
    /// use.
    fn sampler_sources(&mut self, project: &Project) -> HashMap<ChannelId, SamplerSource> {
        let mut sources = HashMap::new();
        let bpm = project.tempo.bpm_at(bodo_kit_core::Tick::ZERO);
        for (id, channel) in &project.channels {
            if let ChannelKind::Sampler(params) = &channel.kind {
                let streamed = match (params.sample, self.streaming && crate::disk_sample::streamable(params)) {
                    (Some(sample), true) => {
                        project.samples.get(&sample).and_then(|reference| self.disk.get(&self.roots.resolve(&reference.path), params))
                    }
                    _ => None,
                };
                let data = match params.sample {
                    _ if streamed.is_some() => SamplerData { sample: None, zones: Arc::from(Vec::new()), disk: streamed },
                    Some(sample) => {
                        let base = if params.resample {
                            self.cache.get(project, sample, &self.roots)
                        } else {
                            self.cache.get_raw(project, sample, &self.roots)
                        };
                        match base {
                            Some(base) => {
                                let markers = if params.load_regions || params.load_slices {
                                    self.cache.markers(project, sample, &self.roots, base.rate)
                                } else {
                                    Default::default()
                                };
                                self.derived.get(&base, &markers, params, bpm)
                            }
                            None => SamplerData::default(),
                        }
                    }
                    None => SamplerData::default(),
                };
                sources.insert(*id, SamplerSource { params: Arc::clone(params), data });
            }
        }
        sources
    }

    /// The audio clip channels, their samples and the time-stretched copies their clips need, rendering any that
    /// are new.
    ///
    /// Allocates; loads samples and renders stretched copies on first use. Complexity: O(n log n) in clips, plus the
    /// rendering.
    fn audio_sources(&mut self, project: &Project) -> HashMap<ChannelId, AudioSource> {
        let mut sources = HashMap::new();
        for (id, channel) in &project.channels {
            if let ChannelKind::AudioClip { sample, .. } = &channel.kind {
                let sample = self.cache.get(project, *sample, &self.roots);
                let stretched = match &sample {
                    Some(data) => stretch_keys(project, *id).into_iter().map(|key| (key, self.stretch.get(data, key))).collect(),
                    None => Vec::new(),
                };
                sources.insert(*id, AudioSource { sample, stretched });
            }
        }
        sources
    }
}

/// The latency of a mix, as delay compensation decides it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MixLatency {
    /// How late the master's output is, in frames.
    pub total: u32,
    /// Each insert's latency, the master included, in mixer order.
    pub inserts: Vec<(InsertId, crate::engine::InsertLatency)>,
}

/// What delay compensation decides for `project`, without loading samples or running anything. `recording` decides
/// the inserts that bypass compensation while recording.
///
/// Allocates. Complexity: O((V + E) log E).
pub fn mix_latency(project: &Project, recording: bool) -> Result<MixLatency, MixError> {
    let shape = topology(project, recording, false, |_| None::<()>);
    let report = compile(shape.desc, 1, None)?.latency;
    let inserts = project
        .mixer
        .order
        .iter()
        .copied()
        .chain(std::iter::once(project.mixer.master.id))
        .filter_map(|id| {
            let input = report.nodes.get(&keys::insert_input(id))?.input;
            let output = report.nodes.get(&keys::insert(id))?.output;
            Some((id, crate::engine::InsertLatency { input, own: output.saturating_sub(input), output }))
        })
        .collect();
    Ok(MixLatency { total: report.total, inserts })
}

/// The inserts that stay audible while some are soloed: the soloed ones, everything that feeds them and everything
/// they feed, over audio routes and the master; `None` when nothing is soloed.
///
/// Complexity: O(V + E) in inserts and routes.
pub fn audible_set(project: &Project) -> Option<HashSet<InsertId>> {
    let master = project.mixer.master.id;
    let soloed: Vec<InsertId> =
        project.mixer.inserts.values().chain(std::iter::once(&project.mixer.master)).filter(|i| i.solo).map(|i| i.id).collect();
    if soloed.is_empty() {
        return None;
    }
    let mut forward: HashMap<InsertId, Vec<InsertId>> = HashMap::new();
    let mut backward: HashMap<InsertId, Vec<InsertId>> = HashMap::new();
    let mut link = |from: InsertId, to: InsertId| {
        forward.entry(from).or_default().push(to);
        backward.entry(to).or_default().push(from);
    };
    for insert in project.mixer.inserts.values() {
        if insert.to_master {
            link(insert.id, master);
        }
    }
    for (key, route) in &project.mixer.routes {
        if let (RouteSource::Insert(from), RouteKind::Audio) = (key.from, route.kind) {
            link(from, key.to);
        }
    }
    let mut audible: HashSet<InsertId> = HashSet::new();
    for edges in [&forward, &backward] {
        let mut stack = soloed.clone();
        let mut seen: HashSet<InsertId> = HashSet::new();
        while let Some(id) = stack.pop() {
            if seen.insert(id) {
                stack.extend(edges.get(&id).into_iter().flatten().copied());
            }
        }
        audible.extend(seen);
    }
    Some(audible)
}
