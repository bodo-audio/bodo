# ADR 0002: Kit is MIT OR Apache-2.0, the DAW is GPL-3.0-or-later

- **Status:** Accepted
- **Date:** 2026-10-13
- **Deciders:** hasan-muzak

## Context

Bodo is free and open source, and it earns money only from the Bodo Cloud AI subscription. Bodo Kit is meant to become a JUCE-like framework that plugin developers can adopt, including companies that ship closed-source or commercial plugins. The DAW, on the other hand, must not be forked into a closed commercial product.

Licensing facts as of October 2026:

- The VST3 SDK is MIT (since 2025-10-29).
- The ASIO SDK is dual licensed, GPLv3 or proprietary.
- VST2 can't be licensed.
- Key dependencies are MIT/Apache, with a few MPL-2.0 crates (`symphonia`, `imbl`, `triple_buffer`, `audio_thread_priority`).

## Decision

| Code | License |
|---|---|
| `kit/` (Bodo Kit) and its examples | MIT OR Apache-2.0 |
| `crates/`, `plugins/`, `apps/`, `xtask/` | GPL-3.0-or-later |
| Marketplace backend (`services/market`) | AGPL-3.0 (proposed) |
| Bodo Cloud gateway | Open question |

- **Kit dependencies:** Kit crates may depend only on permissive or MPL-2.0 crates. `cargo-deny` enforces this, and CI fails on any violation.
- **ASIO:** Kit's `asio` feature is off by default. GPL DAW builds enable it on Windows.
- **No VST2** anywhere.

## Consequences

- Plugin developers can ship proprietary plugins built on Kit, which encourages adoption.
- Forks of the DAW must stay GPL, which protects the "free forever" promise.
- Stock plugins are GPL. Anyone can build them on top of Kit, but redistributing them requires the GPL.
- Contributors need to know which license applies to the crate they are changing. Every crate states it in `Cargo.toml` and in a `LICENSE` file.

## Alternatives considered

- **Everything MIT/Apache:** companies could close-fork the DAW.
- **Everything GPL:** commercial plugin developers would avoid Kit.
- **DAW under AGPL:** the extra network clause adds little for a desktop app.
