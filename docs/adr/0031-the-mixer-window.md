# ADR 0031: The mixer window

- **Status:** Accepted
- **Date:** 2026-10-13
- **Deciders:** hasan-muzak

## Context

M4 completes the mixer ([project.md section 9.7](../../project.md#97-mixer-ui)): strips with every control of section 9.2, the route control, the insert panel with ten FX slots and the strip EQ, an optional routing graph, group editing, effect editors, plugin presets, and exporting the mix and the stems. The engine side ([ADR 0025](0025-the-mixer-strip-as-graph-nodes.md) to [0027](0027-recording-formats-loop-recording-and-stems.md)) already exposes the commands, the effect catalog and lock-free meters. The window must stay fast at 256 inserts, never own document data, and be testable headlessly, where no audio device runs.

## Decision

- **One module per part of the window:** `mixer/` holds the window and its state, the strips, the insert panel, the EQ and the routing graph. Every edit goes through a command; the window keeps only view state (strip size, panel and graph toggles, the chosen slot, meters as shown).
- **Three strip sizes.** Wide shows every control; Compact moves stereo separation, polarity and swap to the insert panel; Extra compact keeps the name, mute, solo, the fader with its meter and the route control. A dB scale down the left lines up with the faders.
- **Meters are polled each frame** from the engine (`Audio::meter`, which clears the peaks it read) and fall back by half every 150 ms in the window. The clip light is the engine's latch; clicking it clears the latch. CPU use per insert refreshes four times a second. Without an engine, `Audio::set_readings` supplies meters, CPU, the oscilloscope and latency, so tests and screenshots see the real drawing code.
- **Selection is the editor's `inserts` set.** A click selects one strip, Shift or Cmd/Ctrl click adds or removes one. Dragging a fader of a selected group, or any fader with Alt, sends the change as `mixer.adjust` with a relative `volumeDb`, so every selected fader moves by the same amount, in one undo step through the gesture.
- **The route control is relative to the selected insert, as in FL:** a strip's control shows whether the selected insert routes into it. A click adds or removes the route, a drag sets its send level (one undo step), Cmd/Ctrl click adds a sidechain-only route. On the master the control is the selected insert's `to_master`.
- **The insert panel** shows the selected insert, or the master. Slots take effects from a picker, from the Browser by drag and drop, or from presets; dragging a slot onto another moves it (`fx.move`); double-clicking opens its editor. The sandbox mode is stored per slot by a new `fx.set_sandbox`; native effects always run inside Bodo, and the setting takes effect for plugins in M5.
- **The EQ curve is the engine's own response:** the panel designs the three bands with the same `BiquadCoeffs::design` call as the strip and draws their summed magnitude at the project's rate, so what is drawn is what plays.
- **The routing graph** draws channels, then inserts in columns by the longest chain of routes leading to them, then the master: O(V + E). Audio routes are solid with their level as width, sidechains dashed.
- **Effect editors are free windows keyed by plugin instance,** so moving a slot keeps its editor open, and they detach like the others. Native effects get a generic editor generated from their `ParamSpec`s: knobs over the normalized range with the parameter's own display string, switches for toggles, menus for choices. Every parameter offers "Create automation clip".
- **Plugin presets** are `.bodopreset` files (`bodo.plugin` format) in the library's Plugin presets folder, one subfolder per effect, through `preset.save_plugin` and `preset.load_plugin`. Loading onto the same effect keeps its instance, so automation and editors stay attached.
- **Exports run through commands.** File > Export mix and Export stems open a dialog (what, range, format, folder) that runs `render.export_mix` or `render.export_stems`; both now take a `start`, so the loop region or the time selection can be exported. `ui.export` opens the dialog for agents.

## Consequences

- The window costs O(i) per frame in the strips drawn, and each strip reads its meter with a few atomic loads.
- An export renders on the interface thread: a long song freezes the window until it is written. Moving renders to a background job with progress is follow-up work.
- Plugins in other formats show their name and pass audio through until hosting arrives in M5; their own editors open then.
- Readings injected for tests never mix with a running engine's: the engine's always win.

## Alternatives considered

- **A separate selection model for the mixer:** the session's editor state already holds selected inserts, and agents see the same selection.
- **Absolute fader values for group moves:** each strip would need its own command per frame; one relative `mixer.adjust` keeps one undo step and the group's offsets.
- **Drawing the EQ curve from an approximation:** cheaper, but it would drift from what the engine plays at high gains and narrow Qs.
- **A full plugin editor framework now:** M5 and M6 bring hosted and Kit editors; a generic editor from parameter descriptions covers native effects today and stays the fallback later.
