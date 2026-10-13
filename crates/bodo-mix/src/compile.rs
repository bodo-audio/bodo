//! Turning a project snapshot into the mix graph (project.md section 9.4).
//!
//! Sampler and audio clip channels feed their target insert, or the master; inserts feed each other through audio
//! routes and the master through `to_master`. Most edits never touch the graph: faders, pans, mutes and send levels go to the
//! running processors through shared atomics and edge gains. Only a change of topology, or of a sampler's settings
//! or sample, compiles a new plan, and processors whose settings didn't change carry over into it.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use bodo_kit_audio::AudioData;
use bodo_kit_core::{MAX_BLOCK_FRAMES, SampleRate};
use bodo_kit_graph::{EdgeDesc, EdgeGains, GraphDesc, LatencyReport, Layout, NodeDesc, Plan, compile};
use bodo_model::channel::{ChannelKind, SamplerParams};
use bodo_model::mixer::{RouteKind, RouteSource};
use bodo_model::{ChannelId, InsertId, Project, SampleRoots};

use crate::MixError;
use crate::audio_clips::stretch_keys;
use crate::keys;
use crate::live::{ChannelLive, StripLive};
use crate::nodes::{AudioClipNode, MixNode, SamplerNode, StripNode};
use crate::record::CaptureLinks;
use crate::sample::SampleCache;
use crate::stretch::StretchCache;

/// What a sampler node was built from, to tell whether it must be rebuilt.
#[derive(Clone, Debug)]
struct SamplerSource {
    params: Arc<SamplerParams>,
    sample: Option<Arc<AudioData>>,
}

impl SamplerSource {
    fn same_as(&self, other: &SamplerSource) -> bool {
        let same_sample = match (&self.sample, &other.sample) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        (Arc::ptr_eq(&self.params, &other.params) || self.params == other.params) && same_sample
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

/// Keeps the graph and the live parameters in step with the project.
#[derive(Debug)]
pub(crate) struct Compiler {
    roots: SampleRoots,
    max_frames: usize,
    cache: SampleCache,
    layout: Option<Layout>,
    samplers: HashMap<ChannelId, SamplerSource>,
    audio: HashMap<ChannelId, AudioSource>,
    stretch: StretchCache,
    channels: HashMap<ChannelId, Arc<ChannelLive>>,
    strips: HashMap<InsertId, Arc<StripLive>>,
    /// Inserts the running plan has processors for.
    built_strips: HashSet<InsertId>,
    /// The control side's links to each strip's recording capture.
    captures: CaptureLinks,
    signature: Vec<u64>,
    gains: Option<EdgeGains>,
    latency: LatencyReport,
    rate: SampleRate,
}

impl Compiler {
    pub(crate) fn new(roots: SampleRoots, max_frames: usize, rate: SampleRate) -> Self {
        Self {
            roots,
            max_frames: max_frames.clamp(1, MAX_BLOCK_FRAMES),
            cache: SampleCache::new(),
            layout: None,
            samplers: HashMap::new(),
            audio: HashMap::new(),
            stretch: StretchCache::default(),
            channels: HashMap::new(),
            strips: HashMap::new(),
            built_strips: HashSet::new(),
            captures: CaptureLinks::default(),
            signature: Vec::new(),
            gains: None,
            latency: LatencyReport::default(),
            rate,
        }
    }

    /// Resolves sample paths through `roots` from the next update on, and tries missing samples again.
    pub(crate) fn set_roots(&mut self, roots: SampleRoots) {
        self.roots = roots;
        self.cache.retry_failed();
    }

    /// The links to every strip's recording capture.
    pub(crate) fn captures_mut(&mut self) -> &mut CaptureLinks {
        &mut self.captures
    }

    pub(crate) fn latency(&self) -> &LatencyReport {
        &self.latency
    }

    /// Brings everything up to date with `project`. Returns a new plan when the graph changed.
    ///
    /// Allocates and may load samples. Complexity: O(V + E) plus sample loading and, on a recompile, the plan's
    /// buffers.
    pub(crate) fn update(&mut self, project: &Project) -> Result<Option<Plan<MixNode>>, MixError> {
        self.update_live(project);
        let sources = self.sampler_sources(project);
        let audio = self.audio_sources(project);
        let signature = signature(project, &sources, &audio);
        if signature == self.signature
            && let Some(gains) = &self.gains
        {
            for (key, route) in &project.mixer.routes {
                if route.kind == RouteKind::Audio {
                    gains.set(route_key(key.from, key.to), route.send_level);
                }
            }
            return Ok(None);
        }

        let master = project.mixer.master.id;
        let mut desc = GraphDesc { nodes: Vec::new(), edges: Vec::new(), output: Some(keys::insert(master)) };
        let pan_law = project.settings.pan_law;
        for id in &project.rack_order {
            let live = || self.channels.get(id).cloned().unwrap_or_default();
            let processor = if let Some(source) = sources.get(id) {
                let fresh = self.samplers.get(id).is_none_or(|old| !old.same_as(source));
                fresh.then(|| {
                    MixNode::Sampler(Box::new(SamplerNode::new(
                        keys::channel(*id),
                        source.sample.clone(),
                        &source.params,
                        live(),
                        pan_law,
                        self.rate,
                    )))
                })
            } else if let Some(source) = audio.get(id) {
                let fresh = self.audio.get(id).is_none_or(|old| !old.same_as(source));
                fresh.then(|| {
                    MixNode::AudioClip(Box::new(AudioClipNode::new(
                        keys::channel(*id),
                        source.sample.clone(),
                        source.stretched.clone(),
                        live(),
                        pan_law,
                    )))
                })
            } else {
                continue;
            };
            desc.nodes.push(NodeDesc { key: keys::channel(*id), channels: 2, ports: 0, latency: 0, processor });
            let target = project.channels.get(id).and_then(|c| c.target).filter(|t| project.mixer.insert(*t).is_some()).unwrap_or(master);
            desc.edges.push(EdgeDesc {
                key: keys::edge(keys::channel(*id), keys::insert(target)) ^ 1,
                from: keys::channel(*id),
                to: keys::insert(target),
                port: 0,
                port_offset: 0,
                gain: 1.0,
            });
        }
        let inserts: Vec<InsertId> = project.mixer.order.iter().copied().chain(std::iter::once(master)).collect();
        for id in &inserts {
            let fresh = !self.built_strips.contains(id);
            let processor = fresh.then(|| {
                let strip = StripNode::new(keys::insert(*id), self.strips.get(id).cloned().unwrap_or_default());
                MixNode::Strip(strip.with_capture(self.captures.link(*id)))
            });
            desc.nodes.push(NodeDesc { key: keys::insert(*id), channels: 2, ports: 1, latency: 0, processor });
            let insert = project.mixer.insert(*id);
            if *id != master && insert.is_some_and(|i| i.to_master) {
                desc.edges.push(EdgeDesc {
                    key: keys::edge(keys::insert(*id), keys::insert(master)) ^ 3,
                    from: keys::insert(*id),
                    to: keys::insert(master),
                    port: 0,
                    port_offset: 0,
                    gain: 1.0,
                });
            }
        }
        for (key, route) in &project.mixer.routes {
            // Sidechain routes feed plugin sidechains, which arrive with FX slots in M4.
            if route.kind != RouteKind::Audio {
                continue;
            }
            let from = match key.from {
                RouteSource::Channel(channel) if sources.contains_key(&channel) => keys::channel(channel),
                RouteSource::Channel(_) => continue,
                RouteSource::Insert(insert) => keys::insert(insert),
            };
            desc.edges.push(EdgeDesc {
                key: route_key(key.from, key.to),
                from,
                to: keys::insert(key.to),
                port: 0,
                port_offset: 0,
                gain: route.send_level,
            });
        }

        let compiled = compile(desc, self.max_frames, self.layout.as_ref())?;
        self.layout = Some(compiled.layout);
        self.gains = Some(compiled.gains);
        self.latency = compiled.latency;
        self.samplers = sources;
        self.audio = audio;
        self.built_strips = inserts.into_iter().collect();
        self.captures.retain(|id| self.built_strips.contains(id));
        self.signature = signature;
        self.stretch.trim();
        self.cache.trim();
        Ok(Some(compiled.plan))
    }

    /// Stores every channel's and insert's live settings, creating the shared cells of new ones.
    fn update_live(&mut self, project: &Project) {
        for (id, channel) in &project.channels {
            let live = self.channels.entry(*id).or_default();
            live.volume.set(channel.volume);
            live.pan.set(channel.pan);
            live.pitch_cents.set(channel.pitch_cents);
            live.muted.store(channel.muted, Ordering::Relaxed);
        }
        self.channels.retain(|id, _| project.channels.contains_key(id));
        for insert in project.mixer.inserts.values().chain(std::iter::once(&project.mixer.master)) {
            let live = self.strips.entry(insert.id).or_default();
            live.volume_db.set(insert.volume_db);
            live.pan.set(insert.pan);
            live.stereo_sep.set(insert.stereo_sep);
            live.polarity_invert.store(insert.polarity_invert, Ordering::Relaxed);
            live.swap_lr.store(insert.swap_lr, Ordering::Relaxed);
            live.muted.store(insert.muted, Ordering::Relaxed);
            live.armed.store(insert.armed, Ordering::Relaxed);
            live.set_input(insert.input.as_ref().map_or(&[], |port| port.channels.as_slice()));
        }
        self.strips.retain(|id, _| project.mixer.insert(*id).is_some());
    }

    /// The sampler channels and what each one plays, loading samples on first use.
    fn sampler_sources(&mut self, project: &Project) -> HashMap<ChannelId, SamplerSource> {
        let mut sources = HashMap::new();
        for (id, channel) in &project.channels {
            if let ChannelKind::Sampler(params) = &channel.kind {
                let sample = params.sample.and_then(|sample| self.cache.get(project, sample, &self.roots));
                sources.insert(*id, SamplerSource { params: Arc::clone(params), sample });
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

/// The key of the edge an audio route compiles to.
fn route_key(from: RouteSource, to: InsertId) -> u64 {
    let from = match from {
        RouteSource::Channel(channel) => keys::channel(channel),
        RouteSource::Insert(insert) => keys::insert(insert),
    };
    keys::edge(from, keys::insert(to)) ^ 2
}

/// Everything that decides the graph's shape and its processors, flattened. Two equal signatures compile to the same
/// graph, apart from gains.
fn signature(project: &Project, sources: &HashMap<ChannelId, SamplerSource>, audio: &HashMap<ChannelId, AudioSource>) -> Vec<u64> {
    let master = project.mixer.master.id;
    let mut out = vec![project.settings.pan_law as u64, keys::insert(master)];
    for id in &project.rack_order {
        if let Some(source) = sources.get(id) {
            out.push(keys::channel(*id));
            out.push(Arc::as_ptr(&source.params) as usize as u64);
            out.push(source.sample.as_ref().map_or(0, |s| Arc::as_ptr(s) as usize as u64));
            out.push(project.channels.get(id).and_then(|c| c.target).map_or(0, keys::insert));
        }
        if let Some(source) = audio.get(id) {
            out.push(keys::channel(*id) ^ 0xA0D1);
            out.push(source.sample.as_ref().map_or(0, |s| Arc::as_ptr(s) as usize as u64));
            out.extend(source.stretched.iter().flat_map(|(key, data)| [u64::from(*key), Arc::as_ptr(data) as usize as u64]));
            out.push(project.channels.get(id).and_then(|c| c.target).map_or(0, keys::insert));
        }
    }
    for id in &project.mixer.order {
        out.push(keys::insert(*id));
        out.push(u64::from(project.mixer.insert(*id).is_some_and(|i| i.to_master)));
    }
    for (key, route) in &project.mixer.routes {
        if route.kind == RouteKind::Audio {
            out.push(route_key(key.from, key.to));
        }
    }
    out
}
