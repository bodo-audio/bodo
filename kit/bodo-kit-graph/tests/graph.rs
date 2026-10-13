//! Tests for the processing graph: cycle checks, compilation, delay compensation and the executor.

#![allow(clippy::unwrap_used, clippy::panic, reason = "test helpers outside #[test] functions")]

use std::collections::HashSet;

use bodo_kit_core::pool::RtPoolConfig;
use bodo_kit_core::rt::permit;
use bodo_kit_dsp::delay::DelayLine;
use bodo_kit_graph::{CompileError, DagError, DagNode, EdgeDesc, GraphDesc, IncrementalDag, NodeDesc, NodeIo, Processor, compile, runner};
use proptest::prelude::*;

/// The block context the tests pass to every node: the index of the block.
#[derive(Debug, Default)]
struct Block(u64);

/// Test processors.
#[derive(Debug)]
enum Node {
    /// Emits `value` on every sample.
    Constant(f32),
    /// Emits a unit impulse on the first sample of the first block.
    Impulse,
    /// Delays port 0 by its latency, exactly as a plugin with that latency would.
    Delay(Vec<DelayLine>, usize),
    /// Emits the number of blocks it has run, so tests can see whether it survived a recompile.
    Counter(f32),
    /// Panics.
    Panic,
    /// Port 0 times a node-specific factor, plus a node-specific sine, so every node changes the signal.
    Mix { factor: f32, phase: f32 },
}

impl Processor<Block> for Node {
    fn process(&mut self, io: &mut NodeIo<'_>, context: &Block) {
        let frames = io.frames;
        for (channel_index, out) in io.output.channels_mut().enumerate() {
            let input = io.inputs.first().map(|port| port.channel(channel_index));
            match self {
                Node::Constant(value) => out.fill(*value),
                Node::Impulse => {
                    out.fill(0.0);
                    if context.0 == 0 {
                        out[0] = 1.0;
                    }
                }
                Node::Delay(lines, latency) => {
                    out.copy_from_slice(&input.unwrap()[..frames]);
                    lines[channel_index].process(out, *latency);
                }
                Node::Counter(count) => out.fill(*count),
                Node::Panic => panic!("deliberate"),
                Node::Mix { factor, phase } => {
                    for (i, sample) in out.iter_mut().enumerate() {
                        let x = input.map_or(0.0, |s| s[i]);
                        *sample = x * *factor + (*phase + i as f32 * 0.01 + context.0 as f32).sin() * 0.1;
                    }
                }
            }
        }
        if let Node::Counter(count) = self {
            *count += 1.0;
        }
    }
}

fn node(key: u64, ports: usize, latency: u32, processor: Option<Node>) -> NodeDesc<Node> {
    NodeDesc { key, channels: 2, ports, latency, processor }
}

fn edge(key: u64, from: u64, to: u64) -> EdgeDesc {
    EdgeDesc { key, from, to, port: 0, port_offset: 0, gain: 1.0, compensate: true }
}

fn render(desc: GraphDesc<Node>, workers: usize, blocks: usize, frames: usize) -> Vec<f32> {
    let compiled = compile(desc, 512, None).unwrap();
    let (mut runner, _sender) = runner(compiled.plan, Block(0), RtPoolConfig::new(workers)).unwrap();
    let mut out = Vec::new();
    for block in 0..blocks {
        runner.process(frames, |b| b.0 = block as u64);
        // Copying out allocates, which the real-time checks would flag, so it is explicitly permitted here.
        runner.read_output(|buffer| permit(|| (0..2).for_each(|c| out.extend_from_slice(buffer.channel(c))))).unwrap();
    }
    assert_eq!(runner.faults(), 0);
    out
}

#[test]
fn a_chain_runs_in_order() {
    let desc = GraphDesc {
        nodes: vec![node(1, 0, 0, Some(Node::Constant(0.5))), node(2, 1, 0, None), node(3, 1, 0, None)],
        edges: vec![edge(10, 1, 2), EdgeDesc { gain: 0.5, ..edge(11, 2, 3) }],
        output: Some(3),
        taps: vec![],
        gain_ramp: 0,
    };
    let out = render(desc, 0, 1, 8);
    assert!(out.iter().all(|&s| s == 0.25), "{out:?}");
}

#[test]
fn compile_reports_errors() {
    let cycle = GraphDesc {
        nodes: vec![node(1, 1, 0, None), node(2, 1, 0, None), node(3, 1, 0, None)],
        edges: vec![edge(10, 1, 2), edge(11, 2, 3), edge(12, 3, 2)],
        output: None,
        taps: vec![],
        gain_ramp: 0,
    };
    match compile(cycle, 64, None).unwrap_err() {
        CompileError::Cycle { path } => assert_eq!(path.iter().copied().collect::<HashSet<_>>(), HashSet::from([2, 3])),
        other => panic!("{other:?}"),
    }
    let duplicate = GraphDesc::<Node> { nodes: vec![node(1, 0, 0, None), node(1, 0, 0, None)], ..GraphDesc::default() };
    assert_eq!(compile(duplicate, 64, None).unwrap_err(), CompileError::DuplicateNode(1));
    let unknown = GraphDesc::<Node> { nodes: vec![node(1, 1, 0, None)], edges: vec![edge(10, 9, 1)], output: None, taps: vec![], gain_ramp: 0 };
    assert_eq!(compile(unknown, 64, None).unwrap_err(), CompileError::UnknownNode { node: 9 });
    let port = GraphDesc::<Node> {
        nodes: vec![node(1, 0, 0, None), node(2, 1, 0, None)],
        edges: vec![EdgeDesc { port: 3, ..edge(10, 1, 2) }],
        output: None,
        taps: vec![],
        gain_ramp: 0,
    };
    assert_eq!(compile(port, 64, None).unwrap_err(), CompileError::NoSuchPort { edge: 10, port: 3 });
}

#[test]
fn delay_compensation_matches_the_spec_formulas() {
    // a (latency 100) and b (0) both feed c; d feeds c's sidechain port, which c uses 30 frames into its 50.
    let desc = GraphDesc::<Node> {
        nodes: vec![node(1, 0, 100, None), node(2, 0, 0, None), node(3, 2, 50, None), node(4, 0, 200, None)],
        edges: vec![edge(10, 1, 3), edge(11, 2, 3), EdgeDesc { port: 1, port_offset: 30, ..edge(12, 4, 3) }],
        output: Some(3),
        taps: vec![],
        gain_ramp: 0,
    };
    let report = compile(desc, 64, None).unwrap().latency;
    // in(c) = max(100 - 0, 0 - 0, 200 - 30) = 170; out(c) = 220.
    assert_eq!(report.nodes[&3].input, 170);
    assert_eq!(report.nodes[&3].output, 220);
    assert_eq!(report.total, 220);
    assert_eq!(report.edges[&10], 70);
    assert_eq!(report.edges[&11], 170);
    assert_eq!(report.edges[&12], 0, "170 + 30 - 200");
}

#[test]
fn processors_survive_recompiles_unless_replaced() {
    let desc = |processor| GraphDesc { nodes: vec![node(1, 0, 0, processor)], edges: vec![], output: Some(1), taps: vec![], gain_ramp: 0 };
    let first = compile(desc(Some(Node::Counter(0.0))), 64, None).unwrap();
    let (mut runner, mut sender) = runner(first.plan, Block(0), RtPoolConfig::new(1)).unwrap();
    let value = |runner: &mut bodo_kit_graph::GraphRunner<Node, Block>| {
        runner.process(4, |_| {});
        runner.read_output(|b| b.channel(0)[0]).unwrap()
    };
    assert_eq!(value(&mut runner), 0.0);
    assert_eq!(value(&mut runner), 1.0);
    let kept = compile(desc(None), 64, Some(&first.layout)).unwrap();
    assert!(sender.send(kept.plan).is_ok());
    assert_eq!(value(&mut runner), 2.0, "the counter moved to the new plan");
    assert_eq!(sender.collect(), 1);
    let replaced = compile(desc(Some(Node::Counter(10.0))), 64, Some(&kept.layout)).unwrap();
    assert!(sender.send(replaced.plan).is_ok());
    assert_eq!(value(&mut runner), 10.0, "a new processor replaces the old one");
}

#[test]
fn edge_gains_change_without_recompiling() {
    let desc = GraphDesc {
        nodes: vec![node(1, 0, 0, Some(Node::Constant(1.0))), node(2, 1, 0, None)],
        edges: vec![edge(10, 1, 2)],
        output: Some(2),
        taps: vec![],
        gain_ramp: 0,
    };
    let compiled = compile(desc, 64, None).unwrap();
    let gains = compiled.gains;
    let (mut runner, _sender) = runner(compiled.plan, Block(0), RtPoolConfig::new(0)).unwrap();
    runner.process(4, |_| {});
    assert!(gains.set(10, 0.0));
    assert!(!gains.set(99, 0.0));
    runner.process(4, |_| {});
    let first = |b: &bodo_kit_dsp::AudioBuffer| <[f32; 4]>::try_from(b.channel(0)).unwrap();
    assert_eq!(runner.read_output(first).unwrap(), [0.75, 0.5, 0.25, 0.0], "ramped over the block");
    runner.process(4, |_| {});
    assert_eq!(runner.read_output(first).unwrap(), [0.0; 4]);
}

#[test]
fn a_panicking_processor_is_contained() {
    if std::env::var_os("RTSAN_ENABLE").is_some() {
        // Unwinding allocates, which RealtimeSanitizer rightly stops; the containment is tested without it.
        return;
    }
    let desc = GraphDesc {
        nodes: vec![node(1, 0, 0, Some(Node::Panic)), node(2, 0, 0, Some(Node::Constant(1.0))), node(3, 1, 0, None)],
        edges: vec![edge(10, 1, 3), edge(11, 2, 3)],
        output: Some(3),
        taps: vec![],
        gain_ramp: 0,
    };
    let compiled = compile(desc, 64, None).unwrap();
    let (mut runner, _sender) = runner(compiled.plan, Block(0), RtPoolConfig::new(2)).unwrap();
    for _ in 0..3 {
        runner.process(8, |_| {});
    }
    assert_eq!(runner.read_output(|b| b.channel(0)[0]).unwrap(), 1.0, "the rest of the graph keeps playing");
    assert_eq!(runner.faults(), 1, "the processor is skipped after it panicked once");
}

/// A random DAG over `n` nodes: edges only go from lower to higher indices, so it is acyclic by construction.
fn random_dag() -> impl Strategy<Value = (usize, Vec<(usize, usize)>, Vec<u32>)> {
    (2usize..14).prop_flat_map(|n| {
        let edges = prop::collection::vec((0..n, 0..n), 0..n * 3).prop_map(|pairs| pairs.into_iter().filter(|(a, b)| a < b).collect::<Vec<_>>());
        (Just(n), edges, prop::collection::vec(0u32..40, n))
    })
}

/// Builds a graph from a random DAG. Nodes without inputs are sources of type `source`; the rest are `inner`. Every
/// node without outputs feeds a final summing node, which is the output.
fn build(n: usize, edges: &[(usize, usize)], latencies: &[u32], source: impl Fn() -> Node, inner: impl Fn(usize, u32) -> Node) -> GraphDesc<Node> {
    let has_input: HashSet<usize> = edges.iter().map(|&(_, b)| b).collect();
    let has_output: HashSet<usize> = edges.iter().map(|&(a, _)| a).collect();
    let mut nodes: Vec<NodeDesc<Node>> = (0..n)
        .map(|i| {
            if has_input.contains(&i) { node(i as u64, 1, latencies[i], Some(inner(i, latencies[i]))) } else { node(i as u64, 0, 0, Some(source())) }
        })
        .collect();
    let sink = n as u64;
    nodes.push(node(sink, 1, 0, None));
    let mut descs: Vec<EdgeDesc> = edges.iter().enumerate().map(|(k, &(a, b))| edge(k as u64, a as u64, b as u64)).collect();
    for i in (0..n).filter(|i| !has_output.contains(i)) {
        descs.push(edge(1000 + i as u64, i as u64, sink));
    }
    GraphDesc { nodes, edges: descs, output: Some(sink), taps: vec![], gain_ramp: 0 }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn incremental_dag_agrees_with_brute_force(n in 2usize..12, attempts in prop::collection::vec((0usize..12, 0usize..12), 0..60)) {
        let mut dag = IncrementalDag::new();
        let nodes: Vec<DagNode> = (0..n).map(|_| dag.add_node()).collect();
        let mut edges: Vec<(usize, usize)> = Vec::new();
        for (a, b) in attempts.into_iter().map(|(a, b)| (a % n, b % n)) {
            // Brute force: the edge closes a cycle when `a` is reachable from `b`.
            let mut reach = vec![false; n];
            let mut stack = vec![b];
            while let Some(x) = stack.pop() {
                if !reach[x] {
                    reach[x] = true;
                    stack.extend(edges.iter().filter(|e| e.0 == x).map(|e| e.1));
                }
            }
            let closes_cycle = reach[a];
            match dag.add_edge(nodes[a], nodes[b]) {
                Ok(()) => {
                    prop_assert!(!closes_cycle);
                    edges.push((a, b));
                }
                Err(DagError::Cycle { path }) => {
                    prop_assert!(closes_cycle);
                    // The path starts with the refused edge and closes through existing edges.
                    prop_assert_eq!(path[0], nodes[a]);
                    let index = |d: DagNode| nodes.iter().position(|&x| x == d).unwrap();
                    for pair in path.windows(2).skip(1) {
                        prop_assert!(edges.contains(&(index(pair[0]), index(pair[1]))));
                    }
                    if path.len() > 1 {
                        prop_assert_eq!(index(path[1]), b);
                        prop_assert!(edges.contains(&(index(*path.last().unwrap()), a)));
                    }
                }
                Err(other) => prop_assert!(false, "{other:?}"),
            }
            let order = dag.topological_order();
            let rank = |i: usize| order.iter().position(|&d| d == nodes[i]).unwrap();
            for &(x, y) in &edges {
                prop_assert!(rank(x) < rank(y));
            }
        }
    }

    #[test]
    fn delay_compensation_aligns_every_path((n, edges, latencies) in random_dag()) {
        // Each inner node really delays its signal by its reported latency. After compensation, an impulse from
        // every source reaches the output at the same frame: the total latency.
        let desc = build(n, &edges, &latencies, || Node::Impulse, |_, latency| Node::Delay(vec![DelayLine::new(64), DelayLine::new(64)], latency as usize));
        let total = compile(build(n, &edges, &latencies, || Node::Impulse, |_, _| Node::Impulse), 512, None).unwrap().latency.total;
        let out = render(desc, 1, 3, 512);
        let left = &out[..512];
        let nonzero: Vec<usize> = (0..512).filter(|&i| left[i] != 0.0).collect();
        prop_assert_eq!(nonzero, vec![total as usize]);
    }

    #[test]
    fn output_is_identical_for_any_number_of_workers((n, edges, latencies) in random_dag(), frames in 1usize..300) {
        let make = || build(n, &edges, &latencies, || Node::Constant(0.25), |i, _| Node::Mix { factor: 0.5 + i as f32 * 0.1, phase: i as f32 });
        let single = render(make(), 0, 4, frames);
        let parallel = render(make(), 3, 4, frames);
        prop_assert_eq!(single.iter().map(|s| s.to_bits()).collect::<Vec<_>>(), parallel.iter().map(|s| s.to_bits()).collect::<Vec<_>>());
    }
}
