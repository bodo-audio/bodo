# ADR 0027: Recording formats, loop recording and stems

- **Status:** Accepted
- **Date:** 2026-10-13
- **Deciders:** hasan-muzak
- **Supersedes in part:** ADR 0022, its consequences that FLAC records as WAV and that loop recording keeps one take

## Context

Project.md section 9.6 asks for takes in WAV, W64 or FLAC. ADR 0022 recorded FLAC as 32-bit float WAV and kept one take across loop passes until the mixer existed. Section 6.8 asks for stems per insert that line up with the mix. M4's exit criterion asks that stems sum to the mix within -100 dBFS.

Constraints:

- The disk thread must stream: a take can be hours long, and its length is unknown until it stops.
- Kit stays permissively licensed, so an encoder can't be pulled in from a GPL or LGPL library.
- Everything on the audio thread stays lock-free and allocation-free.

## Decision

- **Formats** (`bodo-kit-audio`):
  - **FLAC** has a small streaming encoder of its own (`FlacWriter`): 24-bit samples, frames of 4096 samples, independent channels. Each subframe picks the cheapest of verbatim and the fixed predictors of order 0 to 4. The residual is Rice coded in up to 16 partitions. STREAMINFO goes first and is patched with the length and frame sizes when the writer finishes. The MD5 signature is left zero, which the format defines as "not computed". The decoder (symphonia) reads the result back bit for bit.
  - **Wave64** (`W64Writer`, `W64Reader`) writes 32-bit float or 24-bit samples in GUID-tagged chunks with 64-bit sizes, so takes can pass WAV's 4 GB. The decoder recognizes W64 by its header, and the browser lists `.w64` files.
  - `RecordFormat` gains `W64`. `mixer.set_record_format` takes `float32`, `int24`, `flac` or `w64`.
- **Loop recording** (`transport.loop_record`, an `Event` the frontend passes to `Engine::set_loop_record`):
  - When the transport wraps while recording with loop recording on, each strip's capture marks where the new pass starts in its ring. The marks travel in a small bounded queue beside the ring.
  - The disk thread finishes the current file at the mark and starts the next take there. Every pass becomes its own take, starting where the loop starts, and all of them land on the playlist.
  - With loop recording off, a recording across loop wraps stays one take, as before.
  - **MIDI accumulates,** with loop recording on or off: the MIDI capture marks each wrap of the song loop, and the take places every pass's notes inside the loop. A note held across the wrap ends at the loop end. Each pass overdubs the same region, as on a looper.
- **Stems** (`render_stems` in `bodo-mix`, `render.export_stems`, `bodo-cli render --stems`):
  - A stem is what an insert sends to the master. Channels that play straight into the master get stems of their own.
  - Only inserts something feeds get a stem, so a project with 16 empty inserts doesn't write 16 silent files.
  - Stems are taken from graph taps on the edges into the master, and shifted by those edges' compensation delays, so they line up with each other and with the mix.
  - With the master strip at its defaults, the stems sum to the mix within float rounding; a test checks it at -100 dBFS. The master's own processing (its slots, EQ and fader) is not in the stems.
  - `render.export_mix` writes the mix through the master. Both commands write WAV, FLAC or Wave64, and the stem files are numbered in mixer order.

## Consequences

- Recording and exporting need no external encoder, and the formats work wherever Kit does.
- Our FLAC is a little larger than a full encoder's, since there is no LPC and no stereo decorrelation. It stays well under WAV on recorded audio. An LPC stage can be added later without changing the file format.
- Loop takes stack on the playlist as separate clips. Comping them needs the take lanes of a later milestone.
- A stem export renders once for all stems, through one graph. `mix: true` renders the mix a second time.

## Alternatives considered

- **libFLAC or flacenc through FFI:** a mature encoder, but it brings a C build or a new dependency with its own license review into Kit. A fixed-predictor encoder is a few hundred lines and fully testable.
- **One long file per loop recording with markers:** cheaper on disk, but every DAW user expects one take per pass, and the playlist would need markers to split it.
- **Stems by rendering once per insert with everything else muted:** simple, but N renders instead of one, and plugins with internal state (delays, compressors fed by sidechains) would sound different from the mix.
