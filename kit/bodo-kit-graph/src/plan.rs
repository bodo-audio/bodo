//! Describing a processing graph and compiling it into a [`Plan`] the executor can run.
//!
//! A [`GraphDesc`] lists nodes and edges by stable keys. [`compile`] checks it, orders it with Kahn's algorithm,
//! computes plugin delay compensation, and allocates every buffer and delay line the plan needs, all on the control
//! thread. The result is handed to the audio thread through `bodo_kit_core::handoff`.
//!
//! **Delay compensation** (project.md section 9.4) takes one pass in topological order. `L(n)` is the latency a node
//! reports. An edge enters its target at a port that is used `offset` frames into the node's own latency; that is
//! zero for the main input and the latency of the earlier slots for a sidechain input. Then:
//!
//! - `in(n) = max(out(src) - offset)` over the edges into `n`, and at least zero;
//! - `out(n) = in(n) + L(n)`;
//! - each edge is delayed by `in(n) + offset - out(src)`, so every signal reaches its port aligned.
//!
//! **State that outlives a plan:** a node described without a processor takes over the processor of the node with
//! the same key in the previous plan, and an edge whose delay didn't change keeps its delay-line contents. Both
//! moves happen on the audio thread without allocating.
//!
//! **Crossfading topology changes** (ADR 0003): edge gains never jump. With [`GraphDesc::gain_ramp`] set, a gain moves
//! by at most its whole range over that many frames, so a send-level change glides, an edge that is new in a
//! recompiled plan fades in from silence, and an edge described with gain zero fades out. A caller that keeps a
//! removed edge for one more plan at gain zero removes it without a click.
//!
//! **Buffer reuse:** a node that is the only reader of one of its inputs takes over that input's output buffer: it
//! sums its inputs into its own ports before it writes its output, so the input's contents are no longer needed. The
//! output node and taps never hand their buffers on, so the caller can read them after a block.
//!
//! **Timing:** the executor adds up the time each node spends processing ([`NodeTimes`]), for CPU meters.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use atomic_refcell::AtomicRefCell;
use bodo_kit_dsp::AudioBuffer;
use bodo_kit_dsp::delay::DelayLine;
use crossbeam_queue::ArrayQueue;

/// A stable identifier for a node, chosen by the caller.
pub type NodeKey = u64;
/// A stable identifier for an edge, chosen by the caller.
pub type EdgeKey = u64;

/// The most channels a node may have.
pub const MAX_CHANNELS: usize = 64;

/// A node to compile.
#[derive(Debug)]
pub struct NodeDesc<P> {
    /// The node's key, unique in the graph.
    pub key: NodeKey,
    /// Channels of the node's output, and of each of its input ports.
    pub channels: usize,
    /// How many input ports the node has. Port 0 is the main input.
    pub ports: usize,
    /// The latency the node reports, in frames.
    pub latency: u32,
    /// A new processor, or `None` to keep the one the node had in the previous plan. A node with no processor at all
    /// passes its port-0 input through.
    pub processor: Option<P>,
}

/// An edge to compile: audio from the output of `from` into a port of `to`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeDesc {
    /// The edge's key, unique in the graph.
    pub key: EdgeKey,
    /// The source node.
    pub from: NodeKey,
    /// The target node.
    pub to: NodeKey,
    /// The target's input port.
    pub port: usize,
    /// How far into the target's own latency this port is used, in frames.
    pub port_offset: u32,
    /// The gain applied to the signal. It can be changed later through [`EdgeGains`].
    pub gain: f32,
    /// Whether delay compensation delays this edge. An edge that isn't compensated reaches its port undelayed, so
    /// early, which low-latency monitoring wants; delay compensation leaves it out when it lines up the others.
    pub compensate: bool,
}

/// A graph to compile.
#[derive(Debug)]
pub struct GraphDesc<P> {
    /// The nodes.
    pub nodes: Vec<NodeDesc<P>>,
    /// The edges.
    pub edges: Vec<EdgeDesc>,
    /// The node whose output the caller reads after each block, such as the master.
    pub output: Option<NodeKey>,
    /// More nodes whose outputs the caller reads after each block, by their index in this list
    /// ([`crate::GraphRunner::read_tap`]).
    pub taps: Vec<NodeKey>,
    /// Frames over which an edge's gain may move across its whole range, 0 to 1; zero lets it move in one block.
    /// Edges that are new in a recompiled plan start silent and fade in at this rate.
    pub gain_ramp: u32,
}

impl<P> Default for GraphDesc<P> {
    fn default() -> Self {
        Self { nodes: Vec::new(), edges: Vec::new(), output: None, taps: Vec::new(), gain_ramp: 0 }
    }
}

/// Why a graph could not be compiled.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CompileError {
    /// Two nodes share a key.
    #[error("node key {0} is used twice")]
    DuplicateNode(NodeKey),
    /// Two edges share a key.
    #[error("edge key {0} is used twice")]
    DuplicateEdge(EdgeKey),
    /// An edge, or the output, names a node that isn't in the graph.
    #[error("node {node} is not in the graph")]
    UnknownNode {
        /// The missing node.
        node: NodeKey,
    },
    /// An edge targets a port its node doesn't have.
    #[error("edge {edge} targets port {port}, which its node doesn't have")]
    NoSuchPort {
        /// The edge.
        edge: EdgeKey,
        /// The port.
        port: usize,
    },
    /// A node has no channels, or more than [`MAX_CHANNELS`].
    #[error("node {0} must have between 1 and 64 channels")]
    BadChannelCount(NodeKey),
    /// The edges form a cycle. The nodes are listed in cycle order; the last has an edge back to the first.
    #[error("the graph has a cycle through {} nodes", .path.len())]
    Cycle {
        /// The cycle.
        path: Vec<NodeKey>,
    },
}

/// The latency of a node, as delay compensation computed it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct NodeLatency {
    /// How late the node's input is, relative to a source with no latency.
    pub input: u32,
    /// How late the node's output is: its input latency plus its own.
    pub output: u32,
}

/// What delay compensation decided, for display and for agents.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LatencyReport {
    /// Latency per node.
    pub nodes: HashMap<NodeKey, NodeLatency>,
    /// Compensation delay per edge, in frames.
    pub edges: HashMap<EdgeKey, u32>,
    /// The output latency of the output node, or zero without one.
    pub total: u32,
}

/// Where each node and edge sits in a compiled plan, kept by the control thread to compile the next plan.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Layout {
    nodes: HashMap<NodeKey, u32>,
    edges: HashMap<EdgeKey, EdgeLayout>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EdgeLayout {
    index: u32,
    delay: u32,
    channels: usize,
}

/// Live gains of a plan's edges, shared with the control thread so it can change them without recompiling.
#[derive(Clone, Debug)]
pub struct EdgeGains {
    gains: Arc<[AtomicU32]>,
    slots: HashMap<EdgeKey, usize>,
}

impl EdgeGains {
    /// Sets the gain of edge `key`. The executor ramps to it over the next block. Returns whether the edge exists.
    pub fn set(&self, key: EdgeKey, gain: f32) -> bool {
        match self.slots.get(&key).and_then(|&slot| self.gains.get(slot)) {
            Some(cell) => {
                cell.store(gain.to_bits(), Ordering::Relaxed);
                true
            }
            None => false,
        }
    }

    /// The gain of edge `key`.
    pub fn get(&self, key: EdgeKey) -> Option<f32> {
        self.slots.get(&key).and_then(|&slot| self.gains.get(slot)).map(|cell| f32::from_bits(cell.load(Ordering::Relaxed)))
    }
}

/// The processing time each node of a plan has used, added up since the plan started.
#[derive(Clone, Debug)]
pub struct NodeTimes {
    nanos: Arc<[AtomicU64]>,
    slots: HashMap<NodeKey, usize>,
}

impl NodeTimes {
    /// Nanoseconds node `key` has spent processing since its plan started running, or `None` when the plan has no
    /// such node. The count only grows while the plan runs; the next plan starts again from zero.
    pub fn nanos(&self, key: NodeKey) -> Option<u64> {
        self.slots.get(&key).and_then(|&slot| self.nanos.get(slot)).map(|cell| cell.load(Ordering::Relaxed))
    }
}

/// The result of [`compile`].
#[derive(Debug)]
pub struct Compiled<P> {
    /// The plan, ready to send to the audio thread.
    pub plan: Plan<P>,
    /// Where things sit, for compiling the next plan.
    pub layout: Layout,
    /// Delay compensation results.
    pub latency: LatencyReport,
    /// Handles to change edge gains while the plan runs.
    pub gains: EdgeGains,
    /// How long each node takes to process.
    pub times: NodeTimes,
}

/// A processor slot that travels from plan to plan.
#[derive(Debug)]
pub(crate) struct Slot<P> {
    pub(crate) processor: Option<P>,
    /// Set when the processor panicked; it is skipped from then on.
    pub(crate) poisoned: bool,
}

/// The state an edge keeps in its target node.
#[derive(Debug)]
pub(crate) struct EdgeState {
    /// One delay line per source channel; empty when the edge has no delay.
    pub(crate) lines: Vec<DelayLine>,
    /// Where a delayed edge's signal is prepared; `None` without delay.
    pub(crate) scratch: Option<AudioBuffer>,
    /// The gain applied in the last block, ramped from in the next.
    pub(crate) gain: f32,
}

/// A node's input side: its summed ports and its incoming edges.
#[derive(Debug)]
pub(crate) struct NodeInputs {
    pub(crate) ports: Vec<AudioBuffer>,
    pub(crate) edges: Vec<EdgeState>,
}

#[derive(Debug)]
pub(crate) struct PlanNode<P> {
    pub(crate) slot: AtomicRefCell<Slot<P>>,
    /// The index of the node's output buffer in [`Plan::buffers`], which it may have taken over from an input.
    pub(crate) output: u32,
    pub(crate) inputs: AtomicRefCell<NodeInputs>,
    /// This node's incoming edges, `edges[in_start..in_end]`.
    pub(crate) in_start: u32,
    pub(crate) in_end: u32,
    /// The targets of this node's outgoing edges, `dependents[out_start..out_end]`.
    pub(crate) out_start: u32,
    pub(crate) out_end: u32,
    pub(crate) indegree: u32,
    pub(crate) pending: AtomicU32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PlanEdge {
    pub(crate) from: u32,
    pub(crate) to: u32,
    pub(crate) port: u32,
    pub(crate) delay: u32,
    pub(crate) gain_slot: u32,
}

/// A compiled graph: everything the executor needs, allocated up front.
#[derive(Debug)]
pub struct Plan<P> {
    pub(crate) nodes: Vec<PlanNode<P>>,
    /// Output buffers, handed on along chains of single readers.
    pub(crate) buffers: Vec<AtomicRefCell<AudioBuffer>>,
    /// Edges grouped by target node, sorted by port and key within a node, so inputs always sum in the same order.
    pub(crate) edges: Vec<PlanEdge>,
    pub(crate) dependents: Vec<u32>,
    pub(crate) sources: Vec<u32>,
    pub(crate) gains: Arc<[AtomicU32]>,
    pub(crate) ready: ArrayQueue<u32>,
    pub(crate) output: Option<u32>,
    /// Nodes the caller reads after a block, by tap index.
    pub(crate) taps: Vec<u32>,
    /// Nanoseconds each node has spent processing.
    pub(crate) times: Arc<[AtomicU64]>,
    /// Frames a full gain swing takes; zero for one block.
    pub(crate) gain_ramp: u32,
    pub(crate) adopt_nodes: Vec<(u32, u32)>,
    pub(crate) adopt_edges: Vec<(u32, u32)>,
    pub(crate) max_frames: usize,
}

impl<P> Plan<P> {
    /// A plan with no nodes, which renders nothing.
    ///
    /// Allocates.
    pub fn empty(max_frames: usize) -> Self {
        Self {
            nodes: Vec::new(),
            buffers: Vec::new(),
            edges: Vec::new(),
            dependents: Vec::new(),
            sources: Vec::new(),
            gains: Arc::from(Vec::new()),
            ready: ArrayQueue::new(1),
            output: None,
            taps: Vec::new(),
            times: Arc::from(Vec::new()),
            gain_ramp: 0,
            adopt_nodes: Vec::new(),
            adopt_edges: Vec::new(),
            max_frames,
        }
    }

    /// The number of nodes.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// The largest block this plan can run.
    pub fn max_frames(&self) -> usize {
        self.max_frames
    }

    /// How many output buffers the plan allocated: fewer than its nodes when nodes hand their buffers on.
    pub fn buffer_count(&self) -> usize {
        self.buffers.len()
    }

    /// Takes over processors and delay-line contents from `previous`, as the compiler arranged. Called on the audio
    /// thread when this plan replaces `previous`.
    ///
    /// RT-safe. Complexity: O(k) in the number of adopted nodes and edges.
    pub(crate) fn adopt_from(&mut self, previous: &mut Plan<P>) {
        for &(old, new) in &self.adopt_nodes {
            if let (Some(old), Some(new)) = (previous.nodes.get_mut(old as usize), self.nodes.get_mut(new as usize)) {
                core::mem::swap(old.slot.get_mut(), new.slot.get_mut());
            }
        }
        for i in 0..self.adopt_edges.len() {
            let Some(&(old, new)) = self.adopt_edges.get(i) else { break };
            let (Some(old_state), Some(new_state)) = (previous.edge_state(old), self.edge_state(new)) else { continue };
            core::mem::swap(&mut old_state.lines, &mut new_state.lines);
            new_state.gain = old_state.gain;
        }
    }

    /// The state of edge `index`, which lives in its target node, mutably.
    fn edge_state(&mut self, index: u32) -> Option<&mut EdgeState> {
        let edge = self.edges.get(index as usize)?;
        let node = self.nodes.get_mut(edge.to as usize)?;
        let local = index.checked_sub(node.in_start)? as usize;
        node.inputs.get_mut().edges.get_mut(local)
    }
}

/// Compiles `desc` into a plan for blocks of up to `max_frames` frames. Pass the layout of the plan currently running,
/// so nodes and edges can keep their state.
///
/// Allocates. Complexity: O((V + E) log E) for sorting the edges, plus the buffer allocations.
pub fn compile<P>(desc: GraphDesc<P>, max_frames: usize, previous: Option<&Layout>) -> Result<Compiled<P>, CompileError> {
    let GraphDesc { nodes, edges, output, taps, gain_ramp } = desc;
    let mut index: HashMap<NodeKey, usize> = HashMap::with_capacity(nodes.len());
    for (i, node) in nodes.iter().enumerate() {
        if index.insert(node.key, i).is_some() {
            return Err(CompileError::DuplicateNode(node.key));
        }
        if node.channels == 0 || node.channels > MAX_CHANNELS {
            return Err(CompileError::BadChannelCount(node.key));
        }
    }
    let output = match output {
        Some(key) => Some(*index.get(&key).ok_or(CompileError::UnknownNode { node: key })?),
        None => None,
    };
    let taps = taps.iter().map(|key| index.get(key).copied().ok_or(CompileError::UnknownNode { node: *key })).collect::<Result<Vec<_>, _>>()?;

    // Resolve and sort the edges: by target, then port, then key. That fixes the summing order.
    let mut seen_edges = HashMap::with_capacity(edges.len());
    let mut resolved = Vec::with_capacity(edges.len());
    for edge in &edges {
        if seen_edges.insert(edge.key, ()).is_some() {
            return Err(CompileError::DuplicateEdge(edge.key));
        }
        let from = *index.get(&edge.from).ok_or(CompileError::UnknownNode { node: edge.from })?;
        let to = *index.get(&edge.to).ok_or(CompileError::UnknownNode { node: edge.to })?;
        if edge.port >= nodes[to].ports {
            return Err(CompileError::NoSuchPort { edge: edge.key, port: edge.port });
        }
        resolved.push((to, edge.port, edge.key, from, *edge));
    }
    resolved.sort_unstable_by_key(|&(to, port, key, _, _)| (to, port, key));

    let order = topological_order(nodes.len(), &resolved, &nodes)?;

    // Delay compensation in topological order.
    let mut in_edges_of: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    for (e, &(to, ..)) in resolved.iter().enumerate() {
        in_edges_of[to].push(e);
    }
    let mut latency = vec![NodeLatency::default(); nodes.len()];
    for &n in &order {
        let input = in_edges_of[n]
            .iter()
            .filter(|&&e| resolved[e].4.compensate)
            .map(|&e| {
                let (_, _, _, from, edge) = resolved[e];
                i64::from(latency[from].output) - i64::from(edge.port_offset)
            })
            .max()
            .unwrap_or(0)
            .max(0) as u32;
        latency[n] = NodeLatency { input, output: input.saturating_add(nodes[n].latency) };
    }
    let delays: Vec<u32> = resolved
        .iter()
        .map(|&(to, _, _, from, edge)| {
            if !edge.compensate {
                return 0;
            }
            let delay = i64::from(latency[to].input) + i64::from(edge.port_offset) - i64::from(latency[from].output);
            delay.max(0) as u32
        })
        .collect();

    // Build the plan nodes; edges are already grouped by target.
    let gains: Arc<[AtomicU32]> = resolved.iter().map(|&(.., edge)| AtomicU32::new(edge.gain.to_bits())).collect();
    // In a recompile with gain ramps, an edge that isn't carried over starts silent and fades in.
    let fade_in = gain_ramp > 0 && previous.is_some();
    let mut out_degree = vec![0u32; nodes.len()];
    for &(_, _, _, from, _) in &resolved {
        out_degree[from] += 1;
    }
    let mut out_start = vec![0u32; nodes.len() + 1];
    for n in 0..nodes.len() {
        out_start[n + 1] = out_start[n] + out_degree[n];
    }
    let mut dependents = vec![0u32; resolved.len()];
    let mut fill = out_start.clone();
    for &(to, _, _, from, _) in &resolved {
        dependents[fill[from] as usize] = to as u32;
        fill[from] += 1;
    }

    let mut plan_edges = Vec::with_capacity(resolved.len());
    let mut edge_report = HashMap::with_capacity(resolved.len());
    let mut layout = Layout::default();
    let mut adopt_edges = Vec::new();
    let mut slots = HashMap::with_capacity(resolved.len());
    for (e, &(to, port, key, from, _)) in resolved.iter().enumerate() {
        let delay = delays[e];
        let channels = nodes[from].channels;
        plan_edges.push(PlanEdge { from: from as u32, to: to as u32, port: port as u32, delay, gain_slot: e as u32 });
        edge_report.insert(key, delay);
        slots.insert(key, e);
        layout.edges.insert(key, EdgeLayout { index: e as u32, delay, channels });
        if let Some(old) = previous.and_then(|p| p.edges.get(&key))
            && old.delay == delay
            && old.channels == channels
        {
            adopt_edges.push((old.index, e as u32));
        }
    }
    let adopted_edges: HashSet<u32> = adopt_edges.iter().map(|&(_, new)| new).collect();

    // Output buffers: a node takes over the buffer of its first input whose only reader it is, unless the caller
    // reads that input after the block. Topological order guarantees the input has its buffer by then.
    let mut readers = vec![0u32; nodes.len()];
    for &(_, _, _, from, _) in &resolved {
        readers[from] += 1;
    }
    let read_by_caller = |n: usize| output == Some(n) || taps.contains(&n);
    let mut buffer_of = vec![u32::MAX; nodes.len()];
    let mut buffer_channels: Vec<usize> = Vec::new();
    for &n in &order {
        let inherited = in_edges_of[n]
            .iter()
            .map(|&e| resolved[e].3)
            .find(|&from| readers[from] == 1 && !read_by_caller(from) && nodes[from].channels == nodes[n].channels && buffer_of[from] != u32::MAX);
        buffer_of[n] = match inherited {
            Some(from) => buffer_of[from],
            None => {
                buffer_channels.push(nodes[n].channels);
                (buffer_channels.len() - 1) as u32
            }
        };
    }

    let channels: Vec<usize> = nodes.iter().map(|node| node.channels).collect();
    let mut plan_nodes = Vec::with_capacity(nodes.len());
    let mut adopt_nodes = Vec::new();
    let mut node_report = HashMap::with_capacity(nodes.len());
    let mut sources = Vec::new();
    let mut in_cursor = 0u32;
    for (n, node) in nodes.into_iter().enumerate() {
        let in_count = in_edges_of[n].len() as u32;
        if in_count == 0 {
            sources.push(n as u32);
        }
        let edge_states = in_edges_of[n]
            .iter()
            .map(|&e| {
                let delay = delays[e] as usize;
                let (.., from, edge) = resolved[e];
                let source_channels = channels[from];
                let gain = if fade_in && !adopted_edges.contains(&(e as u32)) { 0.0 } else { edge.gain };
                if delay == 0 {
                    EdgeState { lines: Vec::new(), scratch: None, gain }
                } else {
                    EdgeState {
                        lines: (0..source_channels).map(|_| DelayLine::new(delay)).collect(),
                        scratch: Some(AudioBuffer::new(source_channels, max_frames)),
                        gain,
                    }
                }
            })
            .collect();
        if node.processor.is_none()
            && let Some(&old) = previous.and_then(|p| p.nodes.get(&node.key))
        {
            adopt_nodes.push((old, n as u32));
        }
        layout.nodes.insert(node.key, n as u32);
        node_report.insert(node.key, latency[n]);
        plan_nodes.push(PlanNode {
            slot: AtomicRefCell::new(Slot { processor: node.processor, poisoned: false }),
            output: buffer_of[n],
            inputs: AtomicRefCell::new(NodeInputs {
                ports: (0..node.ports).map(|_| AudioBuffer::new(node.channels, max_frames)).collect(),
                edges: edge_states,
            }),
            in_start: in_cursor,
            in_end: in_cursor + in_count,
            out_start: out_start[n],
            out_end: out_start[n + 1],
            indegree: in_count,
            pending: AtomicU32::new(in_count),
        });
        in_cursor += in_count;
    }

    let total = output.map_or(0, |o| latency[o].output);
    let node_count = plan_nodes.len();
    let times: Arc<[AtomicU64]> = (0..node_count).map(|_| AtomicU64::new(0)).collect();
    let time_slots = layout.nodes.iter().map(|(&key, &n)| (key, n as usize)).collect();
    let buffers = buffer_channels.iter().map(|&c| AtomicRefCell::new(AudioBuffer::new(c, max_frames))).collect();
    Ok(Compiled {
        plan: Plan {
            nodes: plan_nodes,
            buffers,
            edges: plan_edges,
            dependents,
            sources,
            gains: Arc::clone(&gains),
            ready: ArrayQueue::new(node_count.max(1)),
            output: output.map(|o| o as u32),
            taps: taps.iter().map(|&t| t as u32).collect(),
            times: Arc::clone(&times),
            gain_ramp,
            adopt_nodes,
            adopt_edges,
            max_frames,
        },
        layout,
        latency: LatencyReport { nodes: node_report, edges: edge_report, total },
        gains: EdgeGains { gains, slots },
        times: NodeTimes { nanos: times, slots: time_slots },
    })
}

/// Kahn's algorithm. Nodes are released in index order among equals, so the order is deterministic.
fn topological_order<P>(
    count: usize,
    resolved: &[(usize, usize, EdgeKey, usize, EdgeDesc)],
    nodes: &[NodeDesc<P>],
) -> Result<Vec<usize>, CompileError> {
    let mut indegree = vec![0usize; count];
    let mut successors: Vec<Vec<usize>> = vec![Vec::new(); count];
    for &(to, _, _, from, _) in resolved {
        indegree[to] += 1;
        successors[from].push(to);
    }
    let mut queue: VecDeque<usize> = (0..count).filter(|&n| indegree[n] == 0).collect();
    let mut order = Vec::with_capacity(count);
    while let Some(n) = queue.pop_front() {
        order.push(n);
        for &s in &successors[n] {
            indegree[s] -= 1;
            if indegree[s] == 0 {
                queue.push_back(s);
            }
        }
    }
    if order.len() == count {
        return Ok(order);
    }
    let path = find_cycle(&successors, &indegree).into_iter().map(|n| nodes[n].key).collect();
    Err(CompileError::Cycle { path })
}

/// Finds one cycle among the nodes Kahn's algorithm couldn't release (those with indegree left).
fn find_cycle(successors: &[Vec<usize>], remaining: &[usize]) -> Vec<usize> {
    // Every unreleased node has an unreleased predecessor, so walking predecessors backwards must repeat a node.
    let mut predecessor = vec![usize::MAX; successors.len()];
    for (from, targets) in successors.iter().enumerate() {
        if remaining[from] == 0 {
            continue;
        }
        for &to in targets {
            if remaining[to] > 0 {
                predecessor[to] = from;
            }
        }
    }
    let Some(start) = (0..successors.len()).find(|&n| remaining[n] > 0) else { return Vec::new() };
    let mut seen = vec![false; successors.len()];
    let mut current = start;
    while !seen[current] && predecessor[current] != usize::MAX {
        seen[current] = true;
        current = predecessor[current];
    }
    // `current` is on the cycle; collect it by walking predecessors once around, then put it in edge order.
    let mut cycle = vec![current];
    let mut node = predecessor[current];
    while node != current && node != usize::MAX {
        cycle.push(node);
        node = predecessor[node];
    }
    cycle.reverse();
    cycle
}
