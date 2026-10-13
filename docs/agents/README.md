# Agent guide

Bodo has an MCP server, so any MCP client can make music with it: Claude Code, Codex, Cursor, or the built-in Bodo Assistant ([project.md section 12](../../project.md#12-command-system-and-mcp)). Agents get the same commands the interface uses, so anything you can do with the mouse, an agent can do too, and every change is one undo step.

- [`commands.md`](commands.md): every command, its params and its effects, generated from the registry.

## What the server offers

| Tool | What it does |
|---|---|
| `inspect_project` | The whole project as compact text: tempo, channels, patterns with their steps as `x...x...` strings, the playlist and the mixer |
| `inspect_pattern {pattern?}` | One pattern in detail: every note with its id, key, position, length and velocity |
| `inspect_mixer` | The master, the inserts with their channels and settings, and the routes |
| `list_commands {namespace?}` | Every command (or one namespace's) with a one-line summary and its effects |
| `describe_command {id}` | One command in full: params with types and descriptions, effects and an example |
| `execute {id, params}` | Runs one command as one undo step |
| `batch {commands, label?}` | Runs several commands in order as one transaction and one undo step; if any fails, nothing changes |

Connected to the running app, the server also has tools that read and drive the interface, as a person would: `ui_inspect` (open windows and the on-screen elements with their labels), `ui_click`, `ui_drag`, `ui_key`, `ui_text` and `ui_screenshot`. Prefer the inspect tools for facts; screenshots are for checking the interface itself.

Resources: `bodo://project`, `bodo://mixer`, `bodo://pattern/{n}` for each pattern, and `bodo://docs/commands` (the command reference).

## Connecting a client

`bodo-cli` serves MCP over stdio in two ways:

- `bodo-cli mcp [project.bodo]` works on a project file, or a new project when none is given, without opening the app. Save with the `project.save` command.
- `bodo-cli mcp --connect [port]` acts on the running Bodo app: you watch the agent work, and its changes join your undo history. Bodo lets agents connect by default; it listens on 127.0.0.1 only and requires a token that only your user account can read. The port is found automatically; give it only when you run several copies of Bodo.

For Claude Code, add the server with:

```sh
claude mcp add bodo -- bodo-cli mcp --connect
```

or put it in a project's `.mcp.json`:

```json
{
  "mcpServers": {
    "bodo": { "command": "bodo-cli", "args": ["mcp", "--connect"] }
  }
}
```

Other clients take the same command and arguments. To work on a file without the app, use `["mcp", "path/to/Song.bodo"]` instead.

## Build a beat

The fastest way from nothing to a beat. A new project starts at 120 BPM with 16 playlist tracks, one empty pattern ("Pattern 1") and a starter drum kit: Kick, Clap, HiHat and Snare, each on its own mixer insert.

1. Find more sounds. `browser.search` lists the factory kit and the user's sample folders:

   ```json
   { "id": "browser.search", "params": { "query": "808" } }
   ```

2. Load one into a new channel:

   ```json
   { "id": "channel.add", "params": { "sample": "factory:Packs/Bodo Drums/808 Bass.wav", "name": "808" } }
   ```

3. Program steps. One character per sixteenth note, `x` on and `.` off; spaces are ignored. Steps play the channel's root key.

   ```json
   { "id": "pattern.set_steps", "params": { "channel": "Kick", "steps": "x... x... x... x..." } }
   { "id": "pattern.set_steps", "params": { "channel": "Clap", "steps": ".... x... .... x..." } }
   ```

4. Make more patterns. `pattern.new` makes an empty pattern and makes it current, so the next `pattern.set_steps` calls go into it.

   ```json
   { "id": "pattern.new", "params": { "name": "Fill" } }
   ```

5. Arrange. Place patterns as clips on playlist tracks; a clip longer than its pattern repeats it.

   ```json
   { "id": "clip.place", "params": { "track": 1, "pattern": "Verse", "start": "3:1:0", "length": "4 bars" } }
   ```

6. Check, play and save:

   ```json
   { "id": "transport.set_mode", "params": { "mode": "song" } }
   { "id": "project.save", "params": { "path": "First Beat.bodo" } }
   ```

   `inspect_project` then shows something like:

   ```text
   Patterns (4):
     2 Verse: 1 bar, 17 notes
         Kick         x...x...x...x...
         Clap         ....x.......x...
         HiHat        x.x.x.x.x.x.x.x.
         808          x.....x...x.....
   Playlist (16 tracks, song length 8 bars):
     1 Drums: Intro at 1:1:0 (2 bars), Verse at 3:1:0 (4 bars), Fill at 7:1:0 (1 bar), Outro at 8:1:0 (1 bar)
   ```

The whole beat as a list of calls is in [`crates/bodo-engine/tests/fixtures/beat.json`](../../crates/bodo-engine/tests/fixtures/beat.json); it can go to `batch` in one call. To hear a saved project without the app, `bodo-cli render "First Beat.bodo" beat.wav` writes a WAV file.

## Conventions

- **Positions** are ticks (960 per quarter note) or text: `"bar:beat:tick"` counted from 1, so `"3:1:0"` is the start of bar 3, or `"bar:beat"`, `"bar"`, or seconds such as `"1.5s"`. Positions inside a pattern count from the pattern's start.
- **Lengths** are ticks or a count and a unit: `"2 bars"`, `"1 beat"`, `"4 steps"` (sixteenth notes) or `"120 ticks"`.
- **Keys** are MIDI numbers or note names; `"C4"` is 60, middle C.
- **References**: name channels, patterns, tracks and inserts by name (`"Kick"`), by number (`"channel:3"`, `"insert:2"`, or just `3`; insert 0 or `"master"` is the master) or by id (`"ch_01J..."`). A name that matches more than one thing is an error that lists the candidates.
- **Selection**: empty params act on the current selection. The current pattern is the one the channel rack shows; `ui.select_pattern` changes it, and `pattern.new` and `pattern.clone` make the new pattern current.
- **Undo**: every `execute` that changes the project is one undo step, and a `batch` is one step for all of its commands. `edit.undo` and `edit.redo` step through the history. Answers end with a `changed:` line naming the step.
- **Errors** read like a compiler's, with a hint, and change nothing. Commands that would lose unsaved work (`project.new`, `project.open`) refuse unless you pass `discardChanges: true`.
- **Connected to the app**, the user may edit between your calls, so inspect again before relying on an older answer.
