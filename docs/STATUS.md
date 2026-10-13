# Status

Progress per milestone ([project.md section 20](../project.md#20-roadmap)). Update this file at every milestone.

## M0: Foundations (done, 2026-10-13)

| Item | State | Where |
|---|---|---|
| Workspace, toolchain, lints | Done | `Cargo.toml`, `rust-toolchain.toml` (Rust 1.98.1), `rustfmt.toml`, `clippy.toml` |
| `cargo xtask ci` | Done | fmt, clippy (default and all features), tests, docs, release build, then every check below |
| `cargo xtask layers` | Done | `xtask/src/layers.rs`; fixture `xtask/fixtures/layers` |
| `cargo xtask unsafe-audit` | Done | `xtask/src/unsafe_audit.rs` |
| `cargo xtask deny` | Done | `deny.toml`, `kit/deny.toml`; fixture `xtask/fixtures/licenses` |
| `cargo xtask text` | Done | `xtask/src/text.rs` |
| CI on Linux, macOS and Windows | Done | `.github/workflows/ci.yml` on [bodo-audio/bodo](https://github.com/bodo-audio/bodo) |
| Docs skeleton | Done | this file, [`architecture.md`](architecture.md), [`agents/`](agents/), [`formats/`](formats/) |
| `bodo-kit-core` | Done | time units, tempo map, bounded SPSC queues, RT worker pool |
| `bodo-kit-midi` | Done | MIDI 1.0 channel messages, Standard MIDI File read and write |

### Exit criteria

| Criterion | State |
|---|---|
| CI green on all 3 OSes | Met. Run 38367742899 passed on ubuntu-latest, macos-latest and windows-latest |
| Layer and license gates fail on deliberate violations | Met. The layers test expects every violation planted in its fixture, and `cargo xtask deny` proves on each run that the Kit policy rejects a planted GPL dependency |
| Tempo-map proptests pass | Met. `kit/bodo-kit-core/tests/tempo_map.rs` |

### Deferred

- **`#[rt_safe]` attribute** (project.md section 19.3): it needs a proc-macro crate, so it lands with the first audio-thread code in M1. Until then RT-safe functions carry the `RT-safe` doc line, and `kit/bodo-kit-core/tests/rt_safety.rs` runs them under `assert_no_alloc`.
- **OS audio workgroup and MMCSS for RT workers**: done in M1.
- **Denormal flushing on RT threads**: M1, with the first DSP.
- **rtsan job, fuzzing, benchmarks, agent evals**: nightly CI jobs, added as the code they cover lands.

## M1: Audio core (done, 2026-10-13)

Started and finished 2026-10-13. Open hardware checks: short exclusive-mode buffers on a real Windows sound card, and cpal on a real PulseAudio machine (see below).

| Item | State | Where |
|---|---|---|
| RT-safety tooling | Done | `#[rt_safe]` (`bodo-kit-macros`), RealtimeSanitizer CI job, denormal flushing ([ADR 0014](adr/0014-rt-safety-tooling-and-plan-handoff.md)) |
| Plan handoff | Done | `bodo_kit_core::handoff` |
| `bodo-kit-dsp` basics | Done | gain, pan laws, stereo, smoothing, delay lines, biquads and SVF, ADSR, interpolation, buffers |
| `bodo-kit-graph` | Done | incremental cycle checks, Kahn ordering, delay compensation, parallel executor ([ADR 0015](adr/0015-graph-executor.md)) |
| `bodo-kit-audio` | Done | decoding, WAV writing, resampling, peaks, disk streaming ([ADR 0016](adr/0016-disk-streaming-in-kit-audio.md)) |
| `bodo-kit-device` | Done | device lists, streams, latency, dropout counting, hot-plug detection, virtual device; ASIO builds in CI; real playback tested in CI on Windows (VB-CABLE) and Linux (PipeWire) |
| `bodo-model` v1 | Done | the project document and `Project::validate()` |
| `bodo-mix` | Done | realtime and offline engine, built-in sampler, sequencer, insert strips |
| WASAPI exclusive mode | Done | the "WASAPI Exclusive" host ([ADR 0017](adr/0017-wasapi-exclusive-mode.md)); see the Windows notes below |
| macOS audio workgroup | Done | RT workers join the device's IO `os_workgroup` through `Engine::set_thread_group`; checked on an Apple Silicon Mac |

### Exit criteria

| Criterion | State |
|---|---|
| 64 frames at 48 kHz without dropouts for 100 sampler voices | Met on an Apple Silicon Mac: 10 s, 7,503 callbacks, 0 dropouts, peak load 9% with 7 workers and 18% with none (`crates/bodo-mix/tests/stress.rs`, run by hand in release) |
| Zero allocations under rtsan | Met: every RT test suite runs under RealtimeSanitizer in CI, and the stress scenario ran clean under it locally |
| Offline renders bit-identical across runs | Met: `renders_are_bit_identical_for_any_workers_and_block_size` |

### Windows and Linux audio, as tested in CI

No Windows or Linux machine is available for testing yet, so CI installs a virtual sound card and plays through it:

- **Windows, shared mode:** real time with no dropouts, 43 ms output latency (2,062 frames at 48 kHz).
- **Windows, exclusive mode:** opens with 4 ms latency at the shortest period. VB-CABLE's own clock only ticks every 10 to 16 ms, so it cannot feed buffers shorter than that; at 1,024 frames (21 ms) exclusive mode runs in real time with no dropouts, and CI checks it there. Short buffers on a real sound card still need a check on a Windows machine.
- **Linux, PipeWire:** real time with no dropouts at every buffer size tried, down to 128 frames (5 ms).
- **Linux, PulseAudio:** cpal's ALSA backend stalls on PulseAudio's null sink at every buffer size, while `aplay` plays fine. Most current distributions run PipeWire; a check on a real PulseAudio machine is still open.

### Deferred to M4

- The 5 ms crossfade on graph changes, buffer reuse across nodes, and CPU time per node ([ADR 0015](adr/0015-graph-executor.md)).
- FX slots, strip EQ, send taps other than post-fader, sidechains, solo and negative track latency in the mixer strip.

## M2 to M9

Not started.
