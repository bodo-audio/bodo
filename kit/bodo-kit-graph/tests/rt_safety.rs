//! Checks that running a graph, including switching to a new plan, never allocates on the audio thread. Debug
//! builds only; the workers are covered by the RealtimeSanitizer job.

#![cfg(debug_assertions)]
#![allow(clippy::unwrap_used, reason = "test helpers outside #[test] functions")]

use assert_no_alloc::{AllocDisabler, assert_no_alloc, reset_violation_count, violation_count};
use bodo_kit_core::pool::RtPoolConfig;
use bodo_kit_graph::{EdgeDesc, GraphDesc, NodeDesc, NodeIo, Processor, compile, runner};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

struct Tone(f32);

impl Processor<()> for Tone {
    fn process(&mut self, io: &mut NodeIo<'_>, _context: &()) {
        for channel in io.output.channels_mut() {
            channel.fill(self.0);
        }
        self.0 += 0.001;
    }
}

fn desc(latency: u32, fresh: bool) -> GraphDesc<Tone> {
    let node = |key, ports, latency, processor| NodeDesc { key, channels: 2, ports, latency, processor };
    GraphDesc {
        nodes: vec![node(1, 0, 0, fresh.then_some(Tone(0.1))), node(2, 0, latency, fresh.then_some(Tone(0.2))), node(3, 1, 0, None)],
        edges: vec![
            EdgeDesc { key: 10, from: 1, to: 3, port: 0, port_offset: 0, gain: 0.5 },
            EdgeDesc { key: 11, from: 2, to: 3, port: 0, port_offset: 0, gain: 0.5 },
        ],
        output: Some(3),
    }
}

#[test]
fn processing_and_plan_switches_do_not_allocate() {
    let first = compile(desc(32, true), 256, None).unwrap();
    let second = compile(desc(48, false), 256, Some(&first.layout)).unwrap();
    let (mut runner, mut sender) = runner(first.plan, (), RtPoolConfig::new(0)).unwrap();
    assert!(sender.send(second.plan).is_ok());
    reset_violation_count();
    assert_no_alloc(|| {
        for _ in 0..50 {
            runner.process(256, |_| {});
            runner.read_output(|buffer| std::hint::black_box(buffer.channel(0)[0]));
        }
    });
    assert_eq!(violation_count(), 0);
    assert_eq!(sender.collect(), 1, "the first plan came back to be freed here");
    assert_eq!(runner.faults(), 0);
}
