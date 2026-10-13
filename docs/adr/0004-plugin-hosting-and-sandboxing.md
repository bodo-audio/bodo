# ADR 0004: Plugin hosting, sandboxing, and no VST2

- **Status:** Accepted
- **Date:** 2026-10-13
- **Deciders:** hasan-muzak

## Context

Users expect VST3 and CLAP on every platform, AU on macOS and LV2 on Linux. A third-party plugin crashing must not take the session down with it: soundcraft runs everything in-process and lists sandboxing as a gap. Facts we have to work within:

- **Licensing:** VST2 can't be licensed, and VST3 has been MIT since October 2025.
- **GUIs across processes:** embedding works on Windows (`SetParent`) and on X11 (reparenting), but not on macOS or Wayland.
- **AUv3** has to be instantiated asynchronously.
- **macOS signing:** signed macOS apps need `disable-library-validation` before they can load third-party plugins.

## Decision

- **Formats and bindings:**
  - CLAP through `clack-host`;
  - VST3 through the `vst3` bindings (coupler-rs);
  - AUv2 and AUv3 through `objc2-audio-toolbox` and `objc2-avf-audio`;
  - LV2 through `livi`.
  - All of them sit behind `bodo-kit-host::HostedPlugin`.
- **Stock plugins** are linked natively, in-process, with no format layer.
- **No VST2.**
- **Scanning** happens out of process in `bodo-plugin-bridge --scan`, one plugin per child process with a timeout. Results are cached by path, mtime, size and hash, and plugins that crash go on a blocklist.
- **Sandbox modes,** chosen per instance:
  - in-process;
  - shared bridge (the default for third-party plugins);
  - per vendor;
  - per plugin.
- **The bridge protocol:**
  - shared-memory audio and event buffers;
  - one synchronous round trip per bridge per block, woken by futex, `os_sync_wait_on_address` or Mach semaphores, or Win32 events;
  - a budget for each block, after which the output is silence and an xrun is counted;
  - a separate non-RT control channel for state, parameter info and editors.
- **After a crash:** the slot shows "Plugin crashed, Reload" and restores the last saved state. Everything else keeps playing.
- **Editors:**
  - in-process plugins get floating windows owned by the main window, which passes them a native handle;
  - bridged plugins embed on Windows and X11, and get their own floating windows on macOS and Wayland.
- **macOS entitlements:**
  - the main app gets `disable-library-validation` and `audio-input`;
  - JIT and unsigned-executable-memory entitlements go only on the bridge binary.

## Consequences

- Third-party crashes can't take Bodo down in bridged modes.
- Bridging costs one context switch per bridge per block. It is batched per bridge, so the cost stays at tens of microseconds.
- On macOS and Wayland, bridged editors float instead of embedding. Bitwig makes the same trade-off.
- We maintain an IPC protocol, and we fuzz it.

## Alternatives considered

- **In-process only:** simple, but fragile.
- **A process per plugin as the only mode:** too heavy for large sessions.
- **Wrapping plugins with C++ `clap-wrapper` or JUCE:** breaks the all-Rust rule.
- **VST2 through reverse-engineered headers:** legally risky, and open-source hosts have received takedown notices.
