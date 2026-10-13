# ADR 0003: Real-time engine and threading model

- **Status:** Accepted; plan publishing refined by ADR 0014
- **Date:** 2026-10-13
- **Deciders:** hasan-muzak

## Context

Dropouts are the worst bug a DAW can have. Reviewing soundcraft turned up several real-time hazards we must avoid:

- a rayon pool joined from inside the audio callback (priority inversion);
- snapshots of the whole document cloned and freed on the audio thread;
- `std::sync::mpsc` sends from the callback;
- parameters looked up by string on every block;
- a fixed block size, no device selection, and no duplex clock.

## Decision

- **Thread roles:**
  - the audio callback (driver);
  - an RT worker pool (physical cores minus one, capped);
  - an engine control thread;
  - a disk I/O thread;
  - a GC thread;
  - the UI thread;
  - a tokio runtime, top layer only.

  Responsibilities are listed in `project.md` section 6.1.
- **The RT contract:** no allocation, no locks, no I/O or syscalls, no panics, denormals flushed to zero, and a fixed maximum block of 4096 frames with everything preallocated. Integer ids only.
- **Communication:**
  - `rtrb` SPSC queues for commands and parameter changes;
  - `arc-swap` to publish each new `ExecPlan`;
  - `basedrop`, or a return queue to the GC thread, so nothing is ever freed on RT threads;
  - `triple_buffer` and atomics for meters and positions.
- **Graph execution:**
  - the engine control thread compiles the `ExecPlan`, and only when something changed;
  - each block runs a dependency-counter scheduler over the plan, in O(V + E);
  - ready nodes go into a preallocated lock-free queue;
  - workers spin briefly, then wait on a futex-style primitive.
- **OS integration:**
  - macOS: workers join the device's `os_workgroup`;
  - Windows: MMCSS "Pro Audio";
  - Linux: `SCHED_FIFO` through rtkit or PipeWire;
  - promotion goes through `audio_thread_priority`.
- **Devices:** `cpal` 0.18 (with ASIO on Windows), using one duplex clock where the backend supports it and `rtaudio` as the fallback. Device, rate and buffer can all be selected. Recording latency is compensated, with a loopback calibration.
- **Offline render** reuses the same `bodo-mix` code path, deterministically.
- **Verification:** `assert_no_alloc` in debug builds, a `rtsan` job in CI, and stress tests on a virtual device.

## Consequences

- The scheduler and the device layer are code we own, so we must test them heavily. In exchange, scheduling is predictable on every platform.
- Plans are immutable snapshots, which keeps reasoning simple. A topology change crossfades over 5 ms.
- Workers that spin cost CPU when there is little load. The spin is short and adapts to the load.

## Alternatives considered

rayon or a general-purpose thread pool (not RT-safe), level-synchronous execution (wastes parallelism on uneven levels), firewheel as the engine (built for games, and per its own README not a DAW engine; we use it as a reference only), and a single-threaded engine (doesn't scale).
