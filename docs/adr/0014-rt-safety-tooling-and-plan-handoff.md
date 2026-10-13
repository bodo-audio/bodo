# ADR 0014: RT-safety tooling and handing plans to the audio thread

- **Status:** Accepted
- **Date:** 2026-10-13
- **Deciders:** hasan-muzak
- **Refines:** ADR 0003, on how plans reach the audio thread

## Context

M1 builds the first code that runs on the audio thread, so the checks that project.md section 19.3 promises must exist now:

- `#[rt_safe]`, an attribute that checks a function in debug builds;
- a RealtimeSanitizer (rtsan) job;
- denormals flushed to zero on every RT thread.

ADR 0003 also says new execution plans are published through `arc-swap`, and retired ones go back to a GC thread through `basedrop` or a return queue. Loading an `Arc` on the audio thread is cheap, but dropping the last reference there would free the plan, so the audio thread would need a return queue anyway.

## Decision

- **`bodo-kit-macros`** is a proc-macro crate that holds `#[rt_safe]`, and `bodo-kit-core` re-exports it. Macro crates get their own tier below L0: any crate may depend on them, and they depend on no workspace crate. `cargo xtask layers` knows the tier.
- **`#[rt_safe]`** requires the function's docs to say `RT-safe`, so the doc line and the check can't drift apart. It wraps the body in `bodo_kit_core::rt::guarded`, which:
  - runs it under `assert_no_alloc`, which reports any allocation in debug builds of programs that install its allocator (every Bodo app and every RT test does);
  - enters RealtimeSanitizer's real-time context through `rtsan-standalone`. That costs nothing unless the build sets `RTSAN_ENABLE`, and then rtsan also catches locks, blocking calls and I/O.
- **Denormals:** `bodo_kit_core::rt::flush_denormals` sets the flush-to-zero flags for the duration of a closure, through the `no_denormals` crate, inside `bodo-kit-core`'s `ffi` module. RT pool workers run their whole life inside it, and the device layer wraps every audio callback in it.
- **Plan handoff:** `bodo_kit_core::handoff` replaces `arc-swap` and `basedrop` for publishing plans.
  - A sender on the engine control thread and a receiver on the audio thread are joined by two bounded queues: new values go in, and retired values come back.
  - At the start of a block, the receiver switches to the newest value, but only while the return queue has room for the old one, so the audio thread never drops a value.
  - The control thread collects the retired values and drops them.
- **The rtsan job** runs the RT test suites with `RTSAN_ENABLE=1` on Linux and macOS in CI.

## Consequences

- A missing `RT-safe` doc line or a stray allocation shows up in the first debug test run, not in a dropout report.
- `extern crate self as bodo_kit_core` in `bodo-kit-core` lets the macro's generated paths work inside that crate too.
- The FPU mode change is formally outside Rust's floating-point model. It only affects denormal results, which audio code wants flushed anyway, and it is confined to one function with a `SAFETY` comment.
- Handoff needs no reference counting, no epoch scheme and no extra dependency. Its cost is two queue operations per published plan.
- If the control thread stops collecting, the return queue fills up and the audio thread keeps playing the plan it has, rather than dropping anything. The control thread must collect regularly.

## Alternatives considered

- **`arc-swap` plus a return queue:** this works, but it needs two mechanisms where one suffices.
- **`basedrop`:** it is unmaintained since 2021, and its collector thread does the same job as the return queue.
- **Fixing denormals inside each DSP algorithm:** it is easy to miss one path, and third-party plugins run on the same threads.
- **Writing the allocation checker ourselves:** it would need our own `unsafe` global allocator; `assert_no_alloc` is small and widely used.
