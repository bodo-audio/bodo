# Commands

Every command registered in `bodo-engine`, generated from the registry by `cargo test -p bodo-engine --test docs` (set `BODO_BLESS=1` to rewrite it). MCP serves the same text and generates its tools from the same registry ([ADR 0005](../adr/0005-everything-is-a-command.md)); AGENTS.md section 7 explains how to add a command.

Conventions:

- Empty params act on the current selection: the current pattern, the piano roll's channel, the selected channels, clips, notes, insert or track.
- Positions are ticks (960 per quarter note) or text: `"bar:beat:tick"` counted from 1, or seconds such as `"1.5s"`. Lengths are ticks or text such as `"2 bars"`, `"1 beat"` or `"4 steps"`.
- Channels, patterns, tracks and inserts are named by id, by number (`"channel:3"`, `"insert:0"` for the master, or just `3`) or by name. An ambiguous name is an error that lists the candidates.
- Effects: `undoable` commands change the project and make one undo step; `destructive` ones can lose unsaved work; `disk` ones write files.

## browser

### `browser.search`

Finds audio files in the factory content and the user's sample folders; pass a result's path to channel.add.

- Effects: none

| Param | Type | Description |
|---|---|---|
| `limit` | integer, optional | The most results, from 1 to 500; 50 by default. |
| `query` | string, optional | Words the sound's folder and name must contain, ignoring case, such as "drums kick". Empty lists everything. |

Example:

```json
{
  "query": "kick"
}
```

## channel

### `channel.add`

Adds a sampler channel to the channel rack, optionally loading a sample.

- Effects: undoable
- Menu: Add > Channel

| Param | Type | Description |
|---|---|---|
| `color` | Color, optional | Its color; the next palette color by default. |
| `name` | string, optional | Its name; the sample's file name by default. |
| `sample` | string, optional | The audio file its sampler plays: a `factory:` path from browser.search, an absolute path, or a path inside the project's folder. Without one the sampler starts empty. |
| `target` | InsertRef, optional | The mixer insert it plays into; by default the first insert no channel uses yet. |

Example:

```json
{
  "name": "Kick",
  "sample": "factory:Packs/Bodo Drums/Kick.wav"
}
```

### `channel.duplicate`

Copies a channel's settings into a new channel after it (notes are not copied).

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `channel` | ChannelRef, optional | The channel; the selected channel by default. |

Example:

```json
{
  "channel": "Kick"
}
```

### `channel.mute`

Mutes or unmutes a channel; toggles without `muted`.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `channel` | ChannelRef, optional | The channel; the selected channel by default. |
| `muted` | boolean, optional | Muted or not; toggles when left out. |

Example:

```json
{
  "channel": "Clap"
}
```

### `channel.remove`

Removes channels with their notes, clips and routes.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `channels` | array of ChannelRef, optional | The channels; the selected channels by default. |

Example:

```json
{
  "channels": [
    "Clap"
  ]
}
```

### `channel.rename`

Renames a channel.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `channel` | ChannelRef, optional | The channel; the selected channel by default. |
| `name` | string | The new name. |

Example:

```json
{
  "channel": "Kick",
  "name": "Big Kick"
}
```

### `channel.reorder`

Moves a channel to another position in the channel rack.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `channel` | ChannelRef | The channel to move. |
| `to` | integer | Its new position in the channel rack, from 1. |

Example:

```json
{
  "channel": "Snare",
  "to": 1
}
```

### `channel.set_color`

Sets a channel's color.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `channel` | ChannelRef, optional | The channel; the selected channel by default. |
| `color` | Color | The new color. |

Example:

```json
{
  "channel": "Kick",
  "color": "#f87171"
}
```

### `channel.set_pan`

Sets a channel's pan from -1 (left) to 1 (right).

- Effects: undoable
- Calls in one gesture fold into one undo step.

| Param | Type | Description |
|---|---|---|
| `channel` | ChannelRef, optional | The channel; the selected channel by default. |
| `pan` | number | Pan from -1 (left) to 1 (right); 0 is center. |

Example:

```json
{
  "channel": "HiHat",
  "pan": 0.25
}
```

### `channel.set_pitch`

Shifts a channel's pitch in cents.

- Effects: undoable
- Calls in one gesture fold into one undo step.

| Param | Type | Description |
|---|---|---|
| `cents` | number | Pitch shift in cents, from -2400 to 2400. |
| `channel` | ChannelRef, optional | The channel; the selected channel by default. |

Example:

```json
{
  "cents": -100,
  "channel": "Snare"
}
```

### `channel.set_root_key`

Sets the key that plays a sampler's sample at its own pitch; steps play this key.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `channel` | ChannelRef, optional | The sampler channel; the selected channel by default. |
| `key` | Key | The key that plays the sample at its own pitch. Steps play this key. |

Example:

```json
{
  "channel": "Kick",
  "key": "C4"
}
```

### `channel.set_sample`

Loads an audio file into a sampler channel.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `channel` | ChannelRef, optional | The sampler channel; the selected channel by default. |
| `sample` | string | The audio file: a `factory:` path, an absolute path, or a path inside the project's folder. |

Example:

```json
{
  "channel": "Snare",
  "sample": "factory:Packs/Bodo Drums/Rim.wav"
}
```

### `channel.set_swing`

Sets how late every second step plays, from 0 to 1.

- Effects: undoable
- Calls in one gesture fold into one undo step.

| Param | Type | Description |
|---|---|---|
| `channel` | ChannelRef, optional | The channel; the selected channel by default. |
| `swing` | number | How late every second step plays, from 0 to 1. |

Example:

```json
{
  "channel": "HiHat",
  "swing": 0.55
}
```

### `channel.set_target_insert`

Sets the mixer insert a channel plays into.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `channel` | ChannelRef, optional | The channel; the selected channel by default. |
| `insert` | InsertRef | The insert it plays into; 0 or "master" for the master. |

Example:

```json
{
  "channel": "Kick",
  "insert": 5
}
```

### `channel.set_volume`

Sets a channel's volume (linear, 1 is unity).

- Effects: undoable
- Calls in one gesture fold into one undo step.

| Param | Type | Description |
|---|---|---|
| `channel` | ChannelRef, optional | The channel; the selected channel by default. |
| `volume` | number | The volume as a linear gain from 0 to 2; 1 is unity (0 dB). |

Example:

```json
{
  "channel": "Kick",
  "volume": 0.8
}
```

## clip

### `clip.move`

Moves clips in time and optionally to another track.

- Effects: undoable
- Calls in one gesture fold into one undo step.

| Param | Type | Description |
|---|---|---|
| `by` | integer, optional | Or: ticks to move every clip by. |
| `clips` | array of ClipRef, optional | The clips; the selected clips by default. |
| `start` | Position, optional | Where the earliest clip goes; the others keep their distance to it. |
| `track` | TrackRef, optional | The track every clip goes to; they stay on their tracks by default. |

Example:

```json
{
  "start": "5:1:0",
  "track": 2
}
```

### `clip.mute`

Mutes or unmutes clips; toggles each without `muted`.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `clips` | array of ClipRef, optional | The clips; the selected clips by default. |
| `muted` | boolean, optional | Muted or not; toggles each clip when left out. |

Example:

```json
{
  "muted": true
}
```

### `clip.place`

Places a pattern on a playlist track; returns the clip's id.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `length` | Length, optional | How long it plays; the pattern's length by default. Longer clips repeat the pattern. |
| `pattern` | PatternRef, optional | The pattern; the current pattern by default. |
| `start` | Position | Where the clip starts. |
| `track` | TrackRef, optional | The track; the selected track (or the first) by default. |

Example:

```json
{
  "pattern": "pattern:1",
  "start": "1:1:0",
  "track": 1
}
```

### `clip.remove`

Removes clips from the playlist.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `clips` | array of ClipRef, optional | The clips; the selected clips by default. |

Example:

```json
{
  "clips": []
}
```

### `clip.resize`

Sets clips' length; a clip longer than its pattern repeats it.

- Effects: undoable
- Calls in one gesture fold into one undo step.

| Param | Type | Description |
|---|---|---|
| `clips` | array of ClipRef, optional | The clips; the selected clips by default. |
| `length` | Length | The new length of every clip. |

Example:

```json
{
  "length": "4 bars"
}
```

## edit

### `edit.deselect`

Clears the selection of channels, clips and notes.

- Effects: none
- Menu: Edit > Deselect

No params.

Example:

```json
{}
```

### `edit.redo`

Redoes the last undone change.

- Effects: none
- Acts on the session, so it can't run inside a batch.
- Menu: Edit > Redo
- Shortcut: Mod+Shift+Z

No params.

Example:

```json
{}
```

### `edit.select`

Selects channels, clips, notes, an insert or a track; commands with empty params act on it.

- Effects: none

| Param | Type | Description |
|---|---|---|
| `add` | boolean, optional | Add to the selection instead of replacing it. |
| `channel` | ChannelRef, optional | The channel the piano roll edits. |
| `channels` | array of ChannelRef, optional | Channels to select in the channel rack. |
| `clips` | array of ClipRef, optional | Clips to select on the playlist. |
| `insert` | InsertRef, optional | The insert to select in the mixer. |
| `notes` | array of integer, optional | Notes of the current pattern to select, by id. |
| `track` | TrackRef, optional | The playlist track to select. |

Example:

```json
{
  "channels": [
    "Kick",
    "Snare"
  ]
}
```

### `edit.select_all`

Selects every channel, every clip, or every note of the current pattern.

- Effects: none
- Menu: Edit > Select all

| Param | Type | Description |
|---|---|---|
| `what` | Kind | What to select. |

Example:

```json
{
  "what": "clips"
}
```

### `edit.undo`

Undoes the last change to the project.

- Effects: none
- Acts on the session, so it can't run inside a batch.
- Menu: Edit > Undo
- Shortcut: Mod+Z

No params.

Example:

```json
{}
```

## mixer

### `mixer.add_insert`

Adds a mixer insert after the last one.

- Effects: undoable
- Menu: Add > Mixer insert

| Param | Type | Description |
|---|---|---|
| `name` | string, optional | Its name; "Insert N" by default. |

Example:

```json
{
  "name": "Drum bus"
}
```

### `mixer.add_route`

Routes a channel or insert into another insert; refuses loops.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `from` | RouteFromRef | Where it starts: a channel or an insert. |
| `level` | number, optional | The send level as a linear gain from 0 to 2; 1 by default. |
| `sidechain` | boolean, optional | Feed only the sidechain inputs of the target's plugins; false by default. |
| `to` | InsertRef | The insert it goes to (never the master; inserts reach it on their own). |

Example:

```json
{
  "from": "insert:1",
  "level": 0.7,
  "to": "insert:5"
}
```

### `mixer.mute`

Mutes or unmutes an insert; toggles without `on`.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `insert` | InsertRef, optional | The insert; the selected insert by default. |
| `on` | boolean, optional | On or off; toggles when left out. |

Example:

```json
{
  "insert": 4
}
```

### `mixer.remove_insert`

Removes an insert; channels that played into it go to the master.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `insert` | InsertRef, optional | The insert; the selected insert by default. |

Example:

```json
{
  "insert": 16
}
```

### `mixer.remove_route`

Removes a route.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `from` | RouteFromRef | Where it starts: a channel or an insert. |
| `to` | InsertRef | The insert it goes to. |

Example:

```json
{
  "from": "insert:1",
  "to": "insert:5"
}
```

### `mixer.rename`

Renames a mixer insert.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `insert` | InsertRef, optional | The insert; the selected insert by default. |
| `name` | string | The new name. |

Example:

```json
{
  "insert": 1,
  "name": "Kick"
}
```

### `mixer.set_color`

Sets an insert's color.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `color` | Color | The new color. |
| `insert` | InsertRef, optional | The insert; the selected insert by default. |

Example:

```json
{
  "color": "#f87171",
  "insert": 1
}
```

### `mixer.set_pan`

Sets an insert's pan from -1 to 1.

- Effects: undoable
- Calls in one gesture fold into one undo step.

| Param | Type | Description |
|---|---|---|
| `insert` | InsertRef, optional | The insert; the selected insert by default. |
| `pan` | number | Pan from -1 (left) to 1 (right). |

Example:

```json
{
  "insert": 2,
  "pan": -0.2
}
```

### `mixer.set_polarity`

Inverts an insert's polarity; toggles without `on`.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `insert` | InsertRef, optional | The insert; the selected insert by default. |
| `on` | boolean, optional | On or off; toggles when left out. |

Example:

```json
{
  "insert": 1,
  "on": true
}
```

### `mixer.set_route`

Sets a route's send level or tap point.

- Effects: undoable
- Calls in one gesture fold into one undo step.

| Param | Type | Description |
|---|---|---|
| `from` | RouteFromRef | Where it starts: a channel or an insert. |
| `level` | number, optional | The send level as a linear gain from 0 to 2. |
| `tap` | Tap, optional | Where on the source strip it taps. |
| `to` | InsertRef | The insert it goes to. |

Example:

```json
{
  "from": "insert:1",
  "level": 0.5,
  "to": "insert:5"
}
```

### `mixer.set_stereo_sep`

Narrows or widens an insert: -1 wider, 0 unchanged, 1 mono.

- Effects: undoable
- Calls in one gesture fold into one undo step.

| Param | Type | Description |
|---|---|---|
| `insert` | InsertRef, optional | The insert; the selected insert by default. |
| `separation` | number | Stereo separation from -1 (twice as wide) through 0 (unchanged) to 1 (mono). |

Example:

```json
{
  "insert": 3,
  "separation": 0.5
}
```

### `mixer.set_swap_lr`

Swaps an insert's channels; toggles without `on`.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `insert` | InsertRef, optional | The insert; the selected insert by default. |
| `on` | boolean, optional | On or off; toggles when left out. |

Example:

```json
{
  "insert": 1,
  "on": true
}
```

### `mixer.set_volume`

Sets an insert's fader in dB.

- Effects: undoable
- Calls in one gesture fold into one undo step.

| Param | Type | Description |
|---|---|---|
| `db` | number | The fader in dB, from -144 (silence) to 6. |
| `insert` | InsertRef, optional | The insert; the selected insert by default. |

Example:

```json
{
  "db": -3.0,
  "insert": 1
}
```

## note

### `note.add_many`

Adds notes for one channel to a pattern and selects them; returns their ids.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `channel` | ChannelRef, optional | The channel that plays them; the piano roll's channel by default. |
| `notes` | array of NoteSpec | The notes to add. |
| `pattern` | PatternRef, optional | The pattern; the current pattern by default. |

Example:

```json
{
  "channel": "Kick",
  "notes": [
    {
      "key": "C4",
      "length": "1 beat",
      "start": "1:1:0"
    },
    {
      "key": 64,
      "start": 960,
      "velocity": 0.6
    }
  ]
}
```

### `note.move`

Moves notes by ticks and semitones.

- Effects: undoable
- Calls in one gesture fold into one undo step.

| Param | Type | Description |
|---|---|---|
| `keys` | integer, optional | Semitones to move them by, up when positive. |
| `notes` | array of integer, optional | The note ids; the selected notes by default. |
| `pattern` | PatternRef, optional | The pattern; the current pattern by default. |
| `ticks` | integer, optional | Ticks to move them by, later when positive. |

Example:

```json
{
  "keys": 2,
  "notes": [
    0
  ],
  "ticks": 240
}
```

### `note.remove`

Deletes notes from a pattern.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `notes` | array of integer, optional | The note ids; the selected notes by default. |
| `pattern` | PatternRef, optional | The pattern; the current pattern by default. |

Example:

```json
{
  "notes": [
    0
  ]
}
```

### `note.resize`

Sets notes' length, or changes it by ticks.

- Effects: undoable
- Calls in one gesture fold into one undo step.

| Param | Type | Description |
|---|---|---|
| `length` | Length, optional | The new length of every note. |
| `notes` | array of integer, optional | The note ids; the selected notes by default. |
| `pattern` | PatternRef, optional | The pattern; the current pattern by default. |
| `ticks` | integer, optional | Or: ticks to add to each note's length (negative shortens; a note keeps at least one tick). |

Example:

```json
{
  "length": "2 steps",
  "notes": [
    0
  ]
}
```

### `note.set_props`

Sets start, key, length, velocity, pan or mute of notes.

- Effects: undoable
- Calls in one gesture fold into one undo step.

| Param | Type | Description |
|---|---|---|
| `key` | Key, optional | A new key for every note. |
| `length` | Length, optional | A new length for every note. |
| `muted` | boolean, optional | Muted or not. |
| `notes` | array of integer, optional | The note ids; the selected notes by default. |
| `pan` | number, optional | A new pan from -1 to 1. |
| `pattern` | PatternRef, optional | The pattern; the current pattern by default. |
| `start` | Position, optional | A new start for every note. |
| `velocity` | number, optional | A new velocity from 0 to 1. |

Example:

```json
{
  "notes": [
    0
  ],
  "velocity": 0.5
}
```

## pattern

### `pattern.clone`

Copies a pattern with its notes into a new current pattern.

- Effects: undoable
- Menu: Patterns > Clone pattern

| Param | Type | Description |
|---|---|---|
| `pattern` | PatternRef, optional | The pattern; the current pattern by default. |

Example:

```json
{}
```

### `pattern.delete`

Deletes a pattern and every clip of it on the playlist.

- Effects: undoable
- Menu: Patterns > Delete pattern

| Param | Type | Description |
|---|---|---|
| `pattern` | PatternRef, optional | The pattern; the current pattern by default. |

Example:

```json
{
  "pattern": "pattern:1"
}
```

### `pattern.new`

Makes a new empty pattern and makes it current.

- Effects: undoable
- Menu: Patterns > New pattern

| Param | Type | Description |
|---|---|---|
| `color` | Color, optional | Its color; the next palette color by default. |
| `name` | string, optional | Its name; "Pattern N" by default. |

Example:

```json
{
  "name": "Verse"
}
```

### `pattern.rename`

Renames a pattern.

- Effects: undoable
- Menu: Patterns > Rename pattern

| Param | Type | Description |
|---|---|---|
| `name` | string | The new name. |
| `pattern` | PatternRef, optional | The pattern; the current pattern by default. |

Example:

```json
{
  "name": "Intro",
  "pattern": "pattern:1"
}
```

### `pattern.set_color`

Sets a pattern's color.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `color` | Color | The new color. |
| `pattern` | PatternRef, optional | The pattern; the current pattern by default. |

Example:

```json
{
  "color": "#60a5fa"
}
```

### `pattern.set_length`

Fixes a pattern's length, or sets it back to auto.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `length` | Length | The length, or "auto" to follow the notes, rounded up to whole bars. |
| `pattern` | PatternRef, optional | The pattern; the current pattern by default. |

Example:

```json
{
  "length": "2 bars"
}
```

### `pattern.set_steps`

Sets a channel's step sequence in a pattern from a string such as x...x...x...x....

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `channel` | ChannelRef | The channel whose steps to set. |
| `pattern` | PatternRef, optional | The pattern; the current pattern by default. |
| `steps` | string | One character per step (a sixteenth note) from the pattern's start: `x` on, `.` off. Spaces and `\|` are ignored, so `"x... x... x... x..."` is four on the beat. Steps after the string are left as they are. |
| `velocity` | number, optional | The velocity of steps that turn on, from 0 to 1; 0.8 by default. |

Example:

```json
{
  "channel": "Kick",
  "steps": "x... x... x... x..."
}
```

### `pattern.toggle_step`

Turns one step of a channel on or off.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `channel` | ChannelRef | The channel. |
| `on` | boolean, optional | On or off; toggles when left out. |
| `pattern` | PatternRef, optional | The pattern; the current pattern by default. |
| `step` | integer | The step, counted from 1. |
| `velocity` | number, optional | The velocity if it turns on, from 0 to 1; 0.8 by default. |

Example:

```json
{
  "channel": "Clap",
  "step": 5
}
```

## playlist

### `playlist.add_track`

Adds a playlist track.

- Effects: undoable
- Menu: Add > Playlist track

| Param | Type | Description |
|---|---|---|
| `at` | integer, optional | Its position, from 1; after the last track by default. |
| `color` | Color, optional | Its color. |
| `name` | string, optional | Its name; "Track N" by default. |

Example:

```json
{
  "name": "Drums"
}
```

### `playlist.remove_track`

Removes a playlist track and its clips.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `track` | TrackRef, optional | The track; the selected track by default. |

Example:

```json
{
  "track": "track:16"
}
```

### `playlist.set_track_props`

Sets a track's name, color, mute, lock or height.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `color` | Color, optional | A new color. |
| `height` | number, optional | Its height in the playlist, from 16 to 400 points. |
| `locked` | boolean, optional | Locked against edits or not. |
| `muted` | boolean, optional | Muted or not. |
| `name` | string, optional | A new name. |
| `track` | TrackRef, optional | The track; the selected track by default. |

Example:

```json
{
  "muted": false,
  "name": "Drums",
  "track": 1
}
```

## project

### `project.new`

Starts a new project from a template, replacing the current one.

- Effects: destructive
- Acts on the session, so it can't run inside a batch.
- Menu: File > New
- Shortcut: Mod+N

| Param | Type | Description |
|---|---|---|
| `discardChanges` | boolean, optional | Drop unsaved changes to the current project; without it, new refuses when there are any. |
| `template` | Template, optional | "basic" (a starter drum kit) or "empty"; basic by default. |

Example:

```json
{
  "template": "basic"
}
```

### `project.open`

Opens a .bodo file or .bodoproj folder, replacing the current project.

- Effects: destructive
- Acts on the session, so it can't run inside a batch.
- Menu: File > Open
- Shortcut: Mod+O

| Param | Type | Description |
|---|---|---|
| `discardChanges` | boolean, optional | Drop unsaved changes to the current project; without it, open refuses when there are any. |
| `path` | string | The `.bodo` file or `.bodoproj` folder. |

Example:

```json
{
  "path": "My Song.bodo"
}
```

### `project.save`

Saves the project to its file, or to path.

- Effects: disk
- Acts on the session, so it can't run inside a batch.
- Menu: File > Save
- Shortcut: Mod+S

| Param | Type | Description |
|---|---|---|
| `path` | string, optional | Where to save: a `.bodo` file, or a `.bodoproj` folder for git. The project's file by default. A path without an extension gets `.bodo`; a relative one starts at the project's folder. |

Example:

```json
{
  "path": "My Song.bodo"
}
```

### `project.save_as`

Saves the project to a new file, which becomes its file.

- Effects: disk
- Acts on the session, so it can't run inside a batch.
- Menu: File > Save as
- Shortcut: Mod+Shift+S

| Param | Type | Description |
|---|---|---|
| `path` | string | Where to save: a `.bodo` file, or a `.bodoproj` folder for git. A path without an extension gets `.bodo`; a relative one starts at the project's folder. |

Example:

```json
{
  "path": "My Song.bodo"
}
```

### `project.set_info`

Sets the song's title and author.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `author` | string, optional | Its author. |
| `title` | string, optional | The song's title. |

Example:

```json
{
  "author": "hasan-muzak",
  "title": "First Beat"
}
```

### `project.set_meter`

Sets the time signature at the start of the song.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `denominator` | integer | The beat unit: 1, 2, 4, 8, 16 or 32. |
| `numerator` | integer | Beats per bar, from 1 to 64. |

Example:

```json
{
  "denominator": 4,
  "numerator": 3
}
```

### `project.set_tempo`

Sets the tempo at the start of the song.

- Effects: undoable
- Calls in one gesture fold into one undo step.

| Param | Type | Description |
|---|---|---|
| `bpm` | number | The tempo at the start of the song, in beats per minute. |

Example:

```json
{
  "bpm": 128.0
}
```

## transport

### `transport.play`

Starts playing from the song position.

- Effects: none

No params.

Example:

```json
{}
```

### `transport.set_mode`

Chooses Pattern mode (loop the current pattern) or Song mode (play the playlist).

- Effects: none

| Param | Type | Description |
|---|---|---|
| `mode` | PlayMode | "pattern" loops the current pattern; "song" plays the playlist. |

Example:

```json
{
  "mode": "song"
}
```

### `transport.set_position`

Moves the playhead.

- Effects: none

| Param | Type | Description |
|---|---|---|
| `at` | Position | The new song position. |

Example:

```json
{
  "at": "2:1:0"
}
```

### `transport.stop`

Stops playing and returns to the start.

- Effects: none

No params.

Example:

```json
{}
```

### `transport.toggle_play`

Plays when stopped and stops when playing.

- Effects: none
- Shortcut: Space

No params.

Example:

```json
{}
```

## ui

### `ui.select_pattern`

Makes a pattern current; Pattern mode plays it and the channel rack shows it.

- Effects: none

| Param | Type | Description |
|---|---|---|
| `offset` | integer, optional | Or: move this many patterns forward (or back, when negative) from the current one. |
| `pattern` | PatternRef, optional | The pattern to make current. |

Example:

```json
{
  "offset": 1
}
```
