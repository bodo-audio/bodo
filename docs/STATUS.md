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

## M2: Shell (done on macOS, 2026-10-13)

Started and finished 2026-10-13. Every check passes on macOS; CI on Linux and Windows runs once these commits are pushed.

| Item | State | Where |
|---|---|---|
| `bodo-engine` | Done | command registry, editing session, undo with gestures, templates, inspection ([ADR 0018](adr/0018-the-editing-session.md)); 67 commands in [`agents/commands.md`](agents/commands.md) |
| `.bodo` save and load | Done | `bodo-io`: zip and `.bodoproj` folders, versions and migrations, atomic saves, 10 rotating backups, sample path rebasing ([format](formats/bodo.md)) |
| Autosave and recovery | Done | every 5 minutes (Options > Autosave), offered back after a crash |
| `bodo-ui` | Done | the owner's layout: top bar, Browser, Channel Rack, Playlist, Piano Roll and Mixer as free windows, layouts, dark and light themes, command palette |
| Transport | Done | play, stop, Pattern and Song modes, song position, tempo, master volume and level |
| Basic MCP server | Done | `bodo-mcp` and `bodo-cli mcp` with generic and inspect tools; `--connect` drives the running app ([ADR 0019](adr/0019-agents-reach-the-app-through-the-control-channel.md)) |
| Control channel | Done | `bodo-control`: token-protected loopback channel, on by default |
| Factory content | Done | the Bodo Drums kit, generated and CC0, and one-shot samplers ([ADR 0020](adr/0020-factory-content-and-one-shot-samplers.md)) |
| `bodo-kit-ui` | Done | knob, fader, LED, step row, meter, line icons |
| `bodo-cli` | Done | `mcp`, `render`, `inspect`, `commands`, `pack`, `unpack` |

### Exit criteria

| Criterion | State |
|---|---|
| Build a 4-pattern beat with the step sequencer and sampler, arrange it, save and reopen it identically | Met. By clicking: `crates/bodo-ui/tests/beat.rs`; through commands: `crates/bodo-engine/tests/beat.rs`; it renders to audio where the steps are: `apps/bodo-cli/tests/render.rs` |
| An external MCP client can build the same beat | Met. `crates/bodo-mcp/tests/server.rs` drives the real MCP server through an MCP client only, saves, reopens and compares |

### Deferred

- **Detaching windows** into their own OS windows (ADR 0012), and the native macOS menu.
- **Sample preview** in the browser, and drag and drop onto the playlist.
- **Swing** is stored but not yet played; the sequencer applies it in M3.
- **Editing depth (M3):** the playlist's paint, slip, slice, select and zoom tools, piano-roll lanes and velocity, custom layouts, editable shortcuts.
- **The full mixer (M4):** the insert panel with FX slots, meters on every insert, solo.
- **MCP:** tool profiles, the streamable HTTP transport and limits on where agents may read and write files come with the assistant (M7).
- **The missing-file resolver** that finds a moved sample by its hash.
- **Opening a project in the running app** from the Hub or a second launch.
- **Headroom:** the master has no limiter yet, so stacking many hits can still go over 0 dBFS.

## M3 to M9

Not started.
