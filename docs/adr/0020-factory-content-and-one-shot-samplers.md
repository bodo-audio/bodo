# ADR 0020: Factory content and one-shot samplers

- **Status:** Accepted
- **Date:** 2026-10-13
- **Deciders:** hasan-muzak

## Context

M2's exit criterion is building a beat with the step sequencer and the sampler. A fresh install needs sounds for that, the sounds must be free for anyone to use in their music, and nothing may come from a commercial product (AGENTS.md section 11). Projects must keep finding those sounds when Bodo is installed somewhere else. And drum steps are short notes: a sampler that stops its sample at the note's end cuts kicks and 808s short.

## Decision

- **Generated, CC0 factory content.** `cargo xtask factory` synthesizes the Bodo Drums kit (eleven one-shots) from seeded noise and oscillators, peak-normalized to -6 dBFS so hits that land together keep headroom. The WAVs are committed under `assets/factory/` and released as CC0-1.0. The committed files are the reference; another platform may round a few samples differently.
- **`factory:` paths.** A sample stored as `factory:Packs/Bodo Drums/Kick.wav` is relative to the factory content wherever it is installed. `bodo_model::SampleRoots` resolves the three forms of sample path (relative to the project folder, absolute, `factory:`), and the engine loads samples through it. `bodo_engine::locate_factory` looks in `BODO_FACTORY`, beside the executable (or in a macOS bundle's Resources, or `../share/bodo` on Linux), then in the repository for development builds.
- **A starter kit.** The basic template, used for new projects, loads Kick, Clap, HiHat and Snare onto inserts 1 to 4 when the factory content is found.
- **One-shot samplers.** `SamplerParams::one_shot`, on by default: a note starts the sample and it plays to its end. A looping sample still stops with its note, and stopping or moving the transport releases everything. Files saved before the field existed load with the default.

## Consequences

- A user can make a beat in the first minute, and agents can find the kit with `browser.search`.
- Projects that use factory sounds stay valid across installs and machines, and so do their hashes.
- The repository carries about 1.6 MB of audio, regenerated only when the generator changes.
- One-shot is a per-channel setting; the sampler window, when it arrives, will expose it with the envelope.

## Alternatives considered

- **Downloading a kit at first run:** needs the network and a host, and the sounds' license would still need care.
- **Absolute paths to factory files:** break when Bodo moves.
- **Gating the sample by note length, with longer step notes:** drum steps would need lengths per sample, and the piano roll would show long overlapping notes.
