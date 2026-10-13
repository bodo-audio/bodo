# ADR 0016: Disk streaming in `bodo-kit-audio` instead of `creek`

- **Status:** Accepted
- **Date:** 2026-10-13
- **Deciders:** hasan-muzak

## Context

project.md section 3 lists `creek` for real-time disk streaming. As of October 2026, `creek-decode-symphonia` 0.3.3 is built on Symphonia 0.5, while Bodo decodes with Symphonia 0.6.1 (MPL-2.0). Using `creek` would put two Symphonia versions into every build, along with two copies of the codecs.

## Decision

`bodo-kit-audio` streams with its own module, `stream`:

- One **disk thread** decodes ahead for every open stream, polling every 5 ms when all streams are full.
- Each stream owns a fixed set of **chunk buffers**, allocated when it opens. They circulate between two bounded queues: empty chunks go to the disk thread, and filled chunks go to the reader.
- The **reader** on the audio thread copies frames out and hands chunks back. It never allocates, frees or waits. When data isn't ready it plays silence and counts an underrun.
- **Seeking** bumps a generation number. The disk thread seeks its decoder within one poll, and the reader drops chunks from older generations.

## Consequences

- One Symphonia version and one set of codecs.
- Seeking to a point that isn't buffered plays a few milliseconds of silence. `creek` pre-caches loop starts to avoid this; Bodo will add a cache for loop and clip starts when audio clips arrive in M3.
- The module is about 400 lines that Bodo maintains. Tests check it against full decoding, across seeks and at the end of the file, and that reads never allocate.

## Alternatives considered

- **`creek` with two Symphonia versions:** larger binaries, duplicate codec code, and two sets of advisories to track.
- **Waiting for `creek` to move to Symphonia 0.6:** it would block M1 on a third party.
