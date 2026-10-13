# ADR 0015: Graph executor: pull summing, state that travels with plans

- **Status:** Accepted
- **Date:** 2026-10-13
- **Deciders:** hasan-muzak
- **Refines:** ADR 0003 (graph execution) and ADR 0009 (compilation and delay compensation)

## Context

`bodo-kit-graph` turns a description of the mixer graph into something the audio thread and the RT workers can run. project.md sections 6.5 and 9.4 and ADR 0003 set the shape: Kahn ordering, one delay-compensation pass, dependency counters per node, a lock-free ready queue, O(V + E) per block, and plugin instances that live outside the plan. Several details were left open:

- how inputs are summed when nodes run in parallel;
- where node state lives while plans come and go;
- how a send level changes without a recompile;
- what happens when a processor misbehaves.

Offline renders must also be bit-identical from run to run.

## Decision

- **Pull summing.** A node sums its own inputs when it runs, in a fixed order: by port, then by edge key. Producers never add into a consumer's buffer. Results don't depend on thread count or timing, so a render is bit-identical with zero workers or many.
- **Generic delay compensation.** Each edge enters its target at a port with an offset: how far into the target's own latency that port is used. That is zero for the main input, and the latency of the earlier slots for a sidechain input. Then `in(n) = max(out(src) - offset)`, `out(n) = in(n) + L(n)`, and each edge is delayed by `in(n) + offset - out(src)`. This is the formula of ADR 0009, with nothing mixer-specific in the Kit crate.
- **State travels with plans.** Each plan owns its nodes' processor slots and the delay lines of its edges. When the audio thread switches plans, it moves each processor described as "keep" into the new plan, along with the contents of every edge whose delay didn't change. It swaps pointers, so nothing allocates, and the old plan carries what is left back to the control thread to be dropped. Plugin instances are never re-created by a recompile.
- **Edge gains** live in atomics shared between a plan and the control thread. A send-level change is one store, and the executor ramps to the new gain over the next block.
- **Interior mutability without locks:** per-node `AtomicRefCell`s. The schedule guarantees they never conflict; a conflict would be counted as a fault, never waited on.
- **A panicking processor** is caught on the thread that ran it and marked poisoned. Its output is silent from then on, the fault is counted, and the rest of the graph keeps playing. A worker never dies holding the block open.
- **Plans are boxed** through the handoff queue, so a switch moves a pointer, not the whole plan.

## Consequences

- Deterministic rendering comes for free. The cost is that a node with many inputs sums them alone, on one thread.
- Every node has its own output buffer, and every input port has a summing buffer. Reusing buffers across nodes by liveness analysis (project.md section 6.5) is deferred: with parallel execution, a buffer can only be reused once every reader of its previous contents is an ancestor of the new writer, and that check is worth building once real sessions show the memory matters.
- The 5 ms crossfade on topology changes (ADR 0003) is deferred to M4, with the full mixer; plans switch at a block boundary until then.
- CPU time per node (project.md section 6.9) is measured in M4.

## Alternatives considered

- **Push summing** (producers add into consumers): less copying, but the floating-point summation order would depend on thread timing, so renders would no longer be bit-identical.
- **A node store outside the plan,** with insert and remove messages: it needs a fixed-capacity store on the audio thread and an extra message protocol. Carrying state inside plans needs neither.
- **Re-creating delay lines on every recompile:** simpler, but every route or mute change would make compensated tracks glitch.
