# ADR 0025: The mixer strip as graph nodes: delay compensation, solo, crossfades and buffer reuse

- **Status:** Accepted
- **Date:** 2026-10-13
- **Deciders:** hasan-muzak
- **Refines:** ADR 0009 (strip order and PDC) and ADR 0015 (graph executor), whose deferred items it closes

## Context

M4 builds the whole mixer of project.md section 9: the strip in the exact order of section 9.3, routes with taps, sidechains into individual slots, delay compensation as section 9.4 defines it, solo by audible set, meters, and CPU per insert. ADR 0015 left three things for M4: the 5 ms crossfade on topology changes, buffer reuse, and CPU time per node. The engine has to do all of this without allocating, locking or waiting on the audio thread, and offline renders must stay bit-identical.

Taps are the hard part. A send can leave an insert at PreFx, PreFader or PostFader, and a sidechain can enter in front of any of the 10 slots. One opaque node per insert can't express either.

## Decision

- **Three nodes per insert** (`bodo-mix` `nodes/strip.rs`):
  1. The **input stage** gets the latency-aligned input sum from the graph, adds the hardware input of an armed insert, and applies polarity and swap. Its output is the PreFx tap. It records the Input and PreFx taps.
  2. The **FX stage** runs slots 1 to 10, then the strip EQ, then stereo separation. Its output is the PreFader tap. It records the PostFx tap, after the EQ, as section 9.3 places it. When a slot has a sidechain, the slot gets an input port of its own.
  3. The **output stage** runs pan, the fader (smoothed per sample), mute and solo silencing with a 5 ms ramp, and the post-fader point (meters, the PostFader record tap). Then comes the delay for a negative track latency. Its output feeds the master, the PostFader routes and the direct hardware output.

  A route's tap picks the node its edge leaves from. The stages of different inserts run in parallel.
- **Delay compensation** uses ADR 0015's port offsets with no mixer-specific code in Kit:
  - The FX stage's latency is the sum of its slot latencies plus `max(latency_offset, 0)`. A disabled slot keeps its latency, and the dry path of a partial mix is delayed by the slot's latency.
  - A sidechain edge enters the FX stage at port `k` with offset `pre_k`, the latency of the slots before slot `k`. That gives section 9.4's `in(n) + pre_k - out(sc)`.
  - A negative offset is a real delay in the output stage, so it adds to the insert's output latency.
  - `PdcBypass::Always` marks the insert's outgoing edges as uncompensated: they carry no delay and don't raise `in(n)` downstream. `WhileRecording` does the same only while the transport records; the engine recompiles when recording starts and stops.
  - Automation for a node is read at `t - in(n)` (`LaneDelays`). Lanes are evaluated once per distinct delay per block, and route levels are automated through the edge gain handles.
  - `mix_latency(project, recording)` reports the total latency and each insert's input, own and output latency without running anything. `inspect_mixer` and `Engine::total_latency` use it.
- **Solo** silences inserts outside `S ∪ ancestors(S) ∪ descendants(S)`, computed over audio routes and `to_master` edges. The silenced inserts keep processing at gain 0, so their sidechains still work. The flag is an atomic on the strip, so solo changes don't recompile.
- **Crossfades on recompile.** Each plan has a gain ramp of 5 ms. An edge new to a plan fades in from 0. A removed edge or node stays in the next plan for 40 ms as a ghost: gain 0, uncompensated, its processor adopted. It ramps out instead of cutting off, and `Engine::flush` drops it later. If a plan with ghosts can't compile, the plan compiles without them.
- **Buffer reuse.** A node writes into the buffer of its input when it is that input's only reader and the input is neither the graph's output nor tapped. A strip's three stages and a chain of single-route inserts then share one buffer. The rule needs no liveness analysis across threads: the reader runs only after the writer, and nothing else reads that buffer.
- **Taps** are a Kit feature: a `GraphDesc` lists node keys with a 64-bit tag, and the runner lends each tapped buffer with its tag after a block. Stems and direct hardware outputs use taps.
- **Monitoring** is lock-free:
  - Per-node times come from the executor's monotonic clock (`NodeTimes`). `Engine::insert_cpu` adds up an insert's three stages.
  - Meters keep their peak since the last read, a clip latch and the master's correlation in atomics (`StripMeter`).
  - The master's last 4096 frames sit in a ring of atomics (`Scope`).
  - The UI reads all of these by insert id, with no messages and no locks.

## Consequences

- Every tap and sidechain point of section 9.3 is a graph edge, so delay compensation, the cycle check and parallel execution see the same graph. A strip costs three nodes instead of one; buffer sharing keeps the memory where it was.
- Null tests hold: at default settings the strip is a bit-exact pass-through, and the random-graph proptest nulls delay-compensated paths against the uncompensated sum.
- A ghost lives one recompile longer than its route. A burst of edits within 40 ms keeps several generations of ghosts, which is bounded by the edit rate and cleared by `flush`.
- Automation is read once per block, at the block's start, per distinct delay. Smoother, sample-accurate parameter automation belongs with the parameter queues of M5.

## Alternatives considered

- **One node per insert with internal taps:** the graph couldn't route from PreFx or PreFader, or line up a sidechain with slot `k`, without mixer-specific code in Kit.
- **Recompiling on solo:** simple, but a solo click would rebuild the plan and cut sidechains for a block.
- **Liveness-based buffer reuse across the whole plan:** saves more memory, but with parallel execution reuse is only safe when every earlier reader is an ancestor of the new writer. The single-reader rule is safe by construction and covers the common case: strip stages and bus chains.
- **Fading by crossfading whole plans:** it would run two graphs for 5 ms and double the CPU at every edit. Per-edge ramps and ghosts cost only the edges that changed.
