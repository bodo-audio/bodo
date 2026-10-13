# ADR 0018: The editing session: transactions, selection, gestures and references

- **Status:** Accepted
- **Date:** 2026-10-13
- **Deciders:** hasan-muzak

## Context

ADR 0005 says every change is a registered command run through `bodo-engine`'s `execute(id, params)`. Building the engine for M2 needed answers that ADR leaves open:

- what a command may change besides the project, such as the selection and the transport;
- how a drag becomes one undo step without the engine knowing about mice;
- how agents and people name things without knowing ids;
- how the engine, which is synchronous and knows nothing about audio, tells the audio engine to play;
- when `Project::validate()` runs (project.md section 8.4).

## Decision

- **One owner.** `bodo_engine::Session` owns the current `Arc<Project>`, the undo history, the editor state, the transport state and the environment (factory content and sample folders). Frontends call it from one thread, or send requests to that thread.
- **Two kinds of handler.**
  - An *edit* gets a `Ctx` holding scratch copies of the project, the editor state and the transport. If it succeeds, the session commits all three together; if it fails, nothing changes. Copying a project costs a handful of pointers.
  - A *session command* acts on the session itself: undo, redo, new, open and save. It can't run inside a batch.
- **Effects are enforced.** An edit that changes the project must be declared `undoable`; the session refuses the commit otherwise. Selection, transport and queries are `Effects::NONE` and never touch the history. New and open refuse to drop unsaved changes unless the call says `discardChanges`.
- **Editor state is not the document.** The current pattern, the piano roll's channel and the selected channels, clips, notes, insert and track live in `EditorState`. Empty params act on them. After every commit, undo and redo, the session drops selections that no longer exist.
- **Gestures.** A `Call` may carry a gesture id. A command with a `merge_key` folds into the newest undo step when the step has the same key: the command id, the key derived from its params, and the gesture id. The UI makes one gesture per drag or paint stroke; agents make none, so each of their calls is a step. Undo seals the newest step, so a gesture never continues across an undo.
- **Transport events.** Transport commands change `TransportState` and queue `Event`s (play, stop, seek, project replaced, saved). Whoever plays audio drains them with `take_events` and reports the playhead back with `set_playhead`. A headless session simply never plays.
- **References.** Channels, patterns, tracks and inserts are named by id, by `kind:N`, by a bare 1-based number, or by name, ignoring case; insert 0 and `master` are the master. An ambiguous name is an error listing the candidates. Positions are ticks, `bar:beat:tick` counted from 1, or seconds; lengths are ticks or `2 bars`, `1 beat`, `4 steps`. Keys are MIDI numbers or names with C4 = 60.
- **Steps are notes.** Step `n` is on when the channel has a note starting inside that sixteenth. Turning it on adds a one-step note at the channel's root key; turning it off removes the notes starting inside it. The channel rack and `pattern.set_steps` both work this way, so steps and the piano roll never disagree.
- **Validation.** `Project::validate()` runs after every commit in debug builds and in deterministic sessions (tests), and an invalid commit is refused with the violations. Release builds rely on commands keeping the invariants, which the property tests check.
- **Reproducible tests.** `Session::deterministic` counts ids up from a fixed start and records no creation time, so command results can be pinned by snapshots.

## Consequences

- A drag on any knob, fader, step row, clip or note is one undo step, while each agent call stays inspectable on its own.
- Agents can work from names and musical positions; ids are still returned for precision.
- The session doesn't depend on audio, so `bodo-cli mcp` runs it headless and tests run without devices.
- The UI must handle events every frame, and must pass gesture ids for drags; forgetting one only costs extra undo steps.
- Selection changes are not undoable, which matches project.md section 10.3.

## Alternatives considered

- **Selection in the document:** it would enter the undo history and save files, which users don't expect.
- **Merging by time window** instead of gesture ids: two quick, separate edits would merge by accident.
- **The engine driving the audio engine directly:** it would tie L7 to devices and threads, and headless use would need a fake device.
