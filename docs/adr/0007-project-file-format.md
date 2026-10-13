# ADR 0007: `.bodo` project file format

- **Status:** Accepted
- **Date:** 2026-10-13
- **Deciders:** hasan-muzak

## Context

Project files have to meet several needs at once:

- be robust, and readable by people and agents;
- diff cleanly in git;
- store plugin state blobs and, optionally, embedded audio;
- support migrations as the model evolves;
- never lose work.

Formats without schema evolution (bincode) or with brittle layouts (rkyv) don't fit. pixel3d and soundcraft both use versioned serde JSON.

## Decision

- **`.bodo` is a zip container** holding:
  - `project.json`: serde JSON with `"format": "bodo"`, an integer `format_version`, and `float_roundtrip`;
  - `plugins/<instance>.state`: binary state blobs;
  - optional `samples/` and `recorded/` folders;
  - an optional `thumbnail.png`.
- **Folder mode** (`.bodoproj/`) is the same layout unpacked, for use with git. `bodo-cli pack` and `bodo-cli unpack` convert between the two.
- **External samples** are referenced by relative path, BLAKE3 hash and an optional marketplace reference. The missing-file resolver searches by hash.
- **Versioning:**
  - a chain of forward migrations, one per version step;
  - older builds refuse newer files with a clear message;
  - migration tests run over a corpus of fixture projects.
- **Safety:**
  - saves are atomic (temp file, fsync, rename);
  - autosave runs every 5 minutes, configurable;
  - the last 10 backups rotate;
  - crash recovery is offered on the next launch.
- **Plugin states** are captured through the plugins' main-thread APIs at save time, never on the audio thread.
- **Interchange:** Standard MIDI Files, WAV, FLAC and stems, plus DAWproject import and export, all in `bodo-io`.

## Consequences

- Agents can read and generate projects directly, and diffs are meaningful.
- JSON is larger and slower than binary. Very large note sets are still fine (tens of MB parse in well under a second), and binary blobs stay outside the JSON.
- Every model change needs a migration discipline, enforced by the fixture corpus.

## Alternatives considered

- **RON:** readable, but agents and other tools support it less well.
- **SQLite single file:** good for history, but opaque to diffs.
- **MessagePack or postcard:** compact, but not human-readable.
- **bincode or rkyv:** no schema evolution, or brittle across versions.
