# ADR 0030: Channel and track groups, track solo, and commands that render audio

- **Status:** Accepted
- **Date:** 2026-10-13
- **Deciders:** hasan-muzak

## Context

M4 closes the remaining gaps of the Channel Rack and the Playlist ([project.md sections 10.2 and 10.3](../../project.md#102-channel-rack)): channel groups with a filter, track groups that fold, mute and solo together, and consolidating clips to audio. The model had no groups and no track solo, and no command rendered audio: rendering lived in `bodo-mix`, which the command engine didn't depend on. Agents must be able to do everything the interface does ([ADR 0005](0005-everything-is-a-command.md)).

## Decision

- **Groups are names.** A channel has `group: Option<String>` and a playlist track has the same. A group exists while something is in it; renaming one renames it on every member, and a name already in use is refused. No separate group table, so nothing can dangle.
- **A track group's tracks sit together.** `playlist.group_tracks` moves the joining tracks right after the group's current ones, so a group is one run of tracks and folds under one header. Folding is a view setting, not part of the project.
- **Track solo is in the model.** `PlaylistTrack::solo`: while any track is soloed, only soloed tracks play, and mute still wins. The song, audio clip and automation indexes all use `PlaylistTrack::plays`.
- **Commands may render.** `bodo-engine` depends on `bodo-mix` (layer 7 on layer 6), and `clip.consolidate` renders with `render_offline` on the command's thread, writes a WAV into the project's `Rendered/` folder (a temporary folder for an unsaved project), and replaces the clips with one audio clip, as one undo step. The render plays only the chosen pattern and audio clips, keeps automation, and leaves the master strip neutral, since the result plays through the master again.
- **A time selection is an interface state with explicit commands.** The playlist keeps it in its view; `edit.copy` and `edit.cut` take `start` and `end`, and `playlist.delete_time` and `insert_time` already took a span, so agents pass the range they mean.

## Consequences

- Agents group, solo, cut time and consolidate through the same commands as the interface; an MCP test does it end to end.
- Consolidating a long stretch blocks the command for as long as the render takes, a fraction of real time. A progress report and cancellation can come with the job system of M5.
- Renaming a group touches every member; groups are small, so it stays cheap.

## Alternatives considered

- **Groups as entities with ids:** adds a table, references that can dangle and validation for both; names do the job.
- **Rendering in the interface and passing a file to a command:** agents could not consolidate.
- **Solo as a mixer concept only:** the mixer's solo set (M4's mixer stream) is per insert; FL's playlist solo is per track, and both are needed.
