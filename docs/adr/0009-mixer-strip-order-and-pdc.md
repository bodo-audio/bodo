# ADR 0009: Mixer strip signal order, routing and delay compensation

- **Status:** Accepted
- **Date:** 2026-10-13
- **Deciders:** hasan-muzak

## Context

The mixer follows the FL model:

- inserts plus one master;
- a channel can feed several inserts;
- inserts route to each other with send levels and sidechains;
- exactly 10 ordered FX slots per insert.

Each strip has volume, pan, stereo separation, reverse polarity, swap L/R, track latency and disk-record arming. The order of these stages must be defined exactly, so that renders can be reproduced and the assistant's reasoning holds up.

## Decision

- **Strip order:**
  1. input sum (latency-aligned);
  2. reverse polarity;
  3. swap L/R;
  4. FX slots 1 to 10;
  5. 3-band strip EQ;
  6. stereo separation (M/S);
  7. pan;
  8. fader;
  9. mute ramp;
  10. post-fader point;
  11. delay for a negative track latency;
  12. outputs.
- **Polarity and swap come before the FX**, like a console preamp, so dynamics and saturation see the corrected signal. This intentionally differs from FL.
- **Slots:**
  - each slot has its own enable switch and dry/wet mix;
  - the dry path is delayed by the slot's latency, so partial mixes never comb-filter;
  - disabled slots keep reporting their latency, so timing never jumps.
- **Taps:**
  - sends tap PreFx, PreFader or PostFader (the default);
  - recording taps Input, PreFx, PostFx or PostFader.
- **Routes:**
  - keyed by `(Channel | Insert) -> Insert`, with a level, a tap and a kind (`Audio` or `Sidechain`);
  - sidechain routes feed only plugin sidechain inputs, chosen per slot.
- **Cycles** are rejected when a route is added, using Pearce-Kelly incremental topological ordering. The fast path is O(1) and the worst case is O(V + E). The error returns the cycle path.
- **Compilation:** Kahn's algorithm, run only when something changes.
- **PDC** is one pass in topological order:
  - `out(n) = in(n) + L(n)`;
  - audio and sidechain edges are aligned using the latency before each sidechained slot;
  - a positive track latency is declared latency (everything else gets delayed);
  - a negative track latency delays this strip;
  - automation is read at `t - in(n)`;
  - `PdcBypass` exists for monitoring.
- **Solo:** the audible set is `S ∪ ancestors(S) ∪ descendants(S)`, computed in O(V + E). Nodes outside it keep processing at gain 0 so their sidechains still work.
- **Pan law:** stereo-fold pan, unity at center, for stereo signals; -3 dB constant power for mono. The law is a project setting.

## Consequences

- Behavior is defined exactly and can be tested with null tests and random-graph PDC tests.
- The pre-FX polarity and swap stages differ from FL. That is documented in the manual and in agent docs.
- Dry/wet compensation and per-edge delay lines cost memory, which is preallocated when each plan is compiled.

## Alternatives considered

- **Polarity and swap after the FX (FL's order):** dynamics would react to the uncorrected signal.
- **Recomputing the topology every block:** wasteful.
- **Running a full cycle check on every route edit:** works, but the incremental check is cheaper and also gives an ordering.
