# ADR 0034: Channels get mixer tracks only when assigned

- **Status:** Accepted
- **Date:** 2026-10-13
- **Deciders:** hasan-muzak

## Context

Until now every new channel took the first mixer insert no other channel used, and the starter kit of [ADR 0020](0020-factory-content-and-one-shot-samplers.md) loaded Kick, Clap, HiHat and Snare onto inserts 1 to 4. The owner found this got in the way: the mixer filled up with tracks nobody chose, named "Insert 1" to "Insert 4" while playing the kick and the snare. FL Studio's channel rack works the other way: a channel plays into the master until you assign it a mixer track from its menu.

## Decision

- **New channels play into the master.** `channel.add`, the starter kit, loaded channel presets and imported MIDI parts no longer take a mixer insert by themselves; an explicit `target` still works. This supersedes the "one channel per insert" part of ADR 0020's starter kit.
- **Assigning is a command,** `channel.assign_insert`, in the channel's menu as "Assign to mixer track". It uses the insert selected in the mixer; with none selected, or with `free`, it takes a free one: the first that no channel plays into and that still has its default name, made after the last when every one is taken. "Assign to a free mixer track" asks for that directly.
- **The track takes the channel's name** when it still has its default name ("Insert 5"); a track the user named keeps its name.
- **`channel.unassign_insert`** ("Unassign from mixer track") plays a channel into the master again.
- **A new project selects no insert,** so the first assignments go to free tracks.
- **The channel rack shows the assignment** under the channel's name, small: "Mixer 3 · Kick".

## Consequences

- The mixer starts empty, and each track in use carries the name of what plays into it.
- The test beat assigns its channels explicitly, so its mixer snapshots name the tracks after the drums.
- Once a mixer track is selected, "Assign to mixer track" uses it until the selection changes; "Assign to a free mixer track" is always one click away.

## Alternatives considered

- **Keeping the automatic free insert:** the behavior the owner asked to remove.
- **Always a new track on assign, ignoring the mixer's selection:** loses the quick way to send several channels to one bus.
