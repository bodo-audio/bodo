# ADR 0017: WASAPI exclusive mode as its own host

- **Status:** Accepted
- **Date:** 2026-10-13
- **Deciders:** hasan-muzak

## Context

Many Windows users make music on a laptop's built-in sound card, with no ASIO driver. FL Studio covers them with its own ASIO driver, which talks to the card directly. Through cpal, Bodo only gets WASAPI shared mode, where Windows mixes every program's audio first. On the CI runner's virtual sound card, shared mode measured 2,062 frames of output latency at 48 kHz, about 43 ms. That is too slow for playing an instrument live (project.md section 6.2).

WASAPI exclusive mode gives one program the card to itself and skips the Windows mixer, so latency comes down to the buffer size. cpal 0.18 does not offer it. project.md section 3 already names the `wasapi` crate (MIT) for this.

## Decision

- `bodo-kit-device` depends on `wasapi` 0.26 on Windows only, and adds a host named **"WASAPI Exclusive"** next to cpal's hosts. `hosts`, `devices` and `open` route to it by name, so the rest of Bodo treats it like any other host.
- A dedicated audio thread opens the device, because WASAPI objects belong to the thread that made them. It runs in event mode: wait for the device's event, render one buffer through the shared `Driver`, convert it, write it. The thread asks MMCSS for real-time priority.
- The format is negotiated best first: 32-bit float, 32-bit integer, 24-bit in 32, packed 24-bit, then 16-bit. It is tried at the engine's channel count, then at the device's own if that is larger.
- The buffer is the requested size, or the device's shortest period, aligned to 128 bytes for Intel HD Audio. When the device rejects the size, Bodo takes the one it proposes.
- Output only for now. Recording in exclusive mode comes with recording, in M3.
- A CI job plays a tone through exclusive mode on a virtual sound card (VB-CABLE) on a Windows runner, so the code runs on real Windows audio paths without a Windows machine.

## Consequences

- Built-in sound cards on Windows get latency close to an ASIO driver's, without installing anything.
- While Bodo runs in exclusive mode, other programs can't play through that card. The UI must say so when the user picks this host, and "the device is busy" becomes a normal error to show.
- Sample-rate conversion is gone: the card must support the project's rate in exclusive mode, or opening fails with a clear message.
- About 300 lines of Windows-only code that Bodo maintains. It is cross-checked with clippy from other platforms and tested on the Windows runner.

## Alternatives considered

- **Writing our own ASIO driver**, like FL Studio: a kernel-adjacent driver to sign, ship and support for every card. Far more work for the same result as exclusive mode.
- **Waiting for cpal to add exclusive mode:** no date, and it would block a feature Windows users expect.
- **`rtaudio` bindings:** a C++ dependency and build, for the one mode the `wasapi` crate already gives us in Rust.
