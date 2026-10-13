# ADR 0011: Enforcing the layer, unsafe and license rules

- **Status:** Proposed
- **Date:** 2026-10-13
- **Deciders:** hasan-muzak

## Context

AGENTS.md sections 2 and 5 and ADR 0002 state rules that CI must enforce: crate layers, where async and egui may appear, where `unsafe` may appear, and which licenses Kit and DAW crates may depend on. M0 builds the gates (`cargo xtask layers`, `unsafe-audit` and `deny`), and a machine check needs answers the prose leaves open:

- Do dev-dependencies count for layering?
- Who may depend on a stock plugin?
- Is "`tokio` only in L8" about direct dependencies or about anything reachable?
- cargo-deny applies one license policy to the whole graph, but Kit and the DAW need different ones.
- project.md section 3 says the workspace pins exact versions, but Kit crates will be published to crates.io.

## Decision

- **Every workspace crate has a place.** A crate is in a layer from the table in AGENTS.md, or is a plugin (`plugins/`), an app (`apps/`) or tooling (`xtask`). An unknown crate fails the check until it is added to the table. L0 to L4 crates live in `kit/` and L5 and up in `crates/`.
- **All dependency kinds count** for layering, dev-dependencies included. A test that needs a higher layer belongs in the higher crate.
- **Stock plugins depend on Kit (L0 to L4) only, and only apps depend on stock plugins.** No library crate and no other plugin may depend on a plugin.
- **Apps** may depend on any library and plugin, but not on another app. Nothing depends on `xtask`.
- **Async is checked transitively:** `tokio`, `reqwest` and `rmcp` may not be reachable from L0 to L7 or from plugins through any chain of normal dependencies on external crates.
- **egui is checked on direct dependencies:** only `bodo-kit-ui`, `bodo-ui` and apps name egui crates. Crates above them reach egui through them, as plugin editors do through `bodo-kit-ui`.
- **The unsafe boundary:**
  - Every crate inherits the workspace lints, which deny `unsafe_code`.
  - Lib and bin roots declare `#![forbid(unsafe_code)]`, unless the crate is an FFI crate with an `ffi` module.
  - FFI crates are the ones project.md section 4.1 names. Only they may put `#[allow(unsafe_code)]` on `mod ffi`, and unsafe code may appear only in their `ffi.rs` files and `ffi/` directories.
  - Every unsafe block and unsafe impl has a `// SAFETY:` comment on its line or in the comment lines directly above.
- **Two cargo-deny policies:**
  - `deny.toml` covers the whole workspace: GPL-3.0-compatible licenses, bans, advisories and sources.
  - `kit/deny.toml` allows only permissive licenses and MPL-2.0, and runs with every non-Kit crate excluded, which also drops the dependencies only those crates use.
  - Each run of `cargo xtask deny` first checks the licenses of the workspace's own crates, then proves on a fixture that the Kit policy rejects a GPL dependency that the workspace policy accepts in a DAW crate.
- **Gates are tested against planted violations.** Fixture workspaces under `xtask/fixtures/` break each rule on purpose, and the gates must report exactly those breaks.
- **Version pinning:** `[workspace.dependencies]` names full versions with Cargo's default caret requirements, and the committed `Cargo.lock` pins the exact versions every build uses. CI builds with `--locked`. Exact `=` requirements would carry into the manifests of published Kit crates and cause resolution conflicts for their users.

## Consequences

- Adding a crate means adding it to the layer table in `xtask/src/layers.rs` and AGENTS.md, and to `FFI_CRATES` in `xtask/src/unsafe_audit.rs` if it needs FFI.
- If the engine ever needs to link a stock plugin directly, for example the channel-rack sampler, that needs a new ADR, or the shared code moves into Kit.
- The transitive async check walks the dependency graph once per crate: O(V + E) each, a few milliseconds on this workspace.
- Unsafe code inside macro bodies is invisible to the audit, which parses source files. The `unsafe_code` lint still catches it once expanded.
- `cargo xtask deny` needs cargo-deny installed (`cargo install cargo-deny --locked`), and its advisory step needs network access.

## Alternatives considered

- **One cargo-deny config with per-crate license exceptions:** exceptions apply to a dependency wherever it appears, so a GPL crate allowed for the DAW would also pass inside Kit.
- **Checking async only on direct dependencies:** a small wrapper crate would hide `tokio` from the check.
- **Ignoring dev-dependencies:** simpler for tests, but upward test dependencies blur the layer graph that the rules exist to keep clear.
- **`=` version requirements:** they pin the workspace build, which the lockfile already does, at the cost of every downstream user of Kit.
