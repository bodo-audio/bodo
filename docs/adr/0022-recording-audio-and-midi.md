# ADR 0022: Recording audio and MIDI

- **Status:** Accepted
- **Date:** 2026-10-13
- **Deciders:** hasan-muzak

## Context

M3's exit criterion asks that recorded audio land within 1 ms of where it was played, after latency compensation ([project.md section 20](../../project.md#20-roadmap)). Recording writes to disk, which the audio thread may never do, and a recording must survive a slow disk without glitching playback. MIDI recording has the same shape: events arrive on the audio thread and must become notes of the pattern, as one undo step.

## Decision

- **Armed inserts record.** An insert has an input (`mixer.set_input`, mono or a stereo pair of device inputs), an arm switch (`mixer.arm`), a tap (input, pre-FX, post-FX or post-fader) and a format. The app opens the device inputs that armed inserts need, falling back to output only when the device can't provide them, and reopens the stream when that changes, never during a recording.
- **A ring per armed insert, a disk thread behind it.** `transport.record` hands each armed strip a fixed-size SPSC ring through `bodo_kit_core::handoff`. The strip copies its tap into the ring; a full ring drops the whole block and counts it, so playback never waits. A disk thread drains the rings into WAV files under `Recorded/` beside the project, or a temporary folder for an unsaved one.
- **Latency compensation from the stream.** `Compensation::from_status` takes the input and output latency the device reports. A take's place on the timeline is the frame its first block played at, earlier by the round trip. A loopback test through the virtual device checks this at several latencies and lands on the exact frame.
- **Takes are one undo step.** When the transport stops, `recording.add_takes` turns each take into a sample stored relative to the project, an audio clip channel named after the insert, and a clip on the first free track.
- **MIDI recording.** Live notes recorded during a take go back to the control thread through a bounded queue and become notes of the current pattern through one batch of `note.add_many`: wrapped into the pattern in Pattern mode, from the bar where recording began in Song mode.
- **Count-in.** With count-in on, recording starts after one bar of clicks while the playhead waits; capture starts when the song does.

## Consequences

- Recording is glitch-free under disk stalls up to the ring's length, and it reports dropped blocks rather than hiding them.
- Pre-FX and post-FX taps give the same signal as the input tap until FX slots arrive in M4. FLAC records as 32-bit float WAV for now.
- Takes that are still finishing when the project is replaced stay on disk but are not placed; loop recording keeps one take across loop passes.

## Alternatives considered

- **Writing from the audio thread with a large buffer:** any disk stall becomes a dropout.
- **Compensating by a user setting:** most devices report their latency; a manual offset can come later for those that don't.
- **One file per loop pass:** wanted for comping, but needs take lanes in the playlist first.
