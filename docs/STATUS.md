# Status

Progress per milestone ([project.md section 20](../project.md#20-roadmap)). Update this file at every milestone.

## M0: Foundations (done, 2026-10-13)

| Item | State | Where |
|---|---|---|
| Workspace, toolchain, lints | Done | `Cargo.toml`, `rust-toolchain.toml` (Rust 1.98.1), `rustfmt.toml`, `clippy.toml` |
| `cargo xtask ci` | Done | fmt, clippy (default and all features), tests, docs, release build, then every check below |
| `cargo xtask layers` | Done | `xtask/src/layers.rs`; fixture `xtask/fixtures/layers` |
| `cargo xtask unsafe-audit` | Done | `xtask/src/unsafe_audit.rs` |
| `cargo xtask deny` | Done | `deny.toml`, `kit/deny.toml`; fixture `xtask/fixtures/licenses` |
| `cargo xtask text` | Done | `xtask/src/text.rs` |
| CI on Linux, macOS and Windows | Done | `.github/workflows/ci.yml` on [bodo-audio/bodo](https://github.com/bodo-audio/bodo) |
| Docs skeleton | Done | this file, [`architecture.md`](architecture.md), [`agents/`](agents/), [`formats/`](formats/) |
| `bodo-kit-core` | Done | time units, tempo map, bounded SPSC queues, RT worker pool |
| `bodo-kit-midi` | Done | MIDI 1.0 channel messages, Standard MIDI File read and write |

### Exit criteria

| Criterion | State |
|---|---|
| CI green on all 3 OSes | Met. Run 38367742899 passed on ubuntu-latest, macos-latest and windows-latest |
| Layer and license gates fail on deliberate violations | Met. The layers test expects every violation planted in its fixture, and `cargo xtask deny` proves on each run that the Kit policy rejects a planted GPL dependency |
| Tempo-map proptests pass | Met. `kit/bodo-kit-core/tests/tempo_map.rs` |

### Deferred

- **`#[rt_safe]` attribute** (project.md section 19.3): it needs a proc-macro crate, so it lands with the first audio-thread code in M1. Until then RT-safe functions carry the `RT-safe` doc line, and `kit/bodo-kit-core/tests/rt_safety.rs` runs them under `assert_no_alloc`.
- **OS audio workgroup and MMCSS for RT workers**: done in M1.
- **Denormal flushing on RT threads**: M1, with the first DSP.
- **rtsan job, fuzzing, benchmarks, agent evals**: nightly CI jobs, added as the code they cover lands.

## M1: Audio core (done, 2026-10-13)

Started and finished 2026-10-13. Open hardware checks: short exclusive-mode buffers on a real Windows sound card, and cpal on a real PulseAudio machine (see below).

| Item | State | Where |
|---|---|---|
| RT-safety tooling | Done | `#[rt_safe]` (`bodo-kit-macros`), RealtimeSanitizer CI job, denormal flushing ([ADR 0014](adr/0014-rt-safety-tooling-and-plan-handoff.md)) |
| Plan handoff | Done | `bodo_kit_core::handoff` |
| `bodo-kit-dsp` basics | Done | gain, pan laws, stereo, smoothing, delay lines, biquads and SVF, ADSR, interpolation, buffers |
| `bodo-kit-graph` | Done | incremental cycle checks, Kahn ordering, delay compensation, parallel executor ([ADR 0015](adr/0015-graph-executor.md)) |
| `bodo-kit-audio` | Done | decoding, WAV writing, resampling, peaks, disk streaming ([ADR 0016](adr/0016-disk-streaming-in-kit-audio.md)) |
| `bodo-kit-device` | Done | device lists, streams, latency, dropout counting, hot-plug detection, virtual device; ASIO builds in CI; real playback tested in CI on Windows (VB-CABLE) and Linux (PipeWire) |
| `bodo-model` v1 | Done | the project document and `Project::validate()` |
| `bodo-mix` | Done | realtime and offline engine, built-in sampler, sequencer, insert strips |
| WASAPI exclusive mode | Done | the "WASAPI Exclusive" host ([ADR 0017](adr/0017-wasapi-exclusive-mode.md)); see the Windows notes below |
| macOS audio workgroup | Done | RT workers join the device's IO `os_workgroup` through `Engine::set_thread_group`; checked on an Apple Silicon Mac |

### Exit criteria

| Criterion | State |
|---|---|
| 64 frames at 48 kHz without dropouts for 100 sampler voices | Met on an Apple Silicon Mac: 10 s, 7,503 callbacks, 0 dropouts, peak load 9% with 7 workers and 18% with none (`crates/bodo-mix/tests/stress.rs`, run by hand in release) |
| Zero allocations under rtsan | Met: every RT test suite runs under RealtimeSanitizer in CI, and the stress scenario ran clean under it locally |
| Offline renders bit-identical across runs | Met: `renders_are_bit_identical_for_any_workers_and_block_size` |

### Windows and Linux audio, as tested in CI

No Windows or Linux machine is available for testing yet, so CI installs a virtual sound card and plays through it:

- **Windows, shared mode:** real time with no dropouts, 43 ms output latency (2,062 frames at 48 kHz).
- **Windows, exclusive mode:** opens with 4 ms latency at the shortest period. VB-CABLE's own clock only ticks every 10 to 16 ms, so it cannot feed buffers shorter than that; at 1,024 frames (21 ms) exclusive mode runs in real time with no dropouts, and CI checks it there. Short buffers on a real sound card still need a check on a Windows machine.
- **Linux, PipeWire:** real time with no dropouts at every buffer size tried, down to 128 frames (5 ms).
- **Linux, PulseAudio:** cpal's ALSA backend stalls on PulseAudio's null sink at every buffer size, while `aplay` plays fine. Most current distributions run PipeWire; a check on a real PulseAudio machine is still open.

### Deferred to M4

- The 5 ms crossfade on graph changes, buffer reuse across nodes, and CPU time per node ([ADR 0015](adr/0015-graph-executor.md)).
- FX slots, strip EQ, send taps other than post-fader, sidechains, solo and negative track latency in the mixer strip.

## M2: Shell (done on macOS, 2026-10-13)

Started and finished 2026-10-13. Every check passes on macOS; CI on Linux and Windows runs once these commits are pushed.

| Item | State | Where |
|---|---|---|
| `bodo-engine` | Done | command registry, editing session, undo with gestures, templates, inspection ([ADR 0018](adr/0018-the-editing-session.md)); 67 commands in [`agents/commands.md`](agents/commands.md) |
| `.bodo` save and load | Done | `bodo-io`: zip and `.bodoproj` folders, versions and migrations, atomic saves, 10 rotating backups, sample path rebasing ([format](formats/bodo.md)) |
| Autosave and recovery | Done | every 5 minutes (Options > Autosave), offered back after a crash |
| `bodo-ui` | Done | the owner's layout: top bar, Browser, Channel Rack, Playlist, Piano Roll and Mixer as free windows, layouts, dark and light themes, command palette |
| Transport | Done | play, stop, Pattern and Song modes, song position, tempo, master volume and level |
| Basic MCP server | Done | `bodo-mcp` and `bodo-cli mcp` with generic and inspect tools; `--connect` drives the running app ([ADR 0019](adr/0019-agents-reach-the-app-through-the-control-channel.md)) |
| Control channel | Done | `bodo-control`: token-protected loopback channel, on by default |
| Factory content | Done | the Bodo Drums kit, generated and CC0, and one-shot samplers ([ADR 0020](adr/0020-factory-content-and-one-shot-samplers.md)) |
| `bodo-kit-ui` | Done | knob, fader, LED, step row, meter, line icons |
| `bodo-cli` | Done | `mcp`, `render`, `inspect`, `commands`, `pack`, `unpack` |

### Exit criteria

| Criterion | State |
|---|---|
| Build a 4-pattern beat with the step sequencer and sampler, arrange it, save and reopen it identically | Met. By clicking: `crates/bodo-ui/tests/beat.rs`; through commands: `crates/bodo-engine/tests/beat.rs`; it renders to audio where the steps are: `apps/bodo-cli/tests/render.rs` |
| An external MCP client can build the same beat | Met. `crates/bodo-mcp/tests/server.rs` drives the real MCP server through an MCP client only, saves, reopens and compares |

### Deferred

- **Detaching windows** into their own OS windows (ADR 0012), and the native macOS menu.
- **Sample preview** in the browser, and drag and drop onto the playlist.
- **Swing** is stored but not yet played; the sequencer applies it in M3.
- **Editing depth (M3):** the playlist's paint, slip, slice, select and zoom tools, piano-roll lanes and velocity, custom layouts, editable shortcuts.
- **The full mixer (M4):** the insert panel with FX slots, meters on every insert, solo.
- **MCP:** tool profiles, the streamable HTTP transport and limits on where agents may read and write files come with the assistant (M7).
- **The missing-file resolver** that finds a moved sample by its hash.
- **Opening a project in the running app** from the Hub or a second launch.
- **Headroom:** the master has no limiter yet, so stacking many hits can still go over 0 dBFS.

## M3: Editing depth (done on macOS, 2026-10-13)

Started and finished 2026-10-13. Every check passes on macOS; CI on Linux and Windows runs once these commits are pushed.

| Item | State | Where |
|---|---|---|
| Playlist tools | Done | all nine tools of section 10.3 with FL's keys, snap from a sixth of a step to a bar, Alt for no snap, loop region, markers, track heights, clip menu (make unique, split by channel, color, stretch, fades) |
| Playlist commands | Done | `clip.slice`, `clip.slip`, `clip.make_unique`, `clip.set_color`, `playlist.insert_time` and `delete_time`, `pattern.split_by_channel`, markers, copy, cut and paste |
| Piano roll tools | Done | the playlist's tools on notes (all but Slip), stamp, strum, arpeggiate, chop, glue, slice, legato, quantize with strength and swing, articulate, flip, transpose, humanize and un-humanize, randomize, limit to scale; scale highlighting and a chord readout (`bodo-kit-music`) |
| Lanes | Done | velocity, release, fine pitch, pan, Mod X and Y; MPE pitch, pressure and timbre curves per note; pitch bend, channel pressure and any CC as pattern lanes. Each stroke is one undo step |
| Automation clips | Done | a channel kind with clips on any track, pattern lanes, eight curve shapes, editing on the playlist, "Create automation clip" on rack knobs; played per block ([ADR 0024](adr/0024-automation-swing-and-the-song-loop.md)) |
| Audio clips and stretching | Done | waveforms, fades, slip and slice, resample or stretch to the tempo with Signalsmith Stretch, sounds dropped from the browser ([ADR 0021](adr/0021-audio-clips-and-time-stretching.md)) |
| Audio recording | Done | arm, input, tap and format per insert; takes written by a disk thread and placed as one undo step, with latency compensation ([ADR 0022](adr/0022-recording-audio-and-midi.md)) |
| MIDI devices | Done | MIDI inputs play the selected channel or bound channels, MIDI recording into the current pattern, metronome, count-in, typing keyboard ([ADR 0023](adr/0023-live-midi-input.md)) |
| SMF import and export | Done | `project.import_midi`, `project.export_midi`, `pattern.export_midi`, and the File menu |
| Swing and the Song loop | Done | swing plays; Shift+drag on the playlist ruler loops Song mode on the exact frame |

There are now 123 commands in [`agents/commands.md`](agents/commands.md).

### Exit criteria

| Criterion | State |
|---|---|
| Every row of the tool table in 10.3 covered by tests | Met. `crates/bodo-ui/tests/playlist_tools.rs` drives each of the nine tools with pointer events and checks the project, the undo history and what must stay out of it; `crates/bodo-engine/tests/playlist_tools.rs` covers the commands behind them. The piano roll's tools have their own: `crates/bodo-ui/tests/piano_roll_tools.rs` |
| Recorded audio lands within 1 ms after latency compensation | Met. `crates/bodo-mix/tests/record.rs` records an impulse through the virtual device's loopback at three latency pairs and finds it on the exact frame; `bodo-ui`'s audio tests run the app's own callback the same way and check the clip's place on the playlist |

### Deferred

- **Automation timing:** values are evaluated at the start of each block and ramped, so they can be a block late; splitting blocks at points is the fix ([ADR 0024](adr/0024-automation-swing-and-the-song-loop.md)). Lanes for CC, pitch bend, pressure, plugins, sends and tempo are stored and edited but not heard until their processors arrive (M4 to M6).
- **Live MIDI timing** is quantized to the audio buffer; only notes are routed, and nothing sends MIDI out yet.
- **Recording:** loop recording into separate takes, FLAC, and pre-FX and post-FX taps that differ from the input (they need M4's FX slots).
- **Audio clips:** files over 30 seconds load into memory, stretched copies render on the control thread, and a tempo change inside a clip uses the tempo at its start.
- **Playback tool** scrubbing doesn't play audio clips at varying speed.
- **Snapshot tolerance:** the interface snapshots allow 400 differing pixels, enough to hide a small regression; a tighter check per region would catch more.

## M4: Mixer (done on macOS, 2026-10-13)

Started and finished 2026-10-13, together with every interface gap the owner asked to close before M4 ends. Every check passes on macOS; CI on Linux and Windows runs once these commits are pushed.

| Item | State | Where |
|---|---|---|
| Full strip | Done | every control of section 9.2 in the order of 9.3: input sum, polarity, swap, 10 slots with enable and latency-compensated dry/wet, the 3-band strip EQ, stereo separation, pan with the project's pan law, a fader smoothed per sample, a 5 ms mute ramp, track latency, outputs ([ADR 0025](adr/0025-the-mixer-strip-as-graph-nodes.md)) |
| Routes, sends and sidechains | Done | channel multi-targets, insert routes at the PreFx, PreFader or PostFader tap, sidechain routes into slots, route exclusively, direct hardware outputs |
| Slots and effects | Done | native effects (Utility, Filter, Delay, Ducker) with parameter descriptors, presets, automation and generic editors ([ADR 0026](adr/0026-native-effects-in-fx-slots.md)) |
| Delay compensation | Done | per-edge delays, sidechain alignment, positive and negative track latency, PDC bypass, total latency in the mixer, the top bar and `inspect_mixer` |
| Solo set | Done | the soloed inserts, their feeders and what they feed; the rest keep running at zero gain |
| Cycle rejection | Done | a route that closes a loop is refused with the loop's path |
| Metering | Done | peak meters and clip lights on every insert, CPU per insert, the master's correlation and an oscilloscope |
| Disk recording | Done | real PreFx and PostFx taps, FLAC and Wave64, loop recording that makes a take per pass ([ADR 0027](adr/0027-recording-formats-loop-recording-and-stems.md)) |
| Stem export | Done | `render.export_stems`, `bodo-cli render --stems`, and File > Export, which renders on a background thread with progress and cancel |
| Mixer window | Done | strips with the route control, the insert panel, Wide, Compact and Extra compact views, the routing graph, group editing ([ADR 0031](adr/0031-the-mixer-window.md)) |
| Graph executor | Done | ADR 0015's deferred 5 ms crossfade on graph changes, buffer reuse and CPU time per node |

There are now 175 commands in [`agents/commands.md`](agents/commands.md).

### Exit criteria

| Criterion | State |
|---|---|
| PDC null tests pass on random graphs | Met. `delay_compensation_nulls_on_random_graphs` in `crates/bodo-mix/tests/mixer.rs` builds random inserts, routes at random taps and slots with random latencies, and nulls against the graph without latency at -100 dBFS or below |
| Cycle rejection returns the path | Met. `routes_that_close_a_loop_are_refused_with_the_path` in `crates/bodo-engine/tests/commands.rs`; the error JSON carries the loop, for audio and sidechain routes |
| Stems sum to the mix (null at -100 dBFS or below) | Met. `stems_sum_to_the_mix` in `crates/bodo-mix/tests/mixer.rs`, and `stems_sum_back_to_the_mix` in `apps/bodo-cli/tests/render.rs` through the command line |

### The interface gaps, closed

The owner noted on 2026-10-13 that much of section 14's interface was missing, and asked that no gap remain at the end of M4. Each line of the list kept here during M3 is now built:

- **Top bar:** loop record, the oscilloscope, auto scrolling, one-click audio recording, memory, CPU and latency in the performance readout.
- **Browser:** the Plugins tab and Plugin database, the library folders of section 14.1 in a Bodo folder in Documents, Recent files, presets (plugin, channel, mixer) and scores, back and up, tag filters, the Marketplace section, and previewing a sound by clicking it ([ADR 0029](adr/0029-the-user-library-presets-and-preview.md)).
- **Channel Rack:** the header (options menu, group filter, swing knob, graph editor, view buttons), the per-step graph editor, channel groups, reordering by dragging, colors, and MIDI input in a channel's menu ([ADR 0030](adr/0030-groups-solo-and-commands-that-render.md)).
- **Channel windows:** the full Sampler window of section 10.2 and a window for every other channel kind ([ADR 0028](adr/0028-the-sampler-and-channel-windows.md)).
- **Playlist:** track groups, track solo, a time selection apart from the loop, consolidating clips to audio, and curve tension handles.
- **Piano roll:** following the playhead, a note properties dialog, and showing or hiding ghost notes.
- **Mixer:** the insert panel, FX slots, sends and routing, meters on every insert, solo and the strip EQ.
- **Windows:** detaching into OS windows, saved layouts, the native macOS menu and editable shortcuts ([ADR 0032](adr/0032-detached-windows-layouts-menus-and-shortcuts.md)).
- **Accessibility and localization (14.7):** full keyboard navigation, screen-reader names and values, an interface size from 75% to 200% with Auto following the screen's width, a high-contrast theme, color-blind-safe meters and clips, and every interface string through translation with a complete Turkish catalog ([ADR 0033](adr/0033-keyboard-sizes-contrast-and-translations.md)).

Three panels of section 14.1 belong to later milestones by definition and arrive with their features: the AI Assistant (M7), the Marketplace window (M8), and editors for external plugins (M5; native effects already have editors).

### Deferred

- **Automation timing** is still read once per block ([ADR 0024](adr/0024-automation-swing-and-the-song-loop.md)); CC, pitch bend, pressure and tempo lanes are heard once instruments and the tempo follower arrive (M5, M6).
- **Plugins in other formats** (CLAP, VST3, AU) pass audio through, and instrument channels are silent, until plugin hosting (M5).
- **Stems** are taken where each insert enters the master, so they equal the mix when the master is at its defaults.
- **Meters and CPU** are not reported over MCP: the editing session doesn't own the audio engine.
- **Text the engine writes** (command errors, undo labels of agent edits) and default names in new projects stay English.
- **Context menus** open only with the pointer; everything in them is also in the menus or the palette.
- **Live MIDI** is quantized to the audio buffer, and only notes reach MIDI Out ports.
- **Keep on disk** streams only samples played as the file holds them; processed samples load into memory.
- **Consolidating clips** renders on the interface thread; it is short for a few clips, but a long selection pauses the window.

## M5 to M9

Not started.
