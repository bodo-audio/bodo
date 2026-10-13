//! Plan features added with the full mixer: taps, buffers handed on along chains, edges left out of delay
//! compensation, gain ramps that crossfade topology changes, and processing times.

#![allow(clippy::unwrap_used, clippy::panic, reason = "test helpers outside #[test] functions")]

use bodo_kit_core::pool::RtPoolConfig;
use bodo_kit_dsp::delay::DelayLine;
use bodo_kit_graph::{EdgeDesc, GraphDesc, GraphRunner, NodeDesc, NodeIo, Processor, compile, runner};

/// The index of the block.
#[derive(Debug, Default)]
struct Block(u64);

#[derive(Debug)]
enum Node {
    /// Emits `value` on every sample.
    Constant(f32),
    /// Emits a unit impulse on the first sample of the first block.
    Impulse,
    /// Delays port 0 by its latency.
    Delay(Vec<DelayLine>, usize),
    /// Port 0 times a factor.
    Scale(f32),
    /// Burns some time.
    Busy,
}

impl Processor<Block> for Node {
    fn process(&mut self, io: &mut NodeIo<'_>, context: &Block) {
        let frames = io.frames;
        for (channel, out) in io.output.channels_mut().enumerate() {
            let input = io.inputs.first().map(|port| port.channel(channel));
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
                    lines[channel].process(out, *latency);
                }
                Node::Scale(factor) => {
                    for (o, i) in out.iter_mut().zip(input.unwrap()) {
                        *o = *i * *factor;
                    }
                }
                Node::Busy => {
                    let mut x = 0.5f32;
                    for _ in 0..2000 {
                        x = (x * 1.0001).sin();
                    }
                    out.fill(x * 1e-9);
                }
            }
        }
    }
}

fn node(key: u64, ports: usize, latency: u32, processor: Option<Node>) -> NodeDesc<Node> {
    NodeDesc { key, channels: 2, ports, latency, processor }
}

fn edge(key: u64, from: u64, to: u64) -> EdgeDesc {
    EdgeDesc { key, from, to, port: 0, port_offset: 0, gain: 1.0, compensate: true }
}

#[test]
fn taps_read_any_node_and_never_hand_their_buffer_on() {
    // 1 -> 2 -> 3: 2 would hand its buffer on to 3, but when it is tapped it keeps it.
    let desc = |taps: Vec<u64>| GraphDesc {
        nodes: vec![node(1, 0, 0, Some(Node::Constant(0.5))), node(2, 1, 0, Some(Node::Scale(2.0))), node(3, 1, 0, None)],
        edges: vec![edge(10, 1, 2), EdgeDesc { gain: 0.5, ..edge(11, 2, 3) }],
        output: Some(3),
        taps,
        gain_ramp: 0,
    };
    let untapped = compile(desc(vec![]), 64, None).unwrap();
    assert_eq!(untapped.plan.buffer_count(), 1, "a chain of single readers shares one buffer");
    let compiled = compile(desc(vec![2]), 64, None).unwrap();
    assert_eq!(compiled.plan.buffer_count(), 2);
    let (mut runner, _sender) = runner(compiled.plan, Block(0), RtPoolConfig::new(0)).unwrap();
    runner.process(8, |_| {});
    assert_eq!(runner.read_tap(0, |b| b.channel(0).to_vec()).unwrap(), [1.0; 8]);
    assert_eq!(runner.read_output(|b| b.channel(0).to_vec()).unwrap(), [0.5; 8]);
    assert_eq!(runner.tap_count(), 1);
    assert!(runner.read_tap(1, |_| ()).is_none());
}

#[test]
fn shared_buffers_render_like_separate_ones() {
    // A chain through five delays: every node hands its buffer on, and the impulse still comes out on time.
    let desc = |tapped: bool| {
        let mut nodes = vec![node(0, 0, 0, Some(Node::Impulse))];
        for k in 1..6u64 {
            nodes.push(node(k, 1, 3, Some(Node::Delay(vec![DelayLine::new(3), DelayLine::new(3)], 3))));
        }
        let edges = (1..6u64).map(|k| edge(100 + k, k - 1, k)).collect();
        let taps = if tapped { (0..5).collect() } else { vec![] };
        GraphDesc { nodes, edges, output: Some(5), taps, gain_ramp: 0 }
    };
    let shared = compile(desc(false), 64, None).unwrap();
    assert_eq!(shared.plan.buffer_count(), 1);
    let separate = compile(desc(true), 64, None).unwrap();
    assert_eq!(separate.plan.buffer_count(), 6);
    let run = |plan| {
        let (mut runner, _sender) = runner(plan, Block(0), RtPoolConfig::new(2)).unwrap();
        runner.process(32, |b| b.0 = 0);
        runner.read_output(|b| b.channel(0).to_vec()).unwrap()
    };
    let (a, b) = (run(shared.plan), run(separate.plan));
    assert_eq!(a, b);
    assert_eq!(a.iter().position(|&s| s == 1.0), Some(15), "five delays of 3 frames");
}

#[test]
fn edges_left_out_of_compensation_arrive_undelayed() {
    // a (latency 100) and b (0) feed c; b's edge isn't compensated, so it isn't delayed. d (300) feeds e the same way.
    let desc = GraphDesc::<Node> {
        nodes: vec![node(1, 0, 100, None), node(2, 0, 0, None), node(3, 1, 0, None), node(4, 0, 300, None), node(5, 1, 0, None)],
        edges: vec![edge(10, 1, 3), EdgeDesc { compensate: false, ..edge(11, 2, 3) }, EdgeDesc { compensate: false, ..edge(12, 4, 5) }],
        output: Some(3),
        ..GraphDesc::default()
    };
    let report = compile(desc, 64, None).unwrap().latency;
    assert_eq!(report.edges[&10], 0);
    assert_eq!(report.edges[&11], 0, "left undelayed");
    assert_eq!(report.nodes[&3].input, 100);
    assert_eq!(report.nodes[&5].input, 0, "an uncompensated edge doesn't make its target wait");
}

fn ramp_desc(second: bool, fresh: bool) -> GraphDesc<Node> {
    let mut nodes = vec![node(1, 0, 0, fresh.then_some(Node::Constant(1.0))), node(3, 1, 0, None)];
    let mut edges = vec![edge(10, 1, 3)];
    if second {
        nodes.push(node(2, 0, 0, Some(Node::Constant(1.0))));
        edges.push(edge(11, 2, 3));
    }
    GraphDesc { nodes, edges, output: Some(3), taps: vec![], gain_ramp: 16 }
}

fn block(runner: &mut GraphRunner<Node, Block>) -> Vec<f32> {
    runner.process(4, |_| {});
    runner.read_output(|b| b.channel(0).to_vec()).unwrap()
}

#[test]
fn gain_changes_and_new_edges_ramp_over_the_gain_ramp() {
    let first = compile(ramp_desc(false, true), 64, None).unwrap();
    let (mut runner, mut sender) = runner(first.plan, Block(0), RtPoolConfig::new(0)).unwrap();
    assert_eq!(block(&mut runner), [1.0; 4], "the first plan starts at full gain");
    assert!(first.gains.set(10, 0.0));
    // A full swing takes 16 frames: a quarter per block of 4.
    assert_eq!(block(&mut runner), [0.9375, 0.875, 0.8125, 0.75]);
    assert_eq!(block(&mut runner)[3], 0.5);
    assert!(first.gains.set(10, 1.0));
    let second = compile(ramp_desc(true, false), 64, Some(&first.layout)).unwrap();
    assert!(sender.send(second.plan).is_ok());
    // Edge 10 was carried over at 0.5 and climbs back; edge 11 is new and fades in from silence.
    assert_eq!(block(&mut runner)[3], 0.75 + 0.25);
    for _ in 0..4 {
        block(&mut runner);
    }
    assert_eq!(block(&mut runner), [2.0; 4]);
}

#[test]
fn node_times_add_up() {
    let desc = GraphDesc { nodes: vec![node(1, 0, 0, Some(Node::Busy))], edges: vec![], output: Some(1), taps: vec![], gain_ramp: 0 };
    let compiled = compile(desc, 512, None).unwrap();
    let times = compiled.times;
    let (mut runner, _sender) = runner(compiled.plan, Block(0), RtPoolConfig::new(0)).unwrap();
    assert_eq!(times.nanos(1), Some(0));
    for _ in 0..20 {
        runner.process(512, |_| {});
    }
    assert!(times.nanos(1).unwrap() > 0);
    assert_eq!(times.nanos(9), None);
}
