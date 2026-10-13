# ADR 0010: Marketplace trust model and signing

- **Status:** Accepted
- **Date:** 2026-10-13
- **Deciders:** hasan-muzak

## Context

The marketplace is free and offers samples, presets, templates and plugins. It starts with curated content and later accepts community submissions. Risks: malware in plugin binaries, stolen or wrongly licensed samples, tampering in transit or on mirrors, and prompt injection through metadata that the assistant reads.

## Decision

- **Package format:** a `.bodopack` is a `tar` archive compressed with `zstd`, with a `bodopack.toml` manifest (SPDX license, tags, BPM, key, BLAKE3 hash of every file, previews).
- **License allowlist:**
  - samples and presets: CC0-1.0 or CC-BY-4.0;
  - plugins: an OSI license, with the source repository and tag required.
- **Plugins are built from source by Bodo CI** on all three platforms, run through clap-validator, pluginval and auval, then signed and notarized. Uploaded binaries are never accepted.
- **Automated checks:**
  - the manifest schema and the license allowlist;
  - file formats, plus loudness sanity checks;
  - duplicates and plagiarism, detected with audio fingerprints;
  - malware scanning;
  - metadata length caps and sanitization.
- **Human curation:** a curator approves every new publisher and the first version of every package. Trusted publishers' later versions are approved automatically once the checks pass.
- **Signatures:**
  - publishers sign with ed25519 (minisign);
  - the Bodo index key countersigns every index entry;
  - clients verify the publisher signature, the index signature and the content hash before using anything;
  - keys rotate through a signed key list;
  - a monotonic index version protects against rollback.
- **Delivery:**
  - a sparse static index (one JSON file per package) in a git repo, served by a CDN;
  - content-addressed artifacts in R2;
  - Opus previews, precomputed peaks and features, so items can be auditioned before download.
- **Use and reporting:**
  - CC-BY attribution is tracked automatically and written to `CREDITS.txt` on export;
  - there is a report button and a documented DMCA process.
- **Metadata as data:** all marketplace metadata reaches the assistant only as untrusted data (see ADR 0008).

## Consequences

- Strong supply-chain guarantees. The cost is CI capacity for building plugins and curator time.
- Closed-source plugins can't be distributed through the marketplace. That is deliberate: everything there is free and open.
- The client ships with the index public keys and must handle key rotation.

## Alternatives considered

- **An open registry with no review:** malware and copyright risk.
- **Accepting uploaded binaries:** can't be verified.
- **TUF (The Update Framework) in full:** more machinery than we need now, though we can revisit it if the threat model grows.
