# ADR 0006: Domain model with 960 PPQ, ULID ids and structural sharing

- **Status:** Accepted
- **Date:** 2026-10-13
- **Deciders:** hasan-muzak

## Context

The project model has four jobs:

- support FL-style patterns, the channel rack, a free-form playlist and an optional single-channel clip mode;
- give agents stable references;
- make undo cheap;
- hand snapshots to the engine without cloning the whole document, which soundcraft does on every edit.

## Decision

- **Time:**
  - 960 PPQ, with `i64` ticks.
  - Straight subdivisions down to 1/256, triplets and quintuplets are exact.
  - The resolution fits in SMF headers.
  - PPQ is stored in project settings, so it can be migrated later.
  - `Tick`, `Frames` and `Seconds` are distinct newtypes.
- **The tempo map** is a set of sorted points (jump or linear) with a cache of cumulative seconds. Conversions take O(log n) and use closed forms inside each segment.
- **IDs** are typed ULID newtypes, stable across save and load, undo and branch merges. Display numbers come from order vectors. A `NoteId` is a `u32` unique within its pattern, and maps directly to CLAP note ids.
- **Collections** are `imbl` persistent maps and vectors, with `Arc` around each entity:
  - an edit copies only the path to the change, O(log n);
  - undo is a stack of `Arc<Project>`, so undo and redo are pointer swaps;
  - a `merge_key` folds a whole gesture into one undo entry.
- **Diffs** use `Arc::ptr_eq` to skip unchanged subtrees. The renderer, the index caches, the sequencer and the Agent Changes panel all depend on this.
- **Patterns** hold notes per channel. `binding: Option<ChannelId>` turns on clip mode, where only that channel may appear. Patterns can also carry their own automation lanes.
- **The mixer** is a map of inserts plus the master, each with `slots: [FxSlot; 10]` (exactly ten, enforced by the type), and a map of routes keyed by source and destination. A source can be a channel or an insert.
- **Invariants** are checked by `Project::validate()` after every transaction in debug builds and in CI:
  - every id resolves;
  - the graph is acyclic;
  - sequences are sorted;
  - every length is positive;
  - clip-mode patterns contain only their bound channel;
  - every sidechain reference points to an existing route.

## Consequences

- Snapshots and undo are cheap, and agent branches cost O(1) to create.
- Persistent collections are slower to iterate than a plain `Vec`. Hot paths read through derived indexes cached per `Arc` pointer.
- Every model change needs serde defaults and, when older files are affected, a migration.

## Alternatives considered

- **480 PPQ:** too coarse for subtle humanization.
- **3840 PPQ:** we don't need it yet, and it can be added later by migration.
- **slotmap keys:** not stable across saves or merges.
- **Deep clones for every undo step:** O(n) per edit.
- **CRDTs (loro, automerge):** we would only need them for collaboration, which comes after 1.0.
