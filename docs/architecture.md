# Architecture notes

[`project.md`](../project.md) is the specification and [`adr/`](adr/) holds the decisions. This page lists the approved exceptions to the layer rules and says where each rule is enforced.

## Layer exceptions

A crate may depend only on crates in strictly lower layers ([AGENTS.md section 2](../AGENTS.md#2-layers)). Any exception is listed here, with the ADR that approved it, and added to `xtask/src/layers.rs`.

There are no exceptions.

## Where the rules are enforced

| Rule | Enforced by | Details |
|---|---|---|
| Layers, Kit never depends on DAW crates, plugins depend on Kit only | `cargo xtask layers` | [ADR 0011](adr/0011-enforcing-the-layer-unsafe-and-license-rules.md) |
| `tokio`, `reqwest` and `rmcp` only in L8 and apps | `cargo xtask layers` | Checked through any chain of external dependencies |
| `egui` only in `bodo-kit-ui`, `bodo-ui` and apps | `cargo xtask layers` | Checked on direct dependencies |
| `unsafe` only in `ffi.rs` or `ffi/` of FFI crates, with `// SAFETY:` comments | `cargo xtask unsafe-audit` and the `unsafe_code` lint | FFI crates are listed in `xtask/src/unsafe_audit.rs` |
| No `unwrap`, `expect`, `panic!`, `todo!`, `dbg!` or printing outside tests | clippy, through the workspace lints | `clippy.toml` allows them in tests |
| Kit dependencies are permissive or MPL-2.0 | `cargo xtask deny` with `kit/deny.toml` | Run with every non-Kit crate excluded |
| DAW dependencies are GPL-3.0-compatible; bans, advisories, sources | `cargo xtask deny` with `deny.toml` | |
| Each crate's license matches its directory | `cargo xtask deny` | `kit/` is MIT OR Apache-2.0, everything else GPL-3.0-or-later |
| No em-dashes in code, comments or docs | `cargo xtask text` | |

## Crates so far

| Crate | Layer | Contents |
|---|---|---|
| `bodo-kit-macros` | macros | `#[rt_safe]` |
| `bodo-kit-core` | L0 | `Tick`, `Frames`, `Seconds`, `SampleRate`, `PPQ`; `TempoMap`; bounded SPSC queues; plan handoff; `RtPool` |
| `bodo-kit-midi` | L0 | MIDI 1.0 channel messages; Standard MIDI File read and write |
| `bodo-kit-dsp` | L1 | Gain, pan laws, smoothing, delay lines, filters, ADSR, buffers |
| `bodo-kit-audio` | L1 | Decoding, writing WAV, FLAC and Wave64, resampling, peaks, disk streaming |
| `bodo-kit-music` | L2 | Pitch classes, scales, chord symbols and chord naming, quantize, strum, arpeggio orders, humanizing |
| `bodo-kit-graph` | L2 | Incremental cycle checks, compilation with delay compensation, taps, crossfaded recompiles and shared buffers, the parallel executor with per-node timing |
| `bodo-kit-device` | L3 | Audio devices and streams, dropout counting, the virtual device with loopback, WASAPI exclusive mode, MIDI ports |
| `bodo-kit-ui` | L3 | Knob, fader, LED, step row, meter and line icons for egui |
| `bodo-model` | L5 | The project document, `Project::validate()`, sample path roots |
| `bodo-io` | L6 | `.bodo` files and folders: save, load, versions, backups, sample path rebasing; Standard MIDI File import and export |
| `bodo-mix` | L6 | The realtime and offline render engine: sampler, audio clips and time stretching ([ADR 0021](adr/0021-audio-clips-and-time-stretching.md)), sequencer with swing and the Song loop, automation ([ADR 0024](adr/0024-automation-swing-and-the-song-loop.md)), the mixer: strips as graph nodes with delay compensation, solo and crossfades ([ADR 0025](adr/0025-the-mixer-strip-as-graph-nodes.md)), native effects in FX slots ([ADR 0026](adr/0026-native-effects-in-fx-slots.md)), recording ([ADR 0022](adr/0022-recording-audio-and-midi.md)), loop recording and stems ([ADR 0027](adr/0027-recording-formats-loop-recording-and-stems.md)), live MIDI and the metronome ([ADR 0023](adr/0023-live-midi-input.md)) |
| `bodo-engine` | L7 | The command registry, the editing session, undo, templates, inspection ([ADR 0018](adr/0018-the-editing-session.md)) |
| `bodo-mcp` | L8 | The MCP server: generic and inspect tools, stdio, forwarding to the app |
| `bodo-control` | L8 | The loopback control channel with token access and discovery |
| `bodo-ui` | L8 | The main window, its panels and free windows, audio and automation |
| `bodo` | app | The desktop app |
| `bodo-cli` | app | `mcp`, `render`, `inspect`, `commands`, `pack`, `unpack` |
| `xtask` | tooling | The CI gates and the factory kit generator |

### Peers in the top layer

`bodo-mcp`, `bodo-control` and `bodo-ui` all sit in L8, so none of them depends on another. Each takes what it needs from a peer through a small interface, and the apps connect them: `bodo-ui` reads control requests through its `ControlSource` trait and answers `mcp.call` through an extension hook that `apps/bodo` fills with `bodo-mcp`, and `bodo-mcp` forwards tool calls through a function that `bodo-cli` backs with `bodo-control`'s client ([ADR 0019](adr/0019-agents-reach-the-app-through-the-control-channel.md)).

### The RT worker pool

`bodo_kit_core::RtPool` is the thread side of the parallel executor in [ADR 0003](adr/0003-realtime-engine-and-threading.md). The audio thread calls `run` once per block, which opens a parallel region: the caller and every worker that wakes in time call the job's `work`, and `run` returns once every worker that joined has left. The job shares out the work itself, through atomics. A worker that wakes after the region closed skips it, so a slow wakeup never delays a block. Workers spin for a configurable time after each region, then park, and `run` unparks only the parked ones. The dependency-counter scheduler from ADR 0003 is a job on this pool, in `bodo-kit-graph`.
