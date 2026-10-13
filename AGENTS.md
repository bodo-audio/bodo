# AGENTS.md

These are the rules for anyone, human or coding agent, who changes code in this repository. The full specification is in [`project.md`](project.md), and decisions are recorded in [`docs/adr/`](docs/adr/).

> **Current state:** milestone M0 ([project.md section 20](project.md#20-roadmap)) is in progress. The workspace, the `xtask` gates, the CI workflow, `bodo-kit-core` and `bodo-kit-midi` exist; [`docs/STATUS.md`](docs/STATUS.md) tracks the rest. Commands below marked with a later milestone don't work yet.

## 1. Repository map

| Path | What | License |
|---|---|---|
| `kit/bodo-kit-*` | Bodo Kit, the JUCE-like framework | MIT OR Apache-2.0 |
| `crates/bodo-*` | The DAW's libraries | GPL-3.0-or-later |
| `plugins/*` | Stock instruments and effects (built on Kit only) | GPL-3.0-or-later |
| `apps/*` | `bodo`, `bodo-hub`, `bodo-cli`, `bodo-plugin-bridge` | GPL-3.0-or-later |
| `xtask/` | Build tooling (`cargo xtask ...`) | GPL-3.0-or-later |
| `docs/` | ADRs, agent guides, format specs, `STATUS.md` | Repository default (GPL-3.0-or-later) |

## 2. Layers

A crate may depend only on crates in **strictly lower** layers. `cargo xtask layers` enforces this; [ADR 0011](docs/adr/0011-enforcing-the-layer-unsafe-and-license-rules.md) spells out the details, and [`docs/architecture.md`](docs/architecture.md) lists the exceptions.

| Layer | Crates |
|---|---|
| macros | `bodo-kit-macros` (proc macros: any crate may use them, and they use no workspace crate) |
| L0 | `bodo-kit-core`, `bodo-kit-midi` |
| L1 | `bodo-kit-dsp`, `bodo-kit-audio` |
| L2 | `bodo-kit-analysis`, `bodo-kit-music`, `bodo-kit-graph` |
| L3 | `bodo-kit-plugin`, `bodo-kit-host`, `bodo-kit-device`, `bodo-kit-ui` |
| L4 | `bodo-kit-plugin-clap`, `-vst3`, `-au`, `-standalone` |
| L5 | `bodo-model` |
| L6 | `bodo-io`, `bodo-mix` |
| L7 | `bodo-engine` |
| L8 | `bodo-llm`, `bodo-mcp`, `bodo-agent`, `bodo-control`, `bodo-market`, `bodo-ui` |

Hard rules:

- **Kit never depends on DAW crates.** This is a license boundary.
- **`tokio`, `reqwest` and `rmcp` only in L8 and apps.** L0 to L7 are synchronous.
- **`egui` only in** `bodo-kit-ui`, `bodo-ui` and apps.
- **Stock plugins** depend on Kit (L0 to L4) only.

## 3. Build and test

```sh
cargo xtask ci            # everything CI runs: fmt, clippy, tests, docs, release build, layers, unsafe audit, text, deny
cargo xtask layers        # layer rules only
cargo xtask unsafe-audit  # unsafe-code boundary only
cargo xtask text          # text style only
cargo xtask deny          # license and advisory policy (needs `cargo install cargo-deny --locked`)
cargo test -p <crate>     # one crate
cargo xtask kit new <name> --kind effect|instrument   # scaffold a Bodo Kit plugin (M6)
```

A change is done only when `cargo xtask ci` passes locally. Golden renders join `ci` when the first DSP lands.

## 4. Real-time rules (audio thread and RT workers)

Code that runs on the audio thread must never:

- allocate or free memory (`Vec::push` past capacity, `Box::new`, `String`, `format!`, dropping the last `Arc`);
- take a lock (`Mutex`, `RwLock`), or block or wait on anything except the executor's own wait primitive;
- do I/O, make syscalls, or log directly (log through the RT log ring);
- panic, or call code that can panic.

Do this instead:

- Use `rtrb` queues, atomics, `triple_buffer`, `arc-swap` loads and `basedrop` handles.
- Preallocate in `activate` / `prepare`, never in `process`.
- Send retired objects to the GC thread.
- Address parameters, nodes and buffers by integer id, never by string.
- Mark RT functions with `#[rt_safe]` and document them with `/// RT-safe`.

Debug builds catch allocations through `assert_no_alloc`, and a CI job runs `rtsan`.

## 5. Code policy

- **`unsafe`:** non-FFI crates declare `#![forbid(unsafe_code)]`. FFI code lives only in `ffi.rs` or `ffi/`, the only place where `#[allow(unsafe_code)]` is allowed. Every `unsafe` block has a `// SAFETY:` comment explaining why it is sound.
- **No `unwrap`, `expect`, `panic!`, `todo!`, `unimplemented!`, `dbg!` or `print!`** outside tests. Clippy denies them.
- **Errors:** `thiserror` in libraries, `anyhow` in apps only. Never `Box<dyn Error>` in public APIs.
- **Logging:** `tracing`.
- **Formatting:** `rustfmt` with `max_width = 150`.
- **Docs:** every public item is documented.
- **Text style:** no em-dashes in code, comments or docs; CI checks this.

## 6. Complexity rule

- Every public function that is not O(1) states its cost: `/// Complexity: O(log n) in the number of clips on the track.`
- Hot paths must be O(n log n) or better. Hot paths are:
  - the audio thread;
  - UI frame building;
  - loops over every note or clip;
  - per-block sequencing;
  - graph compilation;
  - command application.
- O(n^2) or worse on a hot path needs an ADR and a criterion benchmark with a regression threshold.
- Prefer sorted `Vec` with binary search, interval indexes, `imbl` persistent maps and incremental algorithms to recomputing everything.
- Every queue on an RT path is bounded and has an explicit overflow policy.

## 7. Everything is a command

Every project mutation goes through `bodo-engine`'s `execute(id, params)`. That includes the UI, shortcuts, the palette, MCP and the assistant. The UI never owns document data.

### Adding a command (or MCP tool)

1. Define the params struct with `serde` + `schemars::JsonSchema` (camelCase fields; empty params mean "act on the current selection").
2. Register a `CommandSpec`: a namespaced id (`mixer.set_pan`), a one-line `summary`, `effects` (undoable, destructive, disk, network), and a `merge_key` if the command is driven by gestures.
3. Implement `apply` as snapshot in, snapshot out. Never mutate in place.
4. Add a proptest showing `Project::validate()` still holds after the command, plus an `insta` snapshot of its result JSON.
5. MCP exposes the command automatically. If it belongs in the `small` tool profile, add it there.
6. Document it in `docs/agents/commands.md`.

### Adding a stock plugin

1. Run `cargo xtask kit new <name>` and use only the public Bodo Kit API.
2. Give parameters stable integer ids, which are never reused. Give them units and ranges, and `tags` for the assistant.
3. Add golden renders (bit-exact) and a null test at the default settings where that applies.
4. Bundles must pass clap-validator, pluginval and auval in CI.

### Adding a model field

1. Add the field with a serde default.
2. Bump `format_version` only if older files need a migration, and write the migration in `bodo-io`.
3. Add a fixture project to the migration corpus.

## 8. Testing definition of done

- Unit tests, and proptests where invariants exist.
- `insta` snapshots for command and MCP output.
- Golden audio renders for any change to DSP or the mixer.
- `egui_kittest` snapshots for any UI change, in both dark and light themes.
- Benchmarks for any hot-path change.
- An agent-task test (driven only through MCP) for any new user-facing workflow.

## 9. Architecture decisions

Any architectural change gets an ADR in `docs/adr/`, copied from `0000-template.md` and numbered sequentially. Never rewrite an accepted ADR. Write a new one that supersedes it.

## 10. Commits

- Use **Conventional Commits**: `feat(mixer): add sidechain routes`, `fix(engine): ...`, `docs: ...`, `chore: ...`.
- Keep each commit to one logical change, and commit after each completed step.
- **Commits carry human authorship only.** Never add AI co-author trailers (`Co-Authored-By: <AI tool>`), and don't mention AI assistants or tools in commit messages.

## 11. Clean-room rule

Bodo's workflow is inspired by FL Studio. Contributors must not:

- decompile, disassemble or extract data from any commercial product;
- copy proprietary names, skins, icons, graphics or factory content;
- paste code under licenses incompatible with the target crate (Kit: permissive or MPL-2.0 only; DAW: GPL-3.0-compatible).

Learn from public manuals and observed behavior only.
