# ADR 0029: The user library, presets and sound preview

- **Status:** Accepted
- **Date:** 2026-10-13
- **Deciders:** hasan-muzak

## Context

Project.md section 14.1 gives the Browser back and up buttons, the tabs All, Project, Plugins and Sounds, top entries for the current project, recent files, the plugin database and the preset folders, eighteen library folders (Audio, Backup, Templates, Recorded and so on), a Marketplace section that marks new arrivals, and tag filters with search. Those folders need a home on disk, and things to be in them: presets and scores to save and load, templates and demos to start from, recordings, exports and autosaves. Clicking a sound should play it at once, while the audio thread stays free of allocation, locks and disk access (AGENTS.md section 4).

## Decision

- **A user library in the user's documents.** On first run the app makes a `Bodo` folder (or the folder `BODO_LIBRARY` names) with the eighteen folders, and in User data the four preset folders, the Marketplace's Plugins and Stems folders, and the tags file. It is a preference; the Browser's gear changes it. `bodo_engine::library::UserLibrary` knows the layout, and `Env.user` gives it to commands, so agents use the same places.
- **Bodo uses the folders.** A project's first save starts in My projects. Autosaves go to Backup, where the Browser opens them as unsaved projects. Recordings and MIDI exports of a project with no folder of its own go to Recorded and Rendered. `project.save_template` writes to Templates, and `project.new { from }` starts a new, unsaved project from a template or demo, so the original is never overwritten. The demo project is a script of commands over the factory kit, written into Demo projects on first run, so it is CC0 like the kit and always in the current format.
- **Presets are small JSON files** with a `format` and a `version`, like `project.json`: `.bodochannel` (the channel as the project stores it, its samples by path and hash, its plugin states as base64, and a layer's children), `.bodomixer` (an insert's settings and slots) and `.bodoscore` (one channel's notes). Their ids are the saving project's; loading gives everything new ids. `preset.save_*` and `preset.load_*` save into and load from the library; loading is one undo step. A mixer preset leaves the insert's name, routing, hardware ports, recording settings, mute and solo alone. An automation clip can't be a preset: it controls parameters of one project.
- **Keys name library files.** `factory:` and `library:` prefixes name files by their place in the factory content and the user library; tags are stored by key, so they survive moving the library. Every file is tagged by its folders and the words of its name; the user adds their own from the Browser or `library.tag`.
- **An index read off the interface thread.** The Browser builds `LibraryIndex` on a background thread, at most a fixed number of files, and keeps it until a refresh. Search and tag filters read it; folder listings are read once when a folder opens.
- **The Marketplace's owned items** are whatever is in its two folders; `seen.json` records what the user has looked at, and the rest is new.
- **Preview after the master.** `bodo_mix::preview` decodes at most 30 seconds of the file on a background thread, converts it to the engine's rate, and hands it over with `bodo_kit_core::handoff`. The device callback adds it after the engine, so the engine's render needs no change. A new sound or a stop copies the old sound's next 5 ms into the new one's preallocated tail and fades it out, and each sound fades in, so nothing clicks and the audio thread never allocates or frees.
- **Drops land through the recorded elements.** A preset dragged onto the channel rack, a rack row, the mixer, a strip or the piano roll finds its target among the elements windows record for agents and tests, so the windows need no drop code of their own.

## Consequences

- The Browser's folders are real places with real contents from the first run, and agents reach the same files through commands.
- The library lives outside projects, so a project that uses a library sound stores its absolute path; moving the library breaks those paths until the missing-file resolver finds them by hash.
- Plugin presets have a folder and a file type, but nothing loads them until plugins arrive (M5 and M6).
- The index holds up to 20,000 files; a larger library is searched only in part until the index is made incremental.

## Alternatives considered

- **Keeping presets inside projects:** they could not be shared between projects, which is their point.
- **A binary preset format:** smaller, but presets are small, and readable JSON can be versioned, diffed and repaired like `project.json`.
- **Previewing through a sampler channel in the project:** it would change the project and its undo history just to listen.
- **Searching the disk on every keystroke:** fine for a small folder, slow for a real library; the index costs one background read.
