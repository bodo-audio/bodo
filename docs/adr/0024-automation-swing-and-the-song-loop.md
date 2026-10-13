# ADR 0024: Playing automation, swing and the Song loop

- **Status:** Accepted
- **Date:** 2026-10-13
- **Deciders:** hasan-muzak

## Context

Automation clips and pattern lanes ([project.md section 10.5](../../project.md#105-automation)) change parameters over time, swing delays every second step ([section 10.2](../../project.md#102-channel-rack)), and Song mode can loop a region of the playlist. Automated values must reach processors on the audio thread without races with the knobs the user is turning, and a loop must wrap on the exact frame.

## Decision

- **A table of automated values owned by the audio thread.** `BlockContext.params` is a sorted `Vec<(u64, f32)>` of at most 1024 entries, allocated once. At the start of each block, `automation::apply` evaluates the `AutomationIndex` (built with the song index) and writes the values, keyed by node and parameter. The strip and the sampler read the table first and fall back to their live settings; the control thread never writes it. The existing per-block gain ramps smooth the steps. The table is cleared on stop, seek and loop wrap, so parameters return to the project's values.
- **Normalized values.** Points hold 0 to 1; `bodo-model` maps them to each parameter: a fader curve for volume (0.8 is 0 dB), -1 to 1 for pan, plus or minus 2400 cents for pitch, 10 to 999 BPM for tempo.
- **Swing when the index is built.** Notes on odd sixteenth steps move later by swing times half a step. The song index is rebuilt when a channel's swing or automation changes, which a per-channel check detects cheaply.
- **The loop cuts the block.** In Song mode with a loop, the block that reaches the loop's end stops exactly there, and the next block starts at the loop's start as a jump: notes are released and automated values cleared.

## Consequences

- Automation is evaluated once per block, at its start, so a change lands up to one block late (5.3 ms at 256 frames and 48 kHz), and a fast ramp moves in block-sized steps smoothed by the gain ramps. Project.md asks for sample accuracy; evaluating at both ends of the block and ramping between them, or splitting blocks at points, is follow-up work.
- Route, FX slot, plugin, CC, pitch bend, pressure and tempo lanes are stored and edited but not yet heard; they arrive with their processors in M4 to M6.

## Alternatives considered

- **Writing automated values into the processors' atomics from the control thread:** races with the user's knob, and the control thread's timing isn't sample-related.
- **Per-sample evaluation:** exact, but costs a lookup per sample and parameter for little audible gain at today's block sizes.
