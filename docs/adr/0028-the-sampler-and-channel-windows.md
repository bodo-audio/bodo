# ADR 0028: The Sampler and channel windows

- **Status:** Accepted
- **Date:** 2026-10-13
- **Deciders:** hasan-muzak

## Context

[Project.md section 10.2](../../project.md#102-channel-rack) gives the built-in Sampler a full window: the played part and its loop, content options (keep on disk, resample, regions, slice markers), declicking, time stretching, effects rendered into a copy of the sample, three envelopes, a filter, an arpeggiator, and the key range and root note. Layer and MIDI Out channels must play, and every channel kind needs a window ([section 14.1](../../project.md#141-main-window-layout)). The audio thread may not allocate, lock or touch the disk ([AGENTS.md section 4](../../AGENTS.md)), and renders must not depend on block size.

## Decision

- **The model holds every setting.** `SamplerParams` gains the end and start offset, content options, pitch range, declicking, `SamplerStretch`, `SampleEffects`, `SamplerFilter`, two `ModEnvelope`s and the `Arpeggiator`, all with serde defaults, so older files load unchanged. One command, `sampler.set`, sets any of them; nested groups take only the fields given, and knob drags merge per channel into one undo step.
- **What a sampler plays is rendered on the control thread.** `bodo_mix::sampler_source::derive` applies, in order: trim, DC removal, normalize, polarity, channel swap, stereo fade, reverse, fades, loop crossfade and a Signalsmith stretch with pitch shift. File markers (cue points, regions with `ltxt` lengths, `smpl` loops, read by `bodo_kit_audio::markers`) move with the edits and become key zones. Renders are cached by source and shape, so knobs that don't change the sample never render it again. The Sampler window's waveform runs the same function on a thread, so it shows exactly what plays.
- **Voices do the rest per frame.** Each voice plays its region, or the zone of its key at its own pitch, from the start offset; loops forward or back and forth; declicks; runs its volume envelope and a state-variable filter with key tracking. The filter and pitch envelopes move their targets every 16 frames *of the voice's own life*, so renders are identical at any block size.
- **The arpeggiator runs on the song's ticks.** `BlockContext` now carries the block's tick and ticks per frame. The arpeggiator keeps held keys in fixed arrays and interleaves its steps with incoming events, without allocating.
- **Keep on disk streams, where it can.** A real-time engine streams a sampler's sample through the kit's disk thread ([ADR 0016](0016-disk-streaming-in-kit-audio.md)) when the sampler keeps it on disk and plays it as the file holds it. The first second from each place a note or loop starts (with a few frames of lead for interpolation) stays in memory as a head; each voice has its own reader, sought to its head's end, filling a 16 384-frame window. Offline renders always load samples, so exports never wait for the disk.
- **Layers expand to their children** when the song index is built and when live notes reach the audio thread; nested layers flatten, and a layer can't play itself.
- **MIDI Out channels are silent graph nodes.** They turn their notes into messages on a bounded queue (1024, drop and count when full), and a thread in the app sends them to the port. The audio thread never touches a MIDI driver.
- **Channel windows are free windows with their own state.** `windows::show_frame` draws any window from a state the caller keeps; the main windows keep theirs in the layout, and channel windows in the view, keyed by channel, so several are open at once. A channel's name button opens its window.

## Consequences

- Everything in the Sampler window works through commands, so agents can drive the whole sampler.
- Rendered effects, a reverse, a stretch, slices, regions and ping-pong loops need the sample in memory; such samplers load it even when asked to keep it on disk.
- Live MIDI and arpeggiated timing stay quantized to the block start for live input (ADR 0023); sequenced notes and arpeggio steps are frame-exact.
- An instrument channel keeps its plugin and shows it in its window; it plays once plugin hosting arrives (M5).
- A streamed voice whose disk falls behind plays silence and counts an underrun rather than waiting.

## Alternatives considered

- **Applying precomputed effects on the audio thread:** costs CPU on every voice and complicates the RT path for edits that change rarely.
- **Updating modulation at block boundaries:** cheaper to write, but renders would change with the block size.
- **A separate command per sampler setting:** dozens of commands for agents to learn; nested partial updates keep one entry point.
- **Streaming every sampler sample:** short samples gain nothing, and effects need the whole sample anyway.
