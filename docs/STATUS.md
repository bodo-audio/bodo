# Status

Progress per milestone ([project.md section 20](../project.md#20-roadmap)). Update this file at every milestone.

## M0: Foundations (in progress)

| Item | State | Where |
|---|---|---|
| Workspace, toolchain, lints | Done | `Cargo.toml`, `rust-toolchain.toml` (Rust 1.98.1), `rustfmt.toml`, `clippy.toml` |
| `cargo xtask ci` | Done | fmt, clippy (default and all features), tests, docs, release build, then every check below |
| `cargo xtask layers` | Done | `xtask/src/layers.rs`; fixture `xtask/fixtures/layers` |
| `cargo xtask unsafe-audit` | Done | `xtask/src/unsafe_audit.rs` |
| `cargo xtask deny` | Done | `deny.toml`, `kit/deny.toml`; fixture `xtask/fixtures/licenses` |
| `cargo xtask text` | Done | `xtask/src/text.rs` |
| CI on Linux, macOS and Windows | Workflow written, never run | `.github/workflows/ci.yml`; the repository has no GitHub remote yet |
| Docs skeleton | Done | this file, [`architecture.md`](architecture.md), [`agents/`](agents/), [`formats/`](formats/) |
| `bodo-kit-core` | Done | time units, tempo map, bounded SPSC queues, RT worker pool |
| `bodo-kit-midi` | Done | MIDI 1.0 channel messages, Standard MIDI File read and write |

### Exit criteria

| Criterion | State |
|---|---|
| CI green on all 3 OSes | Open. `cargo xtask ci` passes locally on macOS (Apple Silicon); the workflow runs once the repository is on GitHub |
| Layer and license gates fail on deliberate violations | Met. The layers test expects every violation planted in its fixture, and `cargo xtask deny` proves on each run that the Kit policy rejects a planted GPL dependency |
| Tempo-map proptests pass | Met. `kit/bodo-kit-core/tests/tempo_map.rs` |

### Deferred

- **`#[rt_safe]` attribute** (project.md section 19.3): it needs a proc-macro crate, so it lands with the first audio-thread code in M1. Until then RT-safe functions carry the `RT-safe` doc line, and `kit/bodo-kit-core/tests/rt_safety.rs` runs them under `assert_no_alloc`.
- **OS audio workgroup and MMCSS for RT workers**: they need the device layer (M1). Workers can already be promoted to real-time priority.
- **Denormal flushing on RT threads**: M1, with the first DSP.
- **rtsan job, fuzzing, benchmarks, agent evals**: nightly CI jobs, added as the code they cover lands.

## M1 to M9

Not started.
