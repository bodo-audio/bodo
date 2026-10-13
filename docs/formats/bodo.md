# The `.bodo` project format

- **Format version:** 1
- **Decided in:** [ADR 0007](../adr/0007-project-file-format.md), [project.md section 18.1](../../project.md#181-bodo-container)
- **Implemented in:** `crates/bodo-io` (reading, writing, migrations) and `crates/bodo-model` (the data model)

A Bodo project is plain JSON in a container, so people, agents and other tools can read it, and so it diffs well in git. This page specifies the container, every field of the JSON, and the rules for versions, saving and backups.

## 1. Containers

A project is stored in one of two containers with the same layout.

| Container | Name | What it is |
|---|---|---|
| Project file | `MySong.bodo` | A zip archive |
| Project folder | `MySong.bodoproj/` | The same entries as plain files in a folder, for keeping a project in git |

`bodo-cli pack` turns a folder into a file and `bodo-cli unpack` does the opposite. Both keep every entry. Readers decide by what is on disk: a folder is read as a project folder, anything else as a zip file. Writers decide by the name: a path ending in `.bodoproj` is written as a folder, any other path as a zip file.

### 1.1 Entries

| Entry | Required | Contents |
|---|---|---|
| `project.json` | yes | The project ([section 2](#2-projectjson)) |
| `plugins/<instance>.state` | no | The saved state of one plugin instance, as the plugin's own binary blob ([section 4](#4-plugin-states)) |
| `samples/` | no | Embedded or consolidated audio |
| `recorded/` | no | Recordings, filled by "collect all" |
| `thumbnail.png` | no | A picture of the project for the Hub |

Entry names use forward slashes. A reader ignores entries it doesn't know, and a writer keeps them: saving a project over its old file, or converting it to the other container, carries every entry that isn't `project.json` or a plugin state. Entry names that are absolute or contain `..` are never written to disk.

### 1.2 Deterministic bytes

The same project always gives the same bytes, wherever and whenever it is saved:

- `project.json` comes first, then the plugin states sorted by name, then the other entries sorted by name;
- every zip entry is compressed with Deflate and has a fixed time (1980-01-01 00:00:00), fixed Unix permissions (`0644`) and the Unix "made by" system;
- `project.json` itself is deterministic ([section 2.1](#21-layout)).

## 2. `project.json`

### 2.1 Layout

`project.json` is one JSON object, pretty-printed with two-space indentation and ending in a newline. Its first two keys are the format header:

```json
{
  "format": "bodo",
  "format_version": 1,
  "id": "proj_01JABCDEF0123456789ABCDEFG",
  ...
}
```

The project's own fields follow, in this order: `id`, `meta`, `settings`, `tempo`, `meter`, `channels`, `rackOrder`, `patterns`, `tracks`, `trackOrder`, `mixer`, `samples`, `markers`. Keys inside objects keyed by id are sorted, so they come in creation order. Field names are camelCase, except the two header keys.

Numbers are exact: floats are written in their shortest form that reads back to the same value, so a project survives any number of save and load cycles unchanged.

Readers accept any key order, and missing fields that have a default (below) take it. Unknown keys are ignored. At the top level, `id`, `meta`, `settings`, `tempo` and `mixer` are required; the other fields may be left out and default to empty (4/4 for `meter`).

### 2.2 Value conventions

| Kind | JSON form | Notes |
|---|---|---|
| Id | string `<prefix>_<ULID>` | See [section 2.3](#23-ids) |
| Tick | integer | 960 ticks per quarter note (`settings.ppq`) |
| Frames | integer | Sample frames at the project's sample rate |
| Seconds | number | |
| Color | string `#rrggbb` | Lowercase when written, either case when read |
| Gain (linear) | number | 1.0 is unity |
| Pan | number | -1 (left) to 1 (right) |
| Velocity | integer 0 to 65535 | 16-bit, as in MIDI 2.0. The default is 32768 |
| MIDI key | integer 0 to 127 | 60 is middle C |
| Enum without data | camelCase string | Such as `"postFader"` |
| Enum with data | object with `"type"` | Such as `{"type": "fixed", "ticks": 3840}` |

### 2.3 Ids

Every entity has a ULID wrapped in a typed id. In text, an id is its type's prefix, an underscore and the 26-character ULID. A reader refuses an id whose prefix doesn't match the type expected in that place.

| Prefix | Identifies |
|---|---|
| `proj` | The project |
| `ch` | A channel of the channel rack |
| `pat` | A pattern |
| `trk` | A playlist track |
| `clip` | A clip on a playlist track |
| `ins` | A mixer insert, including the master |
| `smp` | A sample file |
| `plug` | A plugin instance |

Notes are the exception: a note id is an integer unique within its pattern, below the pattern's `nextNoteId`.

Ids never change: not when saving and loading, not on undo, not when merging an agent's changes. The numbers people see (insert 3, channel 5) come from the order lists (`rackOrder`, `trackOrder`, `mixer.order`).

### 2.4 `meta`

| Field | Type | Default | Meaning |
|---|---|---|---|
| `title` | string | `""` | The title |
| `author` | string | `""` | The author |
| `credits` | array of strings | `[]` | Credits for licensed content the project uses, such as CC-BY samples |
| `created` | string or null | null | When the project was created, RFC 3339 |

### 2.5 `settings`

| Field | Type | Default | Meaning |
|---|---|---|---|
| `sampleRate` | integer above 0 | 48000 | The rate the project renders at, in Hz |
| `ppq` | integer | 960 | Ticks per quarter note. Stored so a later resolution can be reached by migration |
| `panLaw` | `"constantPower3dB"`, `"compromise4_5dB"`, `"linear6dB"` | `"constantPower3dB"` | How mono sources pan (project.md section 9.5) |

### 2.6 `tempo`

A non-empty array of tempo points, sorted by `at`, the first at tick 0.

| Field | Type | Meaning |
|---|---|---|
| `at` | tick | Where the point is; strictly after the previous point |
| `bpm` | number, 1 to 1000 | Quarter notes per minute at the point |
| `curve` | `"jump"` or `"linear"` | `jump` holds the tempo until the next point; `linear` ramps to the next point's tempo |

### 2.7 `meter`

A non-empty array of time signature changes, sorted by bar, the first at bar 0. The default is 4/4 from the start.

| Field | Type | Meaning |
|---|---|---|
| `bar` | integer, from 0 | The bar the change starts at; strictly after the previous change |
| `numerator` | integer 1 to 64 | Beats per bar |
| `denominator` | 1, 2, 4, 8, 16 or 32 | The beat unit |

### 2.8 `channels` and `rackOrder`

`channels` is an object from channel id to channel. `rackOrder` lists every channel id exactly once, in channel rack order.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `id` | channel id | | Its own id |
| `name` | string | | Its name |
| `color` | color | | Its color |
| `kind` | object | | What it plays, below |
| `target` | insert id or null | null | The mixer insert it plays into; null is the master |
| `volume` | gain | 1.0 | Volume |
| `pan` | pan | 0.0 | Pan |
| `pitchCents` | number | 0.0 | Pitch shift in cents |
| `swing` | number 0 to 1 | 0.0 | How late every second step plays |
| `muted` | boolean | false | Muted |
| `midiIn` | object or null | null | A MIDI input that plays the channel: `port` (string) and `channel` (0 to 15, or null for all) |

`kind` has a `type` and the fields of that type:

| `type` | Fields | Meaning |
|---|---|---|
| `sampler` | the sampler settings, below | The built-in sampler |
| `instrument` | `plugin`: a plugin reference ([section 2.13](#213-plugin-references)) | An instrument plugin |
| `audioClip` | `sample` (sample id), `stretch` (`"none"`, `"resample"` or `"stretch"`, default `"none"`) | Plays a sample from audio clips on the playlist |
| `automation` | the fields of an automation lane ([section 2.12](#212-automation)) | Plays automation from automation clips on the playlist |
| `layer` | `children`: array of channel ids | One trigger plays every child |
| `midiOut` | `port` (string), `channel` (0 to 15) | Sends MIDI to an external port |

Sampler settings, all optional:

| Field | Type | Default | Meaning |
|---|---|---|---|
| `sample` | sample id or null | null | The sample it plays |
| `rootKey` | MIDI key | 60 | The key that plays the sample at its own pitch |
| `keyRange` | `[low, high]` MIDI keys | `[0, 127]` | The keys it responds to |
| `envelope` | object | | The amplitude envelope: `attack`, `decay`, `release` in seconds, `sustain` 0 to 1 |
| `start` | number 0 to 1 | 0.0 | Where playback starts, as a fraction of the sample |
| `loopMode` | `"off"`, `"forward"`, `"pingPong"` | `"off"` | How it loops |
| `loopStart`, `loopEnd` | numbers 0 to 1 | 0.0, 1.0 | The loop points, as fractions of the sample |
| `reverse` | boolean | false | Plays the sample backwards |
| `keepOnDisk` | boolean | false | Streams the sample from disk instead of loading it into memory |
| `polyphony` | integer | 32 | The most voices it plays at once |

### 2.9 `patterns`

An object from pattern id to pattern.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `id` | pattern id | | Its own id |
| `name` | string | | Its name |
| `color` | color | | Its color |
| `length` | `{"type": "auto"}` or `{"type": "fixed", "ticks": n}` | auto | Auto rounds up to whole bars around the last note |
| `binding` | channel id or null | null | Set: a clip-mode pattern, where only this channel may have notes |
| `notes` | object from channel id to an array of notes | `{}` | Each channel's notes, sorted by start, then key, then id |
| `automation` | array of automation lanes | `[]` | Automation that travels with the pattern; at most one lane per target |
| `nextNoteId` | integer | 0 | The id the next new note gets; above every note id in the pattern |

A note:

| Field | Type | Default | Meaning |
|---|---|---|---|
| `id` | integer | | Unique within the pattern; maps to a CLAP note id |
| `key` | MIDI key | | The key |
| `start` | tick | | Where it starts, from the pattern's start |
| `len` | tick, above 0 | | How long it lasts |
| `velocity` | velocity | 32768 | Velocity |
| `releaseVelocity` | velocity | 32768 | Release velocity |
| `finePitchCents` | number | 0.0 | Fine pitch in cents |
| `pan` | pan | 0.0 | Pan |
| `modX`, `modY` | numbers 0 to 1 | 0.0 | Modulation X and Y |
| `color` | integer 0 to 15 | 0 | The note group, which doubles as the MIDI channel |
| `muted` | boolean | false | Muted |
| `slide`, `porta` | booleans | false | Slide and portamento notes |
| `quantizedStart` | tick or null | null | The grid position before humanization, so it can be undone |
| `expr` | object or null | null | Expression curves: `pitch` (semitones), `pressure` and `timbre` (0 to 1), each an array of `{"at": tick from the note's start, "value": number}` |

### 2.10 `tracks` and `trackOrder`

`tracks` is an object from track id to playlist track. `trackOrder` lists every track id exactly once, in playlist order.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `id` | track id | | Its own id |
| `name` | string | | Its name |
| `color` | color | | Its color |
| `height` | number | 48.0 | Its height in the playlist, in points |
| `muted` | boolean | false | Muted |
| `locked` | boolean | false | Locked against edits |
| `clips` | array of clips | `[]` | Sorted by start |

A clip:

| Field | Type | Default | Meaning |
|---|---|---|---|
| `id` | clip id | | Unique in the project |
| `start` | tick | | Where it starts |
| `len` | tick, above 0 | | How long it lasts |
| `offset` | `{"type": "ticks", "value": tick}` or `{"type": "frames", "value": frames}` | 0 ticks | How far into its source the clip starts; frames for audio locked to time |
| `muted` | boolean | false | Muted, separately from the track |
| `color` | color or null | null | Overrides the source's color |
| `source` | `{"type": "pattern" \| "audio" \| "automation", "id": ...}` | | A pattern id, or the id of an `audioClip` or `automation` channel |
| `fades` | `{"fadeIn": tick, "fadeOut": tick}` or null | null | Fades of an audio clip |

### 2.11 `mixer`

| Field | Type | Meaning |
|---|---|---|
| `master` | insert | The master, always the last node of the graph |
| `inserts` | object from insert id to insert | Every other insert |
| `order` | array of insert ids | Display order, insert 1 first; lists every insert except the master exactly once |
| `routes` | array of routes | Routes between channels and inserts; at most one per pair of ends |

An insert:

| Field | Type | Default | Meaning |
|---|---|---|---|
| `id` | insert id | | Its own id |
| `name` | string | | Its name |
| `color` | color | `#737373` | Its color |
| `slots` | array of exactly 10 slots | 10 empty slots | The FX slots, in processing order |
| `eq` | object | flat | The 3-band strip EQ: `low`, `mid`, `high`, each `{"frequency": Hz, "gainDb": dB, "q": number}` |
| `volumeDb` | number, up to +6 | 0.0 | The fader in dB |
| `pan` | pan | 0.0 | Pan |
| `stereoSep` | number -1 to 1 | 0.0 | Stereo separation: -1 wider, 0 unchanged, 1 mono |
| `polarityInvert` | boolean | false | Inverts polarity before the FX |
| `swapLr` | boolean | false | Swaps left and right before the FX |
| `latencyOffset` | frames | 0 | Manual track latency: positive declares latency, negative delays the strip |
| `pdcBypass` | `"never"`, `"always"`, `"whileRecording"` | `"never"` | When delay compensation is bypassed |
| `muted`, `solo` | booleans | false | Mute and solo |
| `toMaster` | boolean | true | Whether it feeds the master; ignored on the master |
| `input`, `output` | `{"channels": [device channel, ...]}` or null | null | Hardware input for recording and monitoring; direct hardware output |
| `armed` | boolean | false | Disk recording armed |
| `recordTap` | `"input"`, `"preFx"`, `"postFx"`, `"postFader"` | `"postFader"` | What disk recording captures |
| `recordFormat` | `"float32"`, `"int24"`, `"flac"` | `"float32"` | The format of recordings |

A slot, all fields optional:

| Field | Type | Default | Meaning |
|---|---|---|---|
| `plugin` | plugin reference or null | null | The plugin in the slot |
| `enabled` | boolean | true | A disabled slot passes audio through and keeps reporting its latency |
| `mix` | number 0 to 1 | 1.0 | Dry/wet mix |
| `sidechain` | route source or null | null | Where the slot's sidechain input comes from; needs a sidechain route from that source into this insert |

A route:

| Field | Type | Default | Meaning |
|---|---|---|---|
| `from` | `{"type": "channel", "id": channel id}` or `{"type": "insert", "id": insert id}` | | The source |
| `to` | insert id | | The target; never the master (inserts reach it through `toMaster`) and never the source itself |
| `sendLevel` | gain | | The send level |
| `tap` | `"preFx"`, `"preFader"`, `"postFader"` | `"postFader"` | Where the signal is taken on the source strip |
| `kind` | `"audio"` or `"sidechain"` | `"audio"` | Sidechain routes feed plugin sidechain inputs only |

### 2.12 Automation

An automation lane, used by `automation` channels and inside patterns:

| Field | Type | Meaning |
|---|---|---|
| `target` | target, below | What the lane controls |
| `points` | array of points | Sorted by `at` |
| `default` | number 0 to 1 | The value before the first point |

A point is `{"at": tick, "value": number 0 to 1, "curve": curve}`. The curve shapes the way to the next point: `{"type": "hold"}`, `{"type": "linear"}` (the default), `{"type": "exp", "tension": -1 to 1}`, `{"type": "sCurve", "tension": -1 to 1}`, `{"type": "stairs", "tension": steps}`, `{"type": "sine"}` or `{"type": "pulse"}`.

Targets:

| Target | JSON |
|---|---|
| An insert parameter | `{"insert": [insert id, "volume" \| "pan" \| "stereoSep"]}` |
| An FX slot parameter | `{"slot": [insert id, slot index 0 to 9, "mix" \| "enabled"]}` |
| A plugin parameter | `{"plugin": [plugin instance id, CLAP or VST3 parameter id]}` |
| A channel parameter | `{"channel": [channel id, "volume" \| "pan" \| "pitch"]}` |
| A route's send level | `{"route": {"from": route source, "to": insert id}}` |
| The tempo | `"tempo"` |

### 2.13 Plugin references

| Field | Type | Default | Meaning |
|---|---|---|---|
| `instance` | plugin instance id | | Names the instance, and its state entry |
| `format` | `"native"`, `"clap"`, `"vst3"`, `"au"`, `"lv2"` | | The plugin format; `native` is a Bodo Kit plugin linked into Bodo |
| `uid` | string | | The format's unique id for the plugin, such as a CLAP id |
| `sandbox` | `"inProcess"`, `"sharedBridge"`, `"perVendor"`, `"perPlugin"` | `"sharedBridge"` | Where the plugin runs (project.md section 7.3) |

The plugin's state is not in the JSON ([section 4](#4-plugin-states)).

### 2.14 `samples`

An object from sample id to sample reference.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `id` | sample id | | Its own id |
| `path` | string | | Where the file is ([section 3](#3-sample-paths)) |
| `hash` | string or null | null | The BLAKE3 hash of the file, 64 lowercase hex digits, used to find a moved file |
| `market` | object or null | null | The marketplace item it came from: `package`, `version` and `item` (its path inside the package) |

### 2.15 `markers`

An array of markers sorted by `at`: `{"at": tick, "name": string, "color": color or null}`.

### 2.16 Invariants

A reader refuses a project that breaks any of these, and lists every one it breaks. Writers never write such a project.

- Every channel, pattern, insert and sample that a channel, pattern, clip or route refers to exists, and is of the right kind: an `audio` clip plays an `audioClip` channel, an `automation` clip plays an `automation` channel.
- `rackOrder`, `trackOrder` and `mixer.order` list every entity of their map exactly once, and `mixer.order` doesn't list the master.
- Notes, clips, markers and the points of automation channels are sorted; note and clip lengths and fixed pattern lengths are above 0.
- Note ids are unique within their pattern and below its `nextNoteId`; clip ids are unique in the project; MIDI keys are at most 127.
- A pattern with a `binding` has notes only for that channel, and has at most one automation lane per target.
- A layer never contains itself, directly or through other layers.
- Routes never target the master or their own source, and the routes between inserts, sidechain routes included, have no cycle.
- A slot's `sidechain` names the source of a sidechain route into its insert.

## 3. Sample paths

A sample's `path` takes one of three forms:

| Form | Example | Meaning |
|---|---|---|
| Relative | `Samples/kick.wav` | Relative to the *project folder*: the folder that holds the `.bodo` file or the `.bodoproj` folder. Always forward slashes |
| Absolute | `/Users/me/Sounds/kick.wav`, `C:\Sounds\kick.wav` | A file outside the project folder |
| Factory | `factory:drums/kick.wav` | Bodo's own factory content, resolved by Bodo and never rebased |

The project folder is the folder *around* the project, so packing a folder into a file in the same place, or unpacking it, keeps every path valid.

A writer stores a file inside the project folder's tree as a relative path, and any other file as an absolute path. When a project is saved to another folder, its relative paths are resolved against the old folder and stored again by the same rule, so they keep naming the same files. Paths are compared as text after resolving `.` and `..`; symbolic links are not followed.

The `hash` is there so a file that moved can be found again by its contents (the missing-file resolver of project.md section 18.1).

## 4. Plugin states

Each plugin instance with a saved state has an entry `plugins/<instance>.state`, where `<instance>` is the instance id in text form, such as `plugins/plug_01JABCDEF0123456789ABCDEFG.state`. The entry holds the plugin's own state blob, byte for byte. Plugins in instrument channels and in the FX slots of every insert and the master can have one. A plugin without a state has no entry, and an entry that no plugin names is ignored.

Bodo collects states through each plugin's main-thread state API when it saves, never from the audio thread.

## 5. Versions and migrations

`format_version` is an integer that grows by one whenever the format changes in a way older readers can't follow.

- A reader refuses a file whose `format` isn't `"bodo"`, or whose `format_version` is missing or not a positive integer.
- A reader refuses a file whose `format_version` is newer than it knows, with a message naming both versions, so a project is never opened half understood.
- A file with an older `format_version` passes through a chain of migrations, one per version step (1 to 2, then 2 to 3, and so on), each working on the JSON before it is read. The next save writes the current version.
- New fields that older files don't have get a default and need no migration. A migration is needed only when old files would otherwise be read wrong.
- Every version has fixture projects in `crates/bodo-io/tests/corpus/`, and every fixture must keep loading.

## 6. Saving

### 6.1 Atomic saves

A save never leaves a damaged project behind:

- A project file is written to a hidden temporary file in the same folder (`.<name>.<process>.<n>.tmp`), flushed to disk, then renamed over the old file. On Unix the folder is flushed too, so the rename survives a crash.
- A new project folder is built under a temporary name and renamed into place. An existing project folder is updated one file at a time, each written the same atomic way, plugin states first and `project.json` last; plugin state files the project no longer has are deleted after that.

Readers skip hidden `.tmp` files.

### 6.2 Backups

Before a save replaces a project, the old version is copied to the `Backup` folder of the project folder, as `Backup/<name> <time>.<extension>`, for example `Backup/My Song 2026-10-13T14-05-09.120Z.bodo`. The time is UTC with millisecond precision, so the names sort in time order. Only the newest backups of each project are kept, 10 by default.

A backup keeps the project's relative sample paths unchanged, so they still start at the project folder, not at `Backup`.

### 6.3 Autosave and recovery

The app autosaves every 5 minutes by default, with the same atomic writes, and the next launch after a crash offers to recover the newest autosave (project.md section 18.1). Autosaves are ordinary project files.
