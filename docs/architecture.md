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
| `bodo-kit-core` | L0 | `Tick`, `Frames`, `Seconds`, `SampleRate`, `PPQ`; `TempoMap`; bounded SPSC queues; `RtPool` |
| `bodo-kit-midi` | L0 | MIDI 1.0 channel messages; Standard MIDI File read and write |
| `xtask` | tooling | The CI gates |

### The RT worker pool

`bodo_kit_core::RtPool` is the thread side of the parallel executor in [ADR 0003](adr/0003-realtime-engine-and-threading.md). The audio thread calls `run` once per block, which opens a parallel region: the caller and every worker that wakes in time call the job's `work`, and `run` returns once every worker that joined has left. The job shares out the work itself, through atomics. A worker that wakes after the region closed skips it, so a slow wakeup never delays a block. Workers spin for a configurable time after each region, then park, and `run` unparks only the parked ones. The dependency-counter scheduler from ADR 0003 will be a job on this pool, in `bodo-kit-graph` (M1).
