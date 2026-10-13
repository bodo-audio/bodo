# ADR 0001: GUI stack is egui + craft-ui on wgpu

- **Status:** Accepted
- **Date:** 2026-10-13
- **Deciders:** hasan-muzak

## Context

Bodo needs a modern UI that looks current for 2026, with heavy custom views: thousands of notes, waveforms, 100+ mixer strips and meters running at 60 to 120 fps. Bodo Kit plugins need the same toolkit inside third-party hosts, where they open in a parent window handle.

The sibling projects pixel3d and soundcraft already use egui 0.36 + `craft-ui` (storytold/craft-libs). That stack brings a token-based design system, docking, a command palette, hand-painted widgets, Tabler path icons and `egui_kittest` snapshot tests. We evaluated four alternatives as of October 2026:

- **gpui (Zed):** the fastest option. Its crates.io release has been frozen since October 2025, so we would need a pinned fork, and it is hard to embed in plugin hosts.
- **iced 0.14:** used by several Rust DAWs. We would have to rebuild the design system and the docking.
- **A custom wgpu + Vello toolkit:** the approach Meadowlark took with Yarrow. It would add many months before any DAW work could start.
- **vizia and slint:** vizia moves slowly, and slint is licensed under GPL or commercial terms.

## Decision

- **Toolkit:** `egui`, `eframe` and `egui-wgpu` 0.36.x with `wgpu` 30, plus `craft-ui` for tokens, docking and the base widgets.
- **Panel code** uses tokens only, never literal colors, and gets the pixel3d look by default.
- **Heavy canvases** use culled, batched egui meshes or instanced `egui_wgpu::CallbackTrait` passes. These are the playlist, the piano roll, waveforms, meters and the spectrum. No frame ever iterates over every item in the project.
- **`bodo-kit-ui`** holds the DAW and plugin widgets (knob, fader, meter, waveform, XY pad, keyboard, envelope). Plugin editors open through `baseview` with an egui integration.
- **UI tests** are `egui_kittest` snapshots in both dark and light themes.

## Consequences

- **What we gain:**
  - we reuse the user's existing design language and code right away;
  - one toolkit serves the DAW, the Hub and plugin editors;
  - accessibility comes through accesskit.
- **Costs:**
  - immediate-mode rendering redraws every frame, so we need a repaint-on-demand policy;
  - complex layouts need care;
  - text shaping is less advanced than in retained-mode toolkits.
- **Perf gates:**
  - UI CPU stays within 4 ms per frame at 120 Hz on M1-class hardware with 10k visible notes;
  - criterion and kittest frame-time tests guard this in CI.
- **We revisit this ADR** if a view can't meet its frame budget after moving to a wgpu callback. Because `bodo-ui` is isolated from the engine, any one view can be replaced without touching the rest.

## Alternatives considered

gpui (fork risk, poor fit for plugin UIs), iced (we would redo the design system), a custom wgpu toolkit (time to first DAW), vizia (pace), slint (licensing).
