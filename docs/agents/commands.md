# Commands

Every command registered in `bodo-engine`, generated from the registry by `cargo test -p bodo-engine --test docs` (set `BODO_BLESS=1` to rewrite it). MCP serves the same text and generates its tools from the same registry ([ADR 0005](../adr/0005-everything-is-a-command.md)); AGENTS.md section 7 explains how to add a command.

Conventions:

- Empty params act on the current selection: the current pattern, the piano roll's channel, the selected channels, clips, notes, insert or track.
- Positions are ticks (960 per quarter note) or text: `"bar:beat:tick"` counted from 1, or seconds such as `"1.5s"`. Lengths are ticks or text such as `"2 bars"`, `"1 beat"` or `"4 steps"`.
- Channels, patterns, tracks and inserts are named by id, by number (`"channel:3"`, `"insert:0"` for the master, or just `3`) or by name. An ambiguous name is an error that lists the candidates.
- Effects: `undoable` commands change the project and make one undo step; `destructive` ones can lose unsaved work; `disk` ones write files.

## automation

### `automation.add_point`

Adds a point to an automation channel.

- Effects: undoable
- Calls in one gesture fold into one undo step.

| Param | Type | Description |
|---|---|---|
| `at` | Position | Where the point is, from the clip's start. |
| `channel` | ChannelRef | The automation channel. |
| `curve` | Shape, optional | How the value moves on to the next point: `"linear"` (the default), `"hold"`, `"sine"`, `"pulse"`, `{"exp": t}` or `{"sCurve": t}` with t from -1 to 1, or `{"stairs": n}`. |
| `value` | number | The value, 0 to 1. |

Example:

```json
{
  "at": "2:1:0",
  "channel": "Insert 1 volume",
  "value": 0.6
}
```

### `automation.clear`

Removes every point of an automation channel.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `channel` | ChannelRef | The automation channel. |

Example:

```json
{
  "channel": "Insert 1 volume"
}
```

### `automation.create_for`

Makes an automation channel for a parameter and a clip of it on the playlist, starting at the parameter's current value.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `length` | Length, optional | How long it lasts; 4 bars by default. |
| `name` | string, optional | The automation channel's name; the target's by default, such as "Insert 1 volume". |
| `start` | Position, optional | Where the clip starts; the start of the song by default. |
| `target` | Target | What the clip controls: `"tempo"`, `{"insert": 1, "param": "volume"}` (or pan, stereoSep), `{"channel": "Kick", "param": "pan"}` (or volume, pitch, cc:74, pitchBend, pressure), or `{"route": {"from": "insert:1", "to": "insert:5"}}`. |
| `track` | TrackRef, optional | The track it goes on; by default the first track with room for it. |

Example:

```json
{
  "length": "4 bars",
  "target": {
    "insert": 1,
    "param": "volume"
  }
}
```

### `automation.place_clip`

Places another clip of an automation channel on the playlist; every clip of the channel plays its one lane.

- Effects: undoable
- Calls in one gesture fold into one undo step.

| Param | Type | Description |
|---|---|---|
| `channel` | ChannelRef | The automation channel. |
| `length` | Length, optional | How long it lasts; up to the lane's last point by default, or 4 bars when every point is at its start. |
| `start` | Position | Where the clip starts. |
| `track` | TrackRef, optional | The track; the selected track (or the first) by default. |

Example:

```json
{
  "channel": "Insert 1 volume",
  "start": "5:1:0",
  "track": 2
}
```

### `automation.remove_lane`

Removes a pattern's lane for a parameter.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `pattern` | PatternRef, optional | The pattern; the current pattern by default. |
| `target` | Target | What the lane controls, as in automation.create_for. |

Example:

```json
{
  "pattern": "Verse",
  "target": {
    "channel": "HiHat",
    "param": "pan"
  }
}
```

### `automation.remove_point`

Removes a point from an automation channel, by index or time.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `at` | Position, optional | Or: the time of the point to remove. |
| `channel` | ChannelRef | The automation channel. |
| `index` | integer, optional | The point's index, from 0 in time order. |

Example:

```json
{
  "channel": "Insert 1 volume",
  "index": 1
}
```

### `automation.set_curve`

Sets how the value moves from a point to the next one.

- Effects: undoable
- Calls in one gesture fold into one undo step.

| Param | Type | Description |
|---|---|---|
| `channel` | ChannelRef | The automation channel. |
| `curve` | Shape | The curve: `"linear"`, `"hold"`, `"sine"`, `"pulse"`, `{"exp": t}` or `{"sCurve": t}` with t from -1 to 1, or `{"stairs": n}`. |
| `index` | integer | The point's index, from 0 in time order; the curve runs from it to the next point. |

Example:

```json
{
  "channel": "Insert 1 volume",
  "curve": "sine",
  "index": 0
}
```

### `automation.set_lane`

Sets a pattern's own lane for a parameter, which repeats with the pattern.

- Effects: undoable
- Calls in one gesture fold into one undo step.

| Param | Type | Description |
|---|---|---|
| `pattern` | PatternRef, optional | The pattern; the current pattern by default. |
| `points` | array of PointSpec | Every point, from the pattern's start; they are sorted by time. |
| `target` | Target | What the lane controls, as in automation.create_for. |

Example:

```json
{
  "pattern": "Verse",
  "points": [
    {
      "at": 0,
      "value": 0.3
    },
    {
      "at": "1:3:0",
      "value": 0.7
    }
  ],
  "target": {
    "channel": "HiHat",
    "param": "pan"
  }
}
```

### `automation.set_points`

Replaces an automation channel's points; times are from the clip's start, values 0 to 1.

- Effects: undoable
- Calls in one gesture fold into one undo step.

| Param | Type | Description |
|---|---|---|
| `channel` | ChannelRef | The automation channel. |
| `points` | array of PointSpec | Every point, in any order; they are sorted by time. |

Example:

```json
{
  "channel": "Insert 1 volume",
  "points": [
    {
      "at": 0,
      "value": 0.8
    },
    {
      "at": "3:1:0",
      "curve": {
        "exp": 0.3
      },
      "value": 0.4
    },
    {
      "at": "5:1:0",
      "value": 0.8
    }
  ]
}
```

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

### `channel.add_audio`

Adds a channel that plays an audio file as clips on the playlist.

- Effects: undoable
- Menu: Add > Audio clip channel

| Param | Type | Description |
|---|---|---|
| `name` | string, optional | Its name; the file's name by default. |
| `sample` | string | The audio file: a `factory:` path from browser.search, an absolute path, or a path inside the project's folder. |
| `target` | InsertRef, optional | The mixer insert it plays into; the master by default. |
| `tempo` | number, optional | The sample's own tempo in BPM, for resample and stretch; the project's current tempo by default. |

Example:

```json
{
  "name": "Crash",
  "sample": "factory:Packs/Bodo Drums/Crash.wav"
}
```

### `channel.add_layer`

Adds a layer channel, which plays every one of its children on each note.

- Effects: undoable
- Menu: Add > Layer

| Param | Type | Description |
|---|---|---|
| `children` | array of ChannelRef, optional | The channels it plays; the selected channels by default. |
| `name` | string, optional | Its name; "Layer" by default. |

Example:

```json
{
  "children": [
    "Kick",
    "Clap"
  ],
  "name": "Big drums"
}
```

### `channel.add_midi_out`

Adds a channel that sends its notes to an external MIDI port.

- Effects: undoable
- Menu: Add > MIDI Out

| Param | Type | Description |
|---|---|---|
| `midiChannel` | integer, optional | The MIDI channel, 1 to 16; 1 by default. |
| `name` | string, optional | Its name; the port's name by default. |
| `port` | string | The MIDI output port it sends to, by name. |

Example:

```json
{
  "midiChannel": 2,
  "port": "IAC Bus 1"
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

### `channel.rename_group`

Renames a channel rack group.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `group` | string | The group's current name. |
| `name` | string | Its new name. |

Example:

```json
{
  "group": "Drums",
  "name": "Beat"
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

### `channel.set_group`

Puts channels in a channel rack group, or leaves them unsorted.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `channels` | array of ChannelRef, optional | The channels; the selected channels by default. |
| `group` | string, optional | The group they join, by name; null or an empty name leaves them unsorted. |

Example:

```json
{
  "channels": [
    "Kick",
    "Snare"
  ],
  "group": "Drums"
}
```

### `channel.set_layer`

Sets the channels a layer plays.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `channel` | ChannelRef, optional | The layer channel; the selected channel by default. |
| `children` | array of ChannelRef | Every channel it plays, replacing the ones it had. |

Example:

```json
{
  "channel": "Big drums",
  "children": [
    "Kick",
    "Snare"
  ]
}
```

### `channel.set_midi_in`

Binds a channel to a MIDI input port and optionally one MIDI channel, or removes the binding.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `channel` | ChannelRef, optional | The channel; the selected channel by default. |
| `midiChannel` | integer, optional | The MIDI channel to listen to, 1 to 16; every channel when null or left out. |
| `port` | string, optional | The MIDI input port's name, as the system lists it; null or left out removes the binding. |

Example:

```json
{
  "channel": "Kick",
  "midiChannel": 10,
  "port": "Pads"
}
```

### `channel.set_midi_out`

Sets the port and MIDI channel a MIDI Out channel sends to.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `channel` | ChannelRef, optional | The MIDI Out channel; the selected channel by default. |
| `midiChannel` | integer, optional | The MIDI channel, 1 to 16. |
| `port` | string, optional | The MIDI output port it sends to. |

Example:

```json
{
  "channel": "IAC Bus 1",
  "midiChannel": 10
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
| `channels` | array of ChannelRef, optional | Or: several channels at once, such as every channel of a group. |
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

### `clip.consolidate`

Renders clips to a WAV in the project's Rendered folder and replaces them with one audio clip.

- Effects: undoable, disk

| Param | Type | Description |
|---|---|---|
| `clips` | array of ClipRef, optional | The clips; the selected clips by default. Automation clips are left as they are. |
| `name` | string, optional | The new channel's name; "Consolidated" by default. |

Example:

```json
{
  "name": "Verse audio"
}
```

### `clip.make_unique`

Gives a pattern clip its own copy of the pattern, to edit without changing the other clips.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `clip` | ClipRef, optional | The pattern clip; the selected clip by default. |

Example:

```json
{}
```

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
- Calls in one gesture fold into one undo step.

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
- Calls in one gesture fold into one undo step.

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

### `clip.place_audio`

Places a clip of an audio clip channel on a playlist track; it lasts the whole sample by default.

- Effects: undoable
- Calls in one gesture fold into one undo step.

| Param | Type | Description |
|---|---|---|
| `channel` | ChannelRef | The audio clip channel whose sample the clip plays. |
| `length` | Length, optional | How long it plays; the whole sample by default. |
| `start` | Position | Where the clip starts. |
| `track` | TrackRef, optional | The track; the selected track (or the first) by default. |

Example:

```json
{
  "channel": "Crash",
  "start": "2:1:0",
  "track": 3
}
```

### `clip.place_sound`

Places an audio file on a playlist track as an audio clip, reusing the channel that already plays it or making one.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `sample` | string | The audio file: a `factory:` path, an absolute path, or a path inside the project's folder. |
| `start` | Position | Where the clip starts. |
| `track` | TrackRef, optional | The track; the selected track (or the first) by default. |

Example:

```json
{
  "sample": "factory:Packs/Bodo Drums/Crash.wav",
  "start": "1:1:0",
  "track": 3
}
```

### `clip.remove`

Removes clips from the playlist.

- Effects: undoable
- Calls in one gesture fold into one undo step.

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

### `clip.set_color`

Colors clips, or gives them their source's color again with null.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `clips` | array of ClipRef, optional | The clips; the selected clips by default. |
| `color` | Color, optional | The color, or null to use the source's color again. |

Example:

```json
{
  "color": "#22d3ee"
}
```

### `clip.set_fades`

Sets the fade-in and fade-out of clips; 0 removes a fade.

- Effects: undoable
- Calls in one gesture fold into one undo step.

| Param | Type | Description |
|---|---|---|
| `clips` | array of ClipRef, optional | The clips; the selected clips by default. |
| `fadeIn` | Length, optional | The fade-in's length; 0 removes it, and it is unchanged when left out. |
| `fadeOut` | Length, optional | The fade-out's length; 0 removes it, and it is unchanged when left out. |

Example:

```json
{
  "fadeIn": "1 beat",
  "fadeOut": 0
}
```

### `clip.set_stretch`

Sets how an audio clip channel follows the tempo: none, resample or stretch.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `channel` | ChannelRef | The audio clip channel. |
| `mode` | Mode | How its clips follow the tempo. |
| `tempo` | number, optional | The sample's own tempo in BPM; unchanged when left out. |

Example:

```json
{
  "channel": "Crash",
  "mode": "stretch",
  "tempo": 120.0
}
```

### `clip.slice`

Cuts clips in two at a position; both halves keep playing the same source.

- Effects: undoable
- Calls in one gesture fold into one undo step.

| Param | Type | Description |
|---|---|---|
| `at` | Position | Where to cut. |
| `clips` | array of ClipRef, optional | Cut only these clips; by default every clip the cut crosses on `tracks`, or else the selected clips. |
| `tracks` | array of TrackRef, optional | Cut every clip these tracks have at that point. |

Example:

```json
{
  "at": "4:1:0",
  "tracks": [
    1
  ]
}
```

### `clip.slip`

Slides the content inside clips without moving or resizing them.

- Effects: undoable
- Calls in one gesture fold into one undo step.

| Param | Type | Description |
|---|---|---|
| `by` | integer | Ticks to slide the content by: positive plays later material, negative earlier. Pattern content wraps around the pattern; audio stops at its start. |
| `clips` | array of ClipRef, optional | The clips; the selected clips by default. |

Example:

```json
{
  "by": 240
}
```

## edit

### `edit.copy`

Copies the selected clips, the selected notes, or a time selection of the playlist, for edit.paste.

- Effects: none
- Menu: Edit > Copy
- Shortcut: Mod+C

| Param | Type | Description |
|---|---|---|
| `end` | Position, optional | Where the time selection ends. |
| `start` | Position, optional | With `end`: a time selection of the playlist. Every track's clips are copied (or cut) between the two, trimmed at the edges, instead of the selection. |
| `what` | CopyKind, optional | What to copy; the selected clips, or else the selected notes, by default. |

Example:

```json
{
  "what": "clips"
}
```

### `edit.cut`

Copies the selected clips or notes and removes them.

- Effects: undoable
- Menu: Edit > Cut
- Shortcut: Mod+X

| Param | Type | Description |
|---|---|---|
| `end` | Position, optional | Where the time selection ends. |
| `start` | Position, optional | With `end`: a time selection of the playlist. Every track's clips are copied (or cut) between the two, trimmed at the edges, instead of the selection. |
| `what` | CopyKind, optional | What to copy; the selected clips, or else the selected notes, by default. |

Example:

```json
{}
```

### `edit.deselect`

Clears the selection of channels, clips and notes.

- Effects: none
- Menu: Edit > Deselect

No params.

Example:

```json
{}
```

### `edit.paste`

Pastes what was copied: clips onto the playlist, notes into the current pattern.

- Effects: undoable
- Menu: Edit > Paste
- Shortcut: Mod+V

| Param | Type | Description |
|---|---|---|
| `at` | Position, optional | Where the copied items start; right after where they were copied from by default. |
| `track` | TrackRef, optional | For clips: the track the first copied track lands on; the same track by default. |

Example:

```json
{
  "at": "9:1:0"
}
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
| `inserts` | array of InsertRef, optional | Mixer inserts to select together, for editing them as a group; the last one becomes the selected insert. |
| `notes` | array of integer, optional | Notes of the current pattern to select, by id. |
| `toggle` | boolean, optional | Flip each given item in or out of the selection, as Shift does with the select tool. |
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

## fx

### `fx.copy_chain`

Copies an insert's FX slots, for fx.paste_chain.

- Effects: none

| Param | Type | Description |
|---|---|---|
| `insert` | InsertRef, optional | The insert whose effects to copy; the selected insert by default. |

Example:

```json
{
  "insert": 2
}
```

### `fx.load`

Loads a native effect into an FX slot, the first empty one by default.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `effect` | string | The effect by uid or name, such as `bodo.delay` or `Delay`. |
| `insert` | InsertRef, optional | The insert; the selected insert by default. |
| `params` | object, optional | Parameter values, by id or name, in each parameter's unit; the others start at their defaults. |
| `slot` | integer, optional | The slot, 1 to 10; the first empty slot by default. |

Example:

```json
{
  "effect": "bodo.ducker",
  "insert": 2,
  "params": {
    "Depth": 18
  },
  "slot": 1
}
```

### `fx.move`

Moves an FX slot to another place in the chain; the slots between shift.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `from` | integer | The slot to move, 1 to 10. |
| `insert` | InsertRef, optional | The insert; the selected insert by default. |
| `to` | integer | Where it goes, 1 to 10; the slots between shift over. |

Example:

```json
{
  "from": 1,
  "insert": 2,
  "to": 3
}
```

### `fx.paste_chain`

Replaces the FX slots of one or more inserts with the copied ones, each effect a new instance.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `inserts` | array of InsertRef, optional | The inserts to paste into; the selected inserts by default. |

Example:

```json
{
  "inserts": [
    3,
    4
  ]
}
```

### `fx.remove`

Empties an FX slot.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `insert` | InsertRef, optional | The insert; the selected insert by default. |
| `slot` | integer | The slot, 1 to 10. |

Example:

```json
{
  "insert": 2,
  "slot": 1
}
```

### `fx.set_enabled`

Turns an FX slot on or off; a slot that is off keeps its latency. Toggles without `on`.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `insert` | InsertRef, optional | The insert; the selected insert by default. |
| `on` | boolean, optional | On or off; toggles when left out. |
| `slot` | integer | The slot, 1 to 10. |

Example:

```json
{
  "insert": 2,
  "on": false,
  "slot": 1
}
```

### `fx.set_mix`

Sets an FX slot's dry/wet mix.

- Effects: undoable
- Calls in one gesture fold into one undo step.

| Param | Type | Description |
|---|---|---|
| `insert` | InsertRef, optional | The insert; the selected insert by default. |
| `mix` | number | Dry/wet mix, 0 (dry) to 1 (wet). |
| `slot` | integer | The slot, 1 to 10. |

Example:

```json
{
  "insert": 2,
  "mix": 0.5,
  "slot": 1
}
```

### `fx.set_sidechain`

Feeds an FX slot's sidechain input from a channel or insert, adding the sidechain route.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `from` | RouteFromRef, optional | The channel or insert to listen to; null to stop. A sidechain route is added when there is none. |
| `insert` | InsertRef, optional | The insert; the selected insert by default. |
| `slot` | integer | The slot, 1 to 10. |

Example:

```json
{
  "from": "insert:1",
  "insert": 2,
  "slot": 1
}
```

### `fx.swap`

Swaps two FX slots.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `a` | integer | One slot, 1 to 10. |
| `b` | integer | The other slot, 1 to 10. |
| `insert` | InsertRef, optional | The insert; the selected insert by default. |

Example:

```json
{
  "a": 1,
  "b": 2,
  "insert": 2
}
```

## library

### `library.list`

Lists a folder of the user library: sounds, projects, presets and scores.

- Effects: none

| Param | Type | Description |
|---|---|---|
| `folder` | string, optional | The folder: one of the library's folders ("Packs", "My projects", "Templates"), a preset folder ("Channel presets", "Mixer presets", "Scores", "Plugin presets"), "Marketplace/Plugins" or "Marketplace/Stems", a `library:` key, or an absolute path. Empty lists the library's folders. |

Example:

```json
{
  "folder": "Channel presets"
}
```

### `library.search`

Finds files of any kind in the factory content, the sample folders and the user library, by words and tags.

- Effects: none

| Param | Type | Description |
|---|---|---|
| `limit` | integer, optional | The most results, from 1 to 500; 50 by default. |
| `query` | string, optional | Words the file's path or tags must contain, ignoring case. Empty matches everything. |
| `tags` | array of string, optional | Tags the file must have, every one. |

Example:

```json
{
  "query": "kick",
  "tags": [
    "bodo drums"
  ]
}
```

### `library.tag`

Adds tags to a library file, or removes them; the Browser filters by them.

- Effects: disk

| Param | Type | Description |
|---|---|---|
| `add` | array of string, optional | Tags to give it. |
| `path` | string | The file: an absolute path, a `library:` key, or a `factory:` path. |
| `remove` | array of string, optional | Tags to take from it. |

Example:

```json
{
  "add": [
    "punchy"
  ],
  "path": "factory:Packs/Bodo Drums/Kick.wav"
}
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

### `mixer.adjust`

Moves the faders, pans or stereo separation of several inserts together, by the same amount.

- Effects: undoable
- Calls in one gesture fold into one undo step.

| Param | Type | Description |
|---|---|---|
| `inserts` | array of InsertRef, optional | The inserts; the selected inserts by default. |
| `pan` | number, optional | Amount to add to each pan, clamped to -1..1. |
| `separation` | number, optional | Amount to add to each stereo separation, clamped to -1..1. |
| `volumeDb` | number, optional | dB to add to each fader, clamped to -144..6. |

Example:

```json
{
  "inserts": [
    1,
    2
  ],
  "volumeDb": -1.5
}
```

### `mixer.arm`

Arms an insert for recording, or disarms it; toggles without `on`. An armed insert monitors its input.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `insert` | InsertRef, optional | The insert; the selected insert by default. |
| `on` | boolean, optional | Armed or not; toggles when left out. |

Example:

```json
{
  "insert": 1,
  "on": true
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

### `mixer.reorder_insert`

Moves an insert to another place in the mixer.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `insert` | InsertRef, optional | The insert to move (not the master). |
| `to` | integer | Its new number, from 1. |

Example:

```json
{
  "insert": 3,
  "to": 1
}
```

### `mixer.route_exclusive`

Routes a channel or insert to one insert, or the master, only, removing its other audio routes.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `from` | RouteFromRef | Where it starts: a channel or an insert. |
| `to` | InsertRef | The insert it goes to, or the master. |

Example:

```json
{
  "from": "insert:1",
  "to": "insert:5"
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

### `mixer.set_eq`

Sets the frequency, gain or Q of a band of an insert's EQ.

- Effects: undoable
- Calls in one gesture fold into one undo step.

| Param | Type | Description |
|---|---|---|
| `band` | Band | Which band. |
| `frequency` | number, optional | The band's frequency in Hz, from 20 to 20000. |
| `gainDb` | number, optional | Its gain in dB, from -24 to 24. |
| `insert` | InsertRef, optional | The insert; the selected insert by default. |
| `q` | number, optional | Its Q (for shelves, the slope), from 0.1 to 10. |

Example:

```json
{
  "band": "mid",
  "frequency": 2500,
  "gainDb": -3.0,
  "insert": 1,
  "q": 1.4
}
```

### `mixer.set_icon`

Sets the icon an insert shows, by name, or none.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `icon` | string, optional | The icon's name, such as `drum`, `guitar` or `microphone`; null for none. |
| `insert` | InsertRef, optional | The insert; the selected insert by default. |

Example:

```json
{
  "icon": "drum",
  "insert": 1
}
```

### `mixer.set_input`

Sets the device input channels an insert records and monitors, or none.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `channels` | array, optional | The device input channels it records and monitors, counted from 0: one for mono, two for stereo. Null for none. |
| `insert` | InsertRef, optional | The insert; the selected insert by default. |

Example:

```json
{
  "channels": [
    0,
    1
  ],
  "insert": 1
}
```

### `mixer.set_latency_offset`

Sets an insert's track latency: positive declares latency, negative delays the insert.

- Effects: undoable
- Calls in one gesture fold into one undo step.

| Param | Type | Description |
|---|---|---|
| `frames` | integer, optional | The track latency in frames: positive declares latency the insert has (everything else waits for it), negative delays the insert. |
| `insert` | InsertRef, optional | The insert; the selected insert by default. |
| `ms` | number, optional | The same in milliseconds, at the project's sample rate. |

Example:

```json
{
  "insert": 1,
  "ms": -2.5
}
```

### `mixer.set_output`

Sends an insert straight to device outputs as well, or stops.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `channels` | array, optional | Device output channels, counted from 0: one for mono, two for stereo; null for no direct output. |
| `insert` | InsertRef, optional | The insert; the selected insert by default. |

Example:

```json
{
  "channels": [
    2,
    3
  ],
  "insert": 3
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

### `mixer.set_pdc_bypass`

Sets when an insert skips delay compensation: never, always or whileRecording.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `insert` | InsertRef, optional | The insert; the selected insert by default. |
| `mode` | Bypass | When to bypass delay compensation. |

Example:

```json
{
  "insert": 1,
  "mode": "whileRecording"
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

### `mixer.set_record_format`

Sets the file format an insert records in.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `format` | Format | The file format. |
| `insert` | InsertRef, optional | The insert; the selected insert by default. |

Example:

```json
{
  "format": "int24",
  "insert": 1
}
```

### `mixer.set_record_tap`

Sets where on its strip an armed insert records from.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `insert` | InsertRef, optional | The insert; the selected insert by default. |
| `tap` | Tap | Where on the strip it records. |

Example:

```json
{
  "insert": 1,
  "tap": "input"
}
```

### `mixer.set_route`

Sets a route's send level, tap point, or whether it only feeds sidechains.

- Effects: undoable
- Calls in one gesture fold into one undo step.

| Param | Type | Description |
|---|---|---|
| `from` | RouteFromRef | Where it starts: a channel or an insert. |
| `level` | number, optional | The send level as a linear gain from 0 to 2. |
| `sidechain` | boolean, optional | True to feed only the target's plugin sidechains, false to carry audio. |
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

### `mixer.set_to_master`

Sets whether an insert feeds the master; toggles without `on`.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `insert` | InsertRef, optional | The insert; the selected insert by default. |
| `on` | boolean, optional | On or off; toggles when left out. |

Example:

```json
{
  "insert": 2,
  "on": false
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

### `mixer.solo`

Solos or unsolos an insert, alone with `exclusive`; toggles without `on`.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `exclusive` | boolean, optional | Solo this insert alone, unsoloing every other. |
| `insert` | InsertRef, optional | The insert; the selected insert by default. |
| `on` | boolean, optional | On or off; toggles when left out. |

Example:

```json
{
  "exclusive": true,
  "insert": 1
}
```

## note

### `note.add_many`

Adds notes for one channel to a pattern and selects them; returns their ids.

- Effects: undoable
- Calls in one gesture fold into one undo step.

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

### `note.arpeggiate`

Turns each chord into a run of single notes over its length.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `gate` | number, optional | How long each note lasts, as a fraction of the rate from 0 to 1; 1 by default. |
| `notes` | array of integer, optional | The note ids; the selected notes by default. |
| `order` | ArpOrder, optional | The order; up by default. |
| `pattern` | PatternRef, optional | The pattern; the current pattern by default. |
| `rate` | Length, optional | How often a new note starts; one step by default. |
| `seed` | integer, optional | The seed of the random order; 1 by default. |

Example:

```json
{
  "gate": 0.5,
  "notes": [
    0
  ],
  "order": "up",
  "rate": "120 ticks"
}
```

### `note.articulate`

Sets each note's length to a share of the time until the next one, from staccato to legato.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `amount` | number | How much of the time until the next note each note lasts, from 0.05 (staccato) to 1 (legato). The last note keeps that share of its own length. |
| `notes` | array of integer, optional | The note ids; the selected notes by default. |
| `pattern` | PatternRef, optional | The pattern; the current pattern by default. |

Example:

```json
{
  "amount": 0.5,
  "notes": [
    0,
    1
  ]
}
```

### `note.chop`

Splits notes into pieces of a length.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `length` | Length | The length of each piece. |
| `notes` | array of integer, optional | The note ids; the selected notes by default. |
| `pattern` | PatternRef, optional | The pattern; the current pattern by default. |

Example:

```json
{
  "length": "120 ticks",
  "notes": [
    0
  ]
}
```

### `note.flip`

Reverses notes in time, or mirrors their keys.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `axis` | Axis | Time or keys. |
| `notes` | array of integer, optional | The note ids; the selected notes by default. |
| `pattern` | PatternRef, optional | The pattern; the current pattern by default. |

Example:

```json
{
  "axis": "keys",
  "notes": [
    0,
    1
  ]
}
```

### `note.glue`

Joins notes of the same key and channel that touch or overlap.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `notes` | array of integer, optional | The note ids; the selected notes by default. |
| `pattern` | PatternRef, optional | The pattern; the current pattern by default. |

Example:

```json
{
  "notes": [
    0,
    1
  ]
}
```

### `note.humanize`

Moves starts by a player-like drift and varies velocities, from a seed.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `notes` | array of integer, optional | The note ids; the selected notes by default. |
| `pattern` | PatternRef, optional | The pattern; the current pattern by default. |
| `seed` | integer, optional | The seed; the same seed gives the same performance. 1 by default. |
| `timingMs` | number, optional | How far starts drift, in milliseconds (the drift's standard deviation), from 0 to 200; 10 by default. |
| `velocity` | number, optional | How much velocities vary, as a fraction from 0 to 1; 0.1 by default. |

Example:

```json
{
  "notes": [
    0,
    1
  ],
  "seed": 7,
  "timingMs": 10,
  "velocity": 0.1
}
```

### `note.legato`

Lengthens each note to the start of the next one on its channel.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `notes` | array of integer, optional | The note ids; the selected notes by default. |
| `pattern` | PatternRef, optional | The pattern; the current pattern by default. |

Example:

```json
{
  "notes": [
    0,
    1
  ]
}
```

### `note.limit_to_scale`

Moves notes to the nearest key of a scale.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `notes` | array of integer, optional | The note ids; the selected notes by default. |
| `pattern` | PatternRef, optional | The pattern; the current pattern by default. |
| `root` | Root | The scale's root, such as "C" or "F#". |
| `scale` | string | The scale: major, minor, harmonic minor, melodic minor, dorian, phrygian, lydian, mixolydian, locrian, major pentatonic, minor pentatonic, blues, whole tone or chromatic. |

Example:

```json
{
  "notes": [
    0,
    1
  ],
  "root": "E",
  "scale": "major"
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

### `note.quantize`

Moves notes toward a grid, with strength and swing; remembers the grid position for un-humanize.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `ends` | boolean, optional | Quantize the ends too, not only the starts. |
| `grid` | Length, optional | The grid; one step (a sixteenth note) by default. |
| `notes` | array of integer, optional | The note ids; the selected notes by default. |
| `pattern` | PatternRef, optional | The pattern; the current pattern by default. |
| `strength` | number, optional | How far notes move toward the grid, from 0 (not at all) to 1 (onto it); 1 by default. |
| `swing` | number, optional | Moves every second grid line later by up to half a grid, from 0 to 1; 0 by default. |

Example:

```json
{
  "grid": "1 beat",
  "notes": [
    0,
    1
  ],
  "strength": 0.5
}
```

### `note.randomize`

Fills a stretch of a pattern with random notes of a scale on a grid, from a seed.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `channel` | ChannelRef, optional | The channel that plays the notes; the piano roll's channel by default. |
| `density` | number, optional | The share of grid steps that get a note, from 0 to 1; 0.5 by default. |
| `high` | integer, optional | The highest key; C5 (72) by default. |
| `length` | Length, optional | How long a stretch to fill; the pattern's length by default. |
| `low` | integer, optional | The lowest key; C4 (60) by default. |
| `pattern` | PatternRef, optional | The pattern; the current pattern by default. |
| `root` | Root, optional | The scale's root: a note name such as "A" or "F#", or a MIDI key; C by default. |
| `scale` | string, optional | The scale, such as "minor" or "dorian"; major by default. |
| `seed` | integer, optional | The seed: the same seed gives the same notes. 1 by default. |
| `start` | Position, optional | Where the notes may start; the pattern's start by default. |
| `step` | Length, optional | The grid the notes start on, and their length; one step by default. |

Example:

```json
{
  "channel": "Kick",
  "density": 0.5,
  "length": "1 bar",
  "root": "A",
  "scale": "minor",
  "seed": 7
}
```

### `note.remove`

Deletes notes from a pattern.

- Effects: undoable
- Calls in one gesture fold into one undo step.

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

### `note.set_expression`

Sets a note's pitch, pressure and timbre curves (MPE).

- Effects: undoable
- Calls in one gesture fold into one undo step.

| Param | Type | Description |
|---|---|---|
| `note` | integer | The note's id. |
| `pattern` | PatternRef, optional | The pattern; the current pattern by default. |
| `pitch` | array, optional | The pitch curve in semitones; an empty list clears it, leaving it out keeps it. |
| `pressure` | array, optional | The pressure curve, 0 to 1. |
| `timbre` | array, optional | The timbre (MPE slide) curve, 0 to 1. |

Example:

```json
{
  "note": 1,
  "pitch": [
    {
      "at": 0,
      "value": 0.0
    },
    {
      "at": 120,
      "value": 1.0
    }
  ]
}
```

### `note.set_props`

Sets start, key, length, velocity, release velocity, pan, fine pitch, Mod X and Y, slide, portamento or mute of notes.

- Effects: undoable
- Calls in one gesture fold into one undo step.

| Param | Type | Description |
|---|---|---|
| `color` | integer, optional | A new note color group, 0 to 15, which instruments may read and the piano roll filters by. |
| `finePitchCents` | number, optional | A new fine pitch in cents, from -1200 to 1200. |
| `key` | Key, optional | A new key for every note. |
| `length` | Length, optional | A new length for every note. |
| `modX` | number, optional | A new Mod X from 0 to 1. |
| `modY` | number, optional | A new Mod Y from 0 to 1. |
| `muted` | boolean, optional | Muted or not. |
| `notes` | array of integer, optional | The note ids; the selected notes by default. |
| `pan` | number, optional | A new pan from -1 to 1. |
| `pattern` | PatternRef, optional | The pattern; the current pattern by default. |
| `porta` | boolean, optional | Portamento or not. |
| `releaseVelocity` | number, optional | A new release velocity from 0 to 1. |
| `slide` | boolean, optional | Slide to the next note or not. |
| `start` | Position, optional | A new start for every note. |
| `velocity` | number, optional | A new velocity from 0 to 1. |

Example:

```json
{
  "modX": 0.25,
  "notes": [
    0
  ],
  "velocity": 0.5
}
```

### `note.slice`

Cuts notes in two where they cross a position.

- Effects: undoable
- Calls in one gesture fold into one undo step.

| Param | Type | Description |
|---|---|---|
| `at` | Position | Where to cut. Notes that don't cross it stay whole. |
| `notes` | array of integer, optional | The note ids; the selected notes by default. |
| `pattern` | PatternRef, optional | The pattern; the current pattern by default. |

Example:

```json
{
  "at": 60,
  "notes": [
    0
  ]
}
```

### `note.stamp`

Adds a chord from its symbol, such as Am7 or C/E, and selects it.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `channel` | ChannelRef, optional | The channel that plays it; the piano roll's channel by default. |
| `chord` | string | The chord symbol, such as "Am7", "Fmaj7", "G7sus4" or "C/E". |
| `length` | Length, optional | How long it lasts; one beat by default. |
| `octave` | integer, optional | The octave of the root, where C4 is middle C; 4 by default. |
| `pattern` | PatternRef, optional | The pattern; the current pattern by default. |
| `start` | Position | Where the chord starts. |
| `velocity` | number, optional | Velocity from 0 to 1; 0.8 by default. |

Example:

```json
{
  "channel": "808",
  "chord": "Cmaj7",
  "start": "1:3:0"
}
```

### `note.strum`

Rolls chords so each voice starts a little after the one before.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `direction` | StrumDirection, optional | Up from the lowest key, or down from the highest; up by default. |
| `notes` | array of integer, optional | The note ids; the selected notes by default. |
| `pattern` | PatternRef, optional | The pattern; the current pattern by default. |
| `time` | Length, optional | How much later each voice of a chord starts than the one before; 30 ticks by default. |

Example:

```json
{
  "direction": "down",
  "notes": [
    0,
    1
  ],
  "time": 30
}
```

### `note.transpose`

Moves notes up or down by semitones.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `notes` | array of integer, optional | The note ids; the selected notes by default. |
| `pattern` | PatternRef, optional | The pattern; the current pattern by default. |
| `semitones` | integer | Semitones to move by, up when positive. |

Example:

```json
{
  "notes": [
    0,
    1
  ],
  "semitones": 2
}
```

### `note.unhumanize`

Puts notes back where quantize or humanize found them.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `notes` | array of integer, optional | The note ids; the selected notes by default. |
| `pattern` | PatternRef, optional | The pattern; the current pattern by default. |

Example:

```json
{
  "notes": [
    0,
    1
  ]
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

### `pattern.export_midi`

Writes a pattern as a MIDI file, one track per channel.

- Effects: disk
- Menu: File > Export pattern as MIDI

| Param | Type | Description |
|---|---|---|
| `path` | string, optional | Where to write the `.mid` file: an absolute path, or one relative to the project's folder (the library's Rendered folder for a project with no file yet). `.mid` is added when the path has no extension. By default a new file in the Rendered folder, named after the pattern. |
| `pattern` | PatternRef, optional | The pattern; the current pattern by default. |

Example:

```json
{
  "path": "Verse.mid",
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

### `pattern.split_by_channel`

Makes one pattern per channel and stacks their clips where the pattern's clips were.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `pattern` | PatternRef, optional | The pattern; the current pattern by default. |

Example:

```json
{
  "pattern": "pattern:2"
}
```

### `pattern.toggle_step`

Turns one step of a channel on or off.

- Effects: undoable
- Calls in one gesture fold into one undo step.

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

### `playlist.delete_time`

Removes a span of the song and closes the gap, cutting clips at its edges.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `at` | Position | Where the time is inserted or removed. |
| `length` | Length | How much. |

Example:

```json
{
  "at": "2:1:0",
  "length": "1 bar"
}
```

### `playlist.group_tracks`

Puts tracks in a named group that folds under one header; they move next to each other.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `name` | string | The group's name. Tracks join an existing group of that name. |
| `tracks` | array of TrackRef | The tracks to put in the group. |

Example:

```json
{
  "name": "Drums",
  "tracks": [
    1,
    3
  ]
}
```

### `playlist.insert_time`

Pushes everything from a position later by a length, cutting clips that cross it.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `at` | Position | Where the time is inserted or removed. |
| `length` | Length | How much. |

Example:

```json
{
  "at": "3:1:0",
  "length": "1 bar"
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

### `playlist.rename_track_group`

Renames a track group.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `group` | string | The group's current name. |
| `name` | string | Its new name. |

Example:

```json
{
  "group": "Drums",
  "name": "Beat"
}
```

### `playlist.set_group_props`

Mutes, unmutes, solos or unsolos every track of a group.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `group` | string | The group, by name. |
| `muted` | boolean, optional | Mutes or unmutes every track of the group. |
| `solo` | boolean, optional | Solos or unsolos every track of the group. |

Example:

```json
{
  "group": "Drums",
  "solo": true
}
```

### `playlist.set_track_props`

Sets a track's name, color, mute, solo, lock or height.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `color` | Color, optional | A new color. |
| `height` | number, optional | Its height in the playlist, from 16 to 400 points. |
| `locked` | boolean, optional | Locked against edits or not. |
| `muted` | boolean, optional | Muted or not. |
| `name` | string, optional | A new name. |
| `solo` | boolean, optional | Soloed or not: while any track is soloed, only soloed tracks play. |
| `track` | TrackRef, optional | The track; the selected track by default. |

Example:

```json
{
  "muted": false,
  "name": "Drums",
  "solo": true,
  "track": 1
}
```

### `playlist.ungroup_tracks`

Dissolves a track group, or takes tracks out of theirs.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `group` | string, optional | The group to dissolve, by name. |
| `tracks` | array of TrackRef, optional | Or: tracks to take out of their groups. |

Example:

```json
{
  "group": "Drums"
}
```

## plugin

### `plugin.describe`

Describes an effect and its parameters: ids, units, ranges and defaults.

- Effects: none

| Param | Type | Description |
|---|---|---|
| `effect` | string | The effect by uid or name, such as `bodo.delay` or `Delay`. |

Example:

```json
{
  "effect": "bodo.ducker"
}
```

### `plugin.get_params`

Lists the parameters of the effect in an FX slot with their values.

- Effects: none

| Param | Type | Description |
|---|---|---|
| `insert` | InsertRef, optional | The insert; the selected insert by default. |
| `slot` | integer | The slot, 1 to 10. |

Example:

```json
{
  "insert": 2,
  "slot": 1
}
```

### `plugin.list`

Lists the effects FX slots can load, with their parameters.

- Effects: none

No params.

Example:

```json
{}
```

### `plugin.set_param`

Sets a parameter of the effect in an FX slot, in its own unit.

- Effects: undoable
- Calls in one gesture fold into one undo step.

| Param | Type | Description |
|---|---|---|
| `insert` | InsertRef, optional | The insert; the selected insert by default. |
| `param` | value | The parameter, by id or name. |
| `slot` | integer | The slot, 1 to 10. |
| `value` | number | Its value, in its own unit; clamped to its range. |

Example:

```json
{
  "insert": 2,
  "param": "Depth",
  "slot": 1,
  "value": 24
}
```

## preset

### `preset.load_channel`

Loads a .bodochannel file as a new channel, or into a channel to replace its sound.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `channel` | ChannelRef, optional | A channel to turn into the preset, keeping its place in the rack, its mixer insert and its MIDI input. By default the preset becomes a new channel. |
| `path` | string | The `.bodochannel` file: an absolute path, a `library:` key, or a name in the library's Channel presets folder. |
| `target` | InsertRef, optional | For a new channel, the mixer insert it plays into; the first insert no channel uses by default. |

Example:

```json
{
  "path": "Kick"
}
```

### `preset.load_mixer`

Gives a mixer insert the settings and FX slots of a .bodomixer file.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `insert` | InsertRef, optional | The insert that takes the preset's settings and FX slots; the selected insert by default. It keeps its name, routing, hardware ports, recording settings, mute and solo. |
| `path` | string | The `.bodomixer` file: an absolute path, a `library:` key, or a name in the library's Mixer presets folder. |

Example:

```json
{
  "insert": 2,
  "path": "Insert 1"
}
```

### `preset.load_score`

Adds the notes of a .bodoscore file to a channel in a pattern.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `at` | Position, optional | Where in the pattern the score starts; its start by default. |
| `channel` | ChannelRef, optional | The channel that plays the notes; the piano roll's channel by default. |
| `path` | string | The `.bodoscore` file: an absolute path, a `library:` key, or a name in the library's Scores folder. |
| `pattern` | PatternRef, optional | The pattern; the current pattern by default. |
| `replace` | boolean, optional | Remove the channel's notes in the pattern first; false by default, which adds the score to them. |

Example:

```json
{
  "at": "1:3:0",
  "channel": "Clap",
  "path": "Pattern 1 Kick"
}
```

### `preset.save_channel`

Saves a channel, with its samples and plugin states, as a .bodochannel file in the library.

- Effects: disk

| Param | Type | Description |
|---|---|---|
| `channel` | ChannelRef, optional | The channel; the piano roll's channel, or the one selected channel, by default. |
| `name` | string, optional | The preset's name; the channel's name by default. |
| `path` | string, optional | Where to write it: an absolute path or a name in the library's Channel presets folder. By default a new file there named after the preset. |

Example:

```json
{
  "channel": "Kick"
}
```

### `preset.save_mixer`

Saves a mixer insert's settings and FX slots as a .bodomixer file in the library.

- Effects: disk

| Param | Type | Description |
|---|---|---|
| `insert` | InsertRef, optional | The insert; the selected insert by default. |
| `name` | string, optional | The preset's name; the insert's name by default. |
| `path` | string, optional | Where to write it: an absolute path or a name in the library's Mixer presets folder. By default a new file there named after the preset. |

Example:

```json
{
  "insert": 1
}
```

### `preset.save_score`

Saves one channel's notes in a pattern as a .bodoscore file in the library.

- Effects: disk

| Param | Type | Description |
|---|---|---|
| `channel` | ChannelRef, optional | The channel whose notes to save; the piano roll's channel by default. |
| `name` | string, optional | The score's name; the pattern's and the channel's names by default. |
| `path` | string, optional | Where to write it: an absolute path or a name in the library's Scores folder. By default a new file there named after the score. |
| `pattern` | PatternRef, optional | The pattern; the current pattern by default. |

Example:

```json
{
  "channel": "Kick"
}
```

## project

### `project.add_marker`

Puts a named marker on the timeline.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `at` | Position | Where it goes. |
| `color` | Color, optional | Its color. |
| `name` | string, optional | Its name; "Marker N" by default. |

Example:

```json
{
  "at": "3:1:0",
  "name": "Drop"
}
```

### `project.export_midi`

Writes the whole playlist as a MIDI file, every clip as it plays, one track per channel.

- Effects: disk
- Menu: File > Export song as MIDI

| Param | Type | Description |
|---|---|---|
| `path` | string, optional | Where to write the `.mid` file: an absolute path, or one relative to the project's folder (the library's Rendered folder for a project with no file yet). `.mid` is added when the path has no extension. By default a new file in the Rendered folder, named after the song. |

Example:

```json
{
  "path": "Song.mid"
}
```

### `project.import_midi`

Imports a MIDI file as a new pattern, with an empty sampler channel per part, and places it on the playlist.

- Effects: undoable
- Menu: File > Import MIDI file

| Param | Type | Description |
|---|---|---|
| `at` | Position, optional | Where the clip starts; bar 1 by default. |
| `path` | string | The `.mid` file: an absolute path, or one relative to the project's folder. |
| `place` | boolean, optional | Place a clip of the new pattern on the playlist; true by default. |
| `tempo` | boolean, optional | Make the file's first tempo and time signature the project's; true by default. |
| `track` | TrackRef, optional | The track for the clip; the first track with no clips by default (a new track when none is empty). |

Example:

```json
{
  "path": "Groove.mid"
}
```

### `project.new`

Starts a new project from a template, replacing the current one.

- Effects: destructive
- Acts on the session, so it can't run inside a batch.
- Menu: File > New
- Shortcut: Mod+N

| Param | Type | Description |
|---|---|---|
| `discardChanges` | boolean, optional | Drop unsaved changes to the current project; without it, new refuses when there are any. |
| `from` | string, optional | A project to start from instead, such as a template or a demo: an absolute path, a `library:` key, or a name in the library's Templates folder. The new project copies it and has no file yet, so the original stays as it was. |
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

### `project.remove_marker`

Removes a marker by name or number.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `marker` | MarkerRef | The marker. |
| `name` | string, optional | A new name, for renaming. |

Example:

```json
{
  "marker": 1
}
```

### `project.rename_marker`

Renames a marker.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `marker` | MarkerRef | The marker. |
| `name` | string, optional | A new name, for renaming. |

Example:

```json
{
  "marker": 1,
  "name": "Chorus"
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

### `project.save_template`

Saves a copy of the project in the library's Templates folder, for project.new to start from.

- Effects: disk
- Acts on the session, so it can't run inside a batch.
- Menu: File > Save as template

| Param | Type | Description |
|---|---|---|
| `name` | string, optional | The template's name; the project's title by default. |

Example:

```json
{
  "name": "My Starter"
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

### `project.set_pan_law`

Sets how mono sources spread as they pan, for the whole project.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `law` | Law | How mono sources spread as they pan. |

Example:

```json
{
  "law": "linear6dB"
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

## recording

### `recording.add_takes`

Places recorded takes on the playlist: a sample, an audio clip channel into the recording insert, and a clip for each.

- Effects: undoable

| Param | Type | Description |
|---|---|---|
| `takes` | array of TakeSpec | The takes of one recording. |

Example:

```json
{
  "takes": [
    {
      "frames": 96000,
      "insert": 1,
      "path": "Recorded/Insert 1_1.wav",
      "start": "2:1:0"
    }
  ]
}
```

## render

### `render.export_mix`

Renders the song, or one pattern, through the master to an audio file.

- Effects: disk
- Menu: File > Export mix

| Param | Type | Description |
|---|---|---|
| `end` | Position, optional | Where to stop; the end of the song or of one pass of the pattern by default. One second of tail follows. |
| `format` | Format, optional | The file format; WAV by default. |
| `path` | string | The file to write: an absolute path, or one relative to the project's folder. The format's extension is added when it has none. |
| `pattern` | PatternRef, optional | Loop this pattern instead of playing the playlist. |

Example:

```json
{
  "path": "Song.wav"
}
```

### `render.export_stems`

Renders the song, or one pattern, to one audio file per insert in a folder, delay compensated so they line up.

- Effects: disk
- Menu: File > Export stems

| Param | Type | Description |
|---|---|---|
| `end` | Position, optional | Where to stop; the end of the song or of one pass of the pattern by default. One second of tail follows. |
| `folder` | string | The folder the stems go in: an absolute path, or one relative to the project's folder. It is made when needed. |
| `format` | Format, optional | The file format; WAV by default. |
| `mix` | boolean, optional | Also write the whole mix, through the master, as `Mix`. |
| `pattern` | PatternRef, optional | Loop this pattern instead of playing the playlist. |

Example:

```json
{
  "folder": "Stems",
  "format": "flac"
}
```

## sampler

### `sampler.set`

Sets any of a sampler channel's settings: the played part and loop, content options, declicking, envelopes, filter, stretch, precomputed effects and arpeggiator.

- Effects: undoable
- Calls in one gesture fold into one undo step.

| Param | Type | Description |
|---|---|---|
| `arp` | Arp, optional | The arpeggiator. |
| `channel` | ChannelRef, optional | The sampler channel; the selected channel by default. |
| `declick` | DeclickMode, optional | Fades against clicks. |
| `effects` | SampleFx, optional | Effects rendered into a copy of the sample. |
| `end` | number, optional | Where the played part ends, 0 to 1 of the sample. |
| `envelope` | Envelope, optional | The volume envelope. |
| `filter` | FilterSettings, optional | The filter. |
| `filterEnvelope` | Envelope, optional | The filter envelope. |
| `highKey` | Key, optional | The highest key it plays. |
| `keepOnDisk` | boolean, optional | Streams the sample from disk where the playback allows it. |
| `loadRegions` | boolean, optional | Plays the file's regions, one per key from the root key. |
| `loadSlices` | boolean, optional | Plays the slices between the file's markers, one per key from the root key. |
| `loopEnd` | number, optional | The loop end, 0 to 1 of the sample. |
| `loopMode` | Loop, optional | How the sample loops. |
| `loopStart` | number, optional | The loop start, 0 to 1 of the sample. |
| `lowKey` | Key, optional | The lowest key it plays. |
| `oneShot` | boolean, optional | Plays the whole sample on every note however short the note. |
| `pitchEnvelope` | Envelope, optional | The pitch envelope. |
| `pitchRange` | number, optional | How far the pitch knob reaches, 1 to 48 semitones. |
| `polyphony` | integer, optional | The most voices at once, 1 to 256. |
| `resample` | boolean, optional | Converts the sample to the project's rate when it loads. |
| `reverse` | boolean, optional | Plays the sample backwards. |
| `rootKey` | Key, optional | The key that plays the sample at its own pitch, such as "C4" or 60. |
| `start` | number, optional | Where the played part starts, 0 to 1 of the sample. |
| `startOffset` | number, optional | Where notes start inside the played part, 0 to 1 of it. |
| `stretch` | Stretch, optional | Time stretching. |

Example:

```json
{
  "arp": {
    "direction": "upDown",
    "enabled": true,
    "rate": "1 step"
  },
  "channel": "Kick",
  "declick": "smooth",
  "effects": {
    "fadeOut": 0.05,
    "normalize": true
  },
  "end": 0.75,
  "filter": {
    "cutoff": 800,
    "kind": "lowPass",
    "resonance": 0.3
  },
  "filterEnvelope": {
    "amount": 3,
    "decay": 0.4,
    "enabled": true
  },
  "loopEnd": 0.5,
  "loopMode": "pingPong",
  "loopStart": 0.25,
  "oneShot": false
}
```

## transport

### `transport.count_in`

Turns count-in on or off: recording starts after a bar of clicks.

- Effects: none

| Param | Type | Description |
|---|---|---|
| `on` | boolean, optional | On or off; toggles when left out. |

Example:

```json
{}
```

### `transport.loop_record`

Turns loop recording on or off: each pass of a loop becomes its own take, and MIDI passes add up.

- Effects: none

| Param | Type | Description |
|---|---|---|
| `on` | boolean, optional | On or off; toggles when left out. |

Example:

```json
{
  "on": true
}
```

### `transport.metronome`

Turns the metronome on or off; it clicks every beat while playing.

- Effects: none

| Param | Type | Description |
|---|---|---|
| `on` | boolean, optional | On or off; toggles when left out. |

Example:

```json
{
  "on": true
}
```

### `transport.play`

Starts playing from the song position.

- Effects: none

No params.

Example:

```json
{}
```

### `transport.record`

Starts playing and recording the armed inserts, or stops a recording; the takes land on the playlist.

- Effects: none

No params.

Example:

```json
{}
```

### `transport.set_loop`

Loops Song mode over a region of the playlist, or stops looping.

- Effects: none

| Param | Type | Description |
|---|---|---|
| `end` | Position, optional | Where it ends; playing wraps back to the start exactly here. |
| `off` | boolean, optional | True to stop looping. |
| `start` | Position, optional | Where the loop starts. |

Example:

```json
{
  "end": "5:1:0",
  "start": "1:1:0"
}
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
