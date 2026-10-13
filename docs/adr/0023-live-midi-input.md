# ADR 0023: Live MIDI input, the metronome and count-in

- **Status:** Accepted
- **Date:** 2026-10-13
- **Deciders:** hasan-muzak

## Context

M3 adds MIDI devices ([project.md section 20](../../project.md#20-roadmap)): a keyboard plays the channel being edited, a channel can listen to a particular port and MIDI channel, and playing and recording need a metronome. MIDI arrives on the driver's thread, the audio thread may not lock or allocate, and a flood of controller data must not stall either.

## Decision

- **Ports through `midir`,** wrapped in `bodo_kit_device::midi`: listing, input with a callback, output, and decoding to `MidiEvent`.
- **A router on the MIDI side.** `MidiRouter` maps a port and MIDI channel to the channels bound with `channel.set_midi_in`; unbound input plays the selected channel. Its table is behind a lock that only the MIDI and control threads take; the control thread updates it when the project or the selection changes.
- **A bounded live queue to the audio thread.** `LiveInput` carries note on, note off and all-off, 1024 events deep; when it is full, new events are dropped and counted. The audio thread takes the queued events at the start of each block and plays them with its own voice ids, so they never collide with the sequencer's.
- **The metronome is part of the engine.** It clicks on exact beat frames through tempo changes, higher on the downbeat, and is mixed after the master fader. `transport.metronome` and `transport.count_in` switch it and the count-in, without undo steps.

## Consequences

- Playing a keyboard never touches a lock on the audio thread and can't block it.
- Live notes play at the start of the next block, so their timing is quantized to the buffer; mapping MIDI timestamps to frames is follow-up work.
- Only notes are routed for now: control changes, pitch bend and pressure come with the plugin host in M5 and M6, and MIDI output exists but nothing sends to it yet.

## Alternatives considered

- **Routing on the audio thread:** the binding table changes with the project, so it would need a handoff of its own for little gain.
- **An unbounded channel:** a stuck audio thread would let it grow without limit.
