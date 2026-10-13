//! The audio device and engine, kept in step with the session (project.md sections 6.2 and 6.4).
//!
//! After every frame's commands, [`Audio::sync`] hands the engine the session's events (play, stop, seek, record, a
//! new project, the metronome and count-in) and its new snapshot when it changed, and reports the playhead back to the
//! session. MIDI inputs play channels live through a router that follows the channels' bindings and the selection, and
//! a finished MIDI recording becomes notes of the current pattern in one undo step. When no device can be opened the
//! app keeps working without sound and says why.
//!
//! **Recording.** The stream opens the input channels that armed inserts take. `transport.record` starts the engine
//! recording into `Recorded/` next to the project (a temporary folder for an unsaved one), with the stream's input
//! plus output latency as the compensation. When the transport stops, the takes come back and land on the playlist
//! through `recording.add_takes`, one undo step.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use bodo_engine::{Call, Event, PlayMode, Session};
use bodo_kit_core::{MAX_BLOCK_FRAMES, Seconds, Tick, rt_safe};
use bodo_kit_device::midi::{self, MidiInput};
use bodo_kit_device::{AudioCallback, AudioStream, Block, DeviceConfig, StreamStatus};
use bodo_mix::{AudioEngine, Compensation, Engine, EngineConfig, LiveInput, MidiRouter, Preview, PreviewPlayer, Take, TakeTarget};
use bodo_model::{ChannelId, Project};
use serde_json::json;

/// The audio and MIDI devices the user chose.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AudioPrefs {
    /// The driver API, such as CoreAudio or WASAPI; the platform default when empty.
    pub host: Option<String>,
    /// The output device; the host's default when empty.
    pub output: Option<String>,
    /// The input device armed inserts record from; the host's default when empty.
    pub input: Option<String>,
    /// Frames per callback; 256 when empty.
    pub buffer_frames: Option<u32>,
    /// The MIDI inputs to open, by name; every input when empty.
    pub midi_inputs: Option<Vec<String>>,
}

/// The master's peak level since the UI last looked, per side, shared with the audio thread.
#[derive(Debug, Default)]
pub struct Meter {
    left: AtomicU32,
    right: AtomicU32,
}

impl Meter {
    /// Raises the stored peaks to the peaks of `left` and `right`.
    ///
    /// RT-safe. Complexity: O(frames).
    #[rt_safe]
    fn record(&self, left: &[f32], right: &[f32]) {
        let peak = |samples: &[f32]| samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        for (cell, value) in [(&self.left, peak(left)), (&self.right, peak(right))] {
            let old = f32::from_bits(cell.load(Ordering::Relaxed));
            if value > old {
                cell.store(value.to_bits(), Ordering::Relaxed);
            }
        }
    }

    /// The peaks since the last call, left and right, as linear levels; resets them.
    pub fn take(&self) -> (f32, f32) {
        (f32::from_bits(self.left.swap(0, Ordering::Relaxed)), f32::from_bits(self.right.swap(0, Ordering::Relaxed)))
    }
}

/// The audio thread's side: the engine, the Browser's preview after the master, then the meter.
struct Callback {
    engine: AudioEngine,
    preview: PreviewPlayer,
    meter: Arc<Meter>,
}

impl AudioCallback for Callback {
    /// RT-safe.
    #[rt_safe]
    fn process(&mut self, block: &mut Block<'_>) {
        self.engine.render_with_input(block.frames, Some(block.input), block.output);
        self.preview.mix_into(block.output, block.frames);
        let channels = block.output.channels();
        let left = block.output.channel(0);
        let right = block.output.channel(1.min(channels.saturating_sub(1)));
        self.meter.record(&left[..block.frames.min(left.len())], &right[..block.frames.min(right.len())]);
    }
}

struct Running {
    stream: AudioStream,
    engine: Engine,
    /// The Browser's sound preview.
    preview: Preview,
    live: LiveInput,
    router: Arc<MidiRouter>,
    /// The open MIDI inputs; dropping one closes it.
    midi: Vec<MidiInput>,
    /// The project revision and selected channel the router last followed.
    routed: Option<(u64, Option<ChannelId>)>,
    /// Where the notes of the recording under way go.
    take: Option<TakeTarget>,
    /// How many input channels the armed inserts took when the stream opened, whether or not the device had them.
    wanted_inputs: usize,
}

/// Opens the MIDI inputs in `prefs`, or every input, each playing through `router`. Inputs that fail to open are
/// skipped and logged.
fn open_midi(prefs: &AudioPrefs, router: &Arc<MidiRouter>) -> Vec<MidiInput> {
    let names = prefs.midi_inputs.clone().unwrap_or_else(midi::input_ports);
    names
        .into_iter()
        .filter_map(|name| {
            let (router, port) = (Arc::clone(router), name.clone());
            match MidiInput::open(&name, move |event| router.route(&port, event.channel, event.message)) {
                Ok(input) => Some(input),
                Err(error) => {
                    tracing::warn!(%error, "a MIDI input could not open");
                    None
                }
            }
        })
        .collect()
}

/// Where a recording started now in `session` puts its notes: the current pattern, wrapped into its loop in Pattern
/// mode or relative to the bar where recording begins in Song mode.
fn take_target(session: &Session) -> Option<TakeTarget> {
    let pattern = session.editor().pattern?;
    Some(match session.transport().mode {
        PlayMode::Pattern => TakeTarget::Pattern(pattern),
        PlayMode::Song => TakeTarget::Song(pattern),
    })
}

/// How many device input channels the armed inserts of `project` take: the highest channel plus one.
///
/// Complexity: O(i) in inserts.
fn needed_inputs(project: &Project) -> usize {
    project
        .mixer
        .inserts
        .values()
        .chain(std::iter::once(&project.mixer.master))
        .filter(|insert| insert.armed)
        .filter_map(|insert| insert.input.as_ref())
        .flat_map(|port| port.channels.iter())
        .map(|channel| usize::from(*channel) + 1)
        .max()
        .unwrap_or(0)
}

/// Where takes are written: `Recorded/` beside the project's file, the user library's Recorded folder for an unsaved
/// project, or a temporary folder when there is no library.
fn recording_folder(session: &Session) -> PathBuf {
    let project_dir = session.file().map(bodo_io::project_folder);
    bodo_engine::library::recorded_folder(session.env(), project_dir.as_deref()).unwrap_or_else(|| std::env::temp_dir().join("Bodo Recordings"))
}

/// Places `takes` on the playlist through the session, as one undo step.
fn place_takes(session: &mut Session, takes: &[Take]) -> Result<usize, String> {
    let project = Arc::clone(session.project());
    let rate = project.settings.sample_rate;
    let specs: Vec<_> = takes
        .iter()
        .map(|take| {
            json!({
                "insert": take.insert.to_string(),
                "path": take.path.display().to_string(),
                "start": take.timeline_tick(&project.tempo, rate),
                "frames": take.frames,
            })
        })
        .collect();
    if specs.is_empty() {
        return Ok(0);
    }
    session.execute("recording.add_takes", json!({ "takes": specs })).map(|_| takes.len()).map_err(|e| e.to_string())
}

/// The audio device and engine.
pub struct Audio {
    running: Option<Running>,
    prefs: AudioPrefs,
    status: String,
    seen_revision: Option<u64>,
    sent_mode: Option<bodo_mix::PlayMode>,
    meter: Arc<Meter>,
    levels: (f32, f32),
    stream_status: StreamStatus,
}

impl std::fmt::Debug for Audio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Audio").field("status", &self.status).finish_non_exhaustive()
    }
}

impl Audio {
    /// No audio: commands and the interface work, nothing plays.
    pub fn off() -> Self {
        Self {
            running: None,
            prefs: AudioPrefs::default(),
            status: String::from("Audio is off"),
            seen_revision: None,
            sent_mode: None,
            meter: Arc::new(Meter::default()),
            levels: (0.0, 0.0),
            stream_status: StreamStatus::default(),
        }
    }

    /// Opens the device in `prefs` and an engine for `session`'s project.
    pub fn start(session: &Session, prefs: &AudioPrefs) -> Self {
        let mut audio = Self::off();
        audio.open(session, prefs);
        audio
    }

    /// Opens (or reopens) the device with `prefs` for `session`'s project.
    pub fn open(&mut self, session: &Session, prefs: &AudioPrefs) {
        self.running = None;
        self.prefs = prefs.clone();
        self.seen_revision = Some(session.revision());
        self.sent_mode = None;
        let project = session.project();
        let rate = project.settings.sample_rate;
        let workers = std::thread::available_parallelism().map_or(0, |n| n.get().saturating_sub(2).min(7));
        let config = EngineConfig { workers, max_frames: MAX_BLOCK_FRAMES, realtime_buffer: Some(prefs.buffer_frames.unwrap_or(256)) };
        // A missing or smaller input device costs only recording: without the armed inputs the app still plays.
        let wanted_inputs = needed_inputs(project);
        let attempts: &[usize] = if wanted_inputs > 0 { &[wanted_inputs, 0] } else { &[0] };
        let mut input_error = None;
        for &inputs in attempts {
            let (mut engine, audio) = match bodo_mix::engine(project, &session.sample_roots(), config) {
                Ok(pair) => pair,
                Err(error) => {
                    self.status = format!("The audio engine could not start: {error}");
                    return;
                }
            };
            let device = DeviceConfig {
                host: prefs.host.clone(),
                output: prefs.output.clone(),
                input: prefs.input.clone(),
                sample_rate: rate,
                buffer_frames: Some(prefs.buffer_frames.unwrap_or(256)),
                input_map: (0..inputs).collect(),
                ..DeviceConfig::default()
            };
            let (preview, preview_player) = bodo_mix::preview(rate);
            match bodo_kit_device::open(&device, Callback { engine: audio, preview: preview_player, meter: Arc::clone(&self.meter) }) {
                Ok(stream) => {
                    engine.set_thread_group(stream.thread_group());
                    let transport = session.transport();
                    engine.set_metronome(transport.metronome);
                    engine.set_count_in(transport.count_in);
                    engine.set_loop(transport.loop_region);
                    let live = engine.live_input();
                    let router = MidiRouter::new(live.clone());
                    router.update(project, session.editor().channel);
                    let midi = open_midi(prefs, &router);
                    let routed = Some((session.revision(), session.editor().channel));
                    let inputs_note = match (&input_error, inputs) {
                        (Some(error), _) => format!("; no inputs: {error}"),
                        (None, 0) => String::new(),
                        (None, n) => format!(", {n} inputs"),
                    };
                    self.status = format!("{} kHz, {} frames{inputs_note}", f64::from(rate.hz()) / 1000.0, prefs.buffer_frames.unwrap_or(256));
                    self.running = Some(Running { stream, engine, preview, live, router, midi, routed, take: None, wanted_inputs });
                    return;
                }
                Err(error) if inputs > 0 => {
                    tracing::warn!(%error, inputs, "the armed inputs could not be opened");
                    input_error = Some(error);
                }
                Err(error) => {
                    tracing::warn!(%error, "no audio device");
                    self.status = format!("No sound: {error}");
                }
            }
        }
    }

    /// Whether a device is open.
    pub fn is_on(&self) -> bool {
        self.running.is_some()
    }

    /// Whether the engine is playing.
    pub fn is_playing(&self) -> bool {
        self.running.as_ref().is_some_and(|r| r.engine.status().playing)
    }

    /// What the device is doing, or why there is no sound.
    pub fn status(&self) -> &str {
        &self.status
    }

    /// The device in use.
    pub fn prefs(&self) -> &AudioPrefs {
        &self.prefs
    }

    /// The master's peak levels, decaying, for the meters.
    pub fn levels(&self) -> (f32, f32) {
        self.levels
    }

    /// The device's load and dropout counters.
    pub fn stream_status(&self) -> StreamStatus {
        self.stream_status
    }

    /// Presses (`on`) or releases key `key` on `channel` with `velocity` from 0 to 1, right away, as a MIDI keyboard
    /// would; the computer keyboard plays through this. Returns false when there is no sound or the queue is full.
    pub fn play_note(&self, channel: ChannelId, key: u8, velocity: f32, on: bool) -> bool {
        self.running.as_ref().is_some_and(|r| r.live.note(channel, key, velocity, on))
    }

    /// Plays the audio file at `path` on its own, after the master, replacing any sound previewing now. Returns false
    /// when there is no sound.
    pub fn preview(&mut self, path: &std::path::Path) -> bool {
        match self.running.as_mut() {
            Some(running) => {
                running.preview.play(path);
                true
            }
            None => false,
        }
    }

    /// Stops the preview, fading it out.
    pub fn stop_preview(&mut self) {
        if let Some(running) = self.running.as_mut() {
            running.preview.stop();
        }
    }

    /// The file previewing now, while it loads or plays.
    pub fn previewing(&self) -> Option<&std::path::Path> {
        self.running.as_ref().and_then(|r| r.preview.current())
    }

    /// The names of the MIDI inputs that are open.
    pub fn midi_inputs(&self) -> Vec<&str> {
        self.running.as_ref().map(|r| r.midi.iter().map(MidiInput::name).collect()).unwrap_or_default()
    }

    /// Whether a count-in is clicking before the recording starts.
    pub fn is_counting_in(&self) -> bool {
        self.running.as_ref().is_some_and(|r| r.engine.status().counting_in)
    }

    /// Brings the engine up to date with `session` and reports the playhead back. Call once per frame.
    ///
    /// Allocates when the project changed. Complexity: the engine's update, O(V + E) plus the song index when notes or
    /// clips changed.
    pub fn sync(&mut self, session: &mut Session) {
        let events = session.take_events();
        // Armed inputs the stream doesn't carry need it reopened first, so a recording started this frame hears them;
        // never in the middle of a recording, though.
        if self.running.as_ref().is_some_and(|r| !r.engine.is_recording() && needed_inputs(session.project()) != r.wanted_inputs) {
            let prefs = self.prefs.clone();
            self.open(session, &prefs);
        }
        let Some(running) = self.running.as_mut() else { return };
        if let Some(error) = running.preview.poll() {
            self.status = format!("Could not preview: {error}");
        }
        let mut reopen = false;
        for event in events {
            match event {
                Event::Play => running.engine.play(),
                Event::Stop => {
                    running.engine.stop();
                    if running.engine.is_recording() {
                        running.engine.stop_recording();
                    }
                }
                Event::Seek(tick) => running.engine.seek(tick),
                Event::ProjectReplaced => reopen = true,
                Event::Saved(_) => running.engine.set_sample_roots(session.sample_roots()),
                Event::Record => {
                    if running.engine.set_project(session.project()).is_ok() {
                        self.seen_revision = Some(session.revision());
                    }
                    // Audio records only from armed inserts; without any, a recording takes only notes.
                    let mixer = &session.project().mixer;
                    let armed = mixer.inserts.values().chain(std::iter::once(&mixer.master)).any(|insert| insert.armed);
                    let compensation = Compensation::from_status(&running.stream.status());
                    self.status = match armed.then(|| running.engine.start_recording(session.project(), &recording_folder(session), compensation)) {
                        None => String::from("Recording notes"),
                        Some(Ok(count)) => format!("Recording {count} insert{}", if count == 1 { "" } else { "s" }),
                        Some(Err(error)) => format!("Not recording audio: {error}"),
                    };
                    running.take = take_target(session);
                    running.engine.record();
                }
                Event::Metronome(on) => running.engine.set_metronome(on),
                Event::CountIn(on) => running.engine.set_count_in(on),
                Event::Loop(region) => running.engine.set_loop(region),
            }
        }
        if reopen || session.project().settings.sample_rate != running.stream.rate() {
            let prefs = self.prefs.clone();
            self.open(session, &prefs);
            return;
        }
        if self.seen_revision != Some(session.revision()) {
            if let Err(error) = running.engine.set_project(session.project()) {
                tracing::warn!(%error, "the engine could not follow the project");
            }
            self.seen_revision = Some(session.revision());
        }
        let mode = match session.transport().mode {
            PlayMode::Song => Some(bodo_mix::PlayMode::Song),
            PlayMode::Pattern => session.editor().pattern.map(bodo_mix::PlayMode::Pattern),
        };
        if let Some(mode) = mode
            && self.sent_mode != Some(mode)
        {
            running.engine.set_mode(mode);
            self.sent_mode = Some(mode);
        }
        let routes = Some((session.revision(), session.editor().channel));
        if running.routed != routes {
            running.router.update(session.project(), session.editor().channel);
            running.routed = routes;
        }
        running.engine.flush();
        if let Some(take) = running.engine.take_midi_recording()
            && let Some(target) = running.take.take()
        {
            let rate = session.project().settings.sample_rate;
            let notes = bodo_mix::midi_record::place(&take, session.project(), rate, target);
            add_take(session, target, &notes);
        }
        let status = running.engine.status();
        if running.engine.is_recording()
            && let Some(done) = running.engine.finish_recording()
        {
            self.status = match done.map_err(|e| e.to_string()).and_then(|takes| place_takes(session, &takes)) {
                Ok(count) => format!("Recorded {count} take{}", if count == 1 { "" } else { "s" }),
                Err(error) => format!("The recording could not be placed: {error}"),
            };
        }
        let project = session.project();
        let seconds = status.frame as f64 / f64::from(project.settings.sample_rate.hz());
        let mut tick = project.tempo.ticks_at(Seconds(seconds)).round() as i64;
        if let (PlayMode::Pattern, Some(pattern)) = (session.transport().mode, session.editor().pattern) {
            let length = bodo_engine::pattern_length(project, pattern).0.max(1);
            tick = tick.rem_euclid(length);
        }
        session.set_playhead(Tick(tick), status.playing);
        self.stream_status = running.stream.status();
        if self.stream_status.disconnected {
            self.running = None;
            self.status = String::from("The audio device went away; open Options > Audio settings to reconnect");
            return;
        }
        let (left, right) = self.meter.take();
        let decay = 0.85;
        self.levels = (left.max(self.levels.0 * decay), right.max(self.levels.1 * decay));
    }
}

/// Adds the recorded `notes` to `target`'s pattern with `note.add_many`, one call per channel in one batch, so the take
/// is one undo step.
fn add_take(session: &mut Session, target: TakeTarget, notes: &[bodo_mix::RecordedNote]) {
    let mut by_channel: BTreeMap<ChannelId, Vec<serde_json::Value>> = BTreeMap::new();
    for note in notes {
        by_channel
            .entry(note.channel)
            .or_default()
            .push(json!({ "key": note.key, "start": note.start.0, "length": note.len.0, "velocity": note.velocity }));
    }
    let calls: Vec<Call> = by_channel
        .into_iter()
        .map(|(channel, notes)| Call::new("note.add_many", json!({ "pattern": target.pattern(), "channel": channel, "notes": notes })))
        .collect();
    if calls.is_empty() {
        return;
    }
    if let Err(error) = session.batch(&calls, Some("Record MIDI")) {
        tracing::warn!(%error, "the MIDI recording could not be added");
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use bodo_engine::Env;
    use bodo_kit_device::{Pacing, VirtualConfig, VirtualDevice};
    use bodo_mix::RecordedNote;
    use bodo_model::playlist::ClipSource;

    use super::*;

    #[test]
    fn a_midi_take_lands_in_the_pattern_as_one_undo_step() {
        let mut session = Session::deterministic(Env::default());
        let channel: ChannelId =
            serde_json::from_value(session.execute("channel.add", json!({ "name": "Keys" })).unwrap().result["channel"].clone()).unwrap();
        let pattern = session.editor().pattern.unwrap();
        let target = take_target(&session).unwrap();
        assert_eq!(target, TakeTarget::Pattern(pattern), "new sessions are in Pattern mode");
        let steps = session.undo_labels().count();
        let note = |key, start| RecordedNote { channel, key, start: Tick(start), len: Tick(240), velocity: 0.5 };
        add_take(&mut session, target, &[note(60, 0), note(64, 480)]);
        let summary = bodo_engine::inspect::pattern(&session, None).unwrap().text;
        assert!(summary.contains("C4 at 1:1:0") && summary.contains("E4 at 1:1:480"), "{summary}");
        assert_eq!(session.undo_labels().count(), steps + 1);
        assert_eq!(session.undo_labels().last(), Some("Record MIDI"));
        add_take(&mut session, target, &[]);
        assert_eq!(session.undo_labels().count(), steps + 1, "an empty take changes nothing");
    }

    /// The app's own callback on a loopback device: an armed insert records the device input, and the take lands on
    /// the playlist at its compensated start.
    #[test]
    fn armed_inserts_record_the_device_input_and_the_take_lands_on_the_playlist() {
        let dir = std::env::temp_dir().join(format!("bodo-ui-record-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut session = Session::deterministic(Env::default());
        session.execute("project.save", json!({ "path": dir.join("Song.bodo").display().to_string() })).unwrap();
        assert_eq!(recording_folder(&session), bodo_io::project_folder(&dir.join("Song.bodo")).join("Recorded"));

        session.execute("mixer.set_input", json!({ "insert": 2, "channels": [2, 3] })).unwrap();
        assert_eq!(needed_inputs(session.project()), 0, "inputs count once their insert is armed");
        session.execute("mixer.arm", json!({ "insert": 2, "on": true })).unwrap();
        assert_eq!(needed_inputs(session.project()), 4);
        session.execute("mixer.set_input", json!({ "insert": 2, "channels": [0, 1] })).unwrap();
        assert_eq!(needed_inputs(session.project()), 2);

        let project = Arc::clone(session.project());
        let (mut engine, audio) =
            bodo_mix::engine(&project, &session.sample_roots(), EngineConfig { max_frames: 128, ..EngineConfig::default() }).unwrap();
        let config = VirtualConfig {
            buffer_frames: 128,
            input_channels: 2,
            loopback: Some(256),
            input_latency: 128,
            output_latency: 128,
            pacing: Pacing::Freewheel,
            ..VirtualConfig::default()
        };
        let preview = bodo_mix::preview(project.settings.sample_rate).1;
        let mut device = VirtualDevice::new(config, Callback { engine: audio, preview, meter: Arc::new(Meter::default()) });
        let compensation = Compensation::from_status(&device.status());
        assert_eq!(engine.start_recording(&project, &recording_folder(&session), compensation).unwrap(), 1);
        engine.seek(Tick(3840));
        engine.play();
        let end = 3 * i64::from(project.settings.sample_rate.hz());
        while engine.status().frame < end {
            device.render();
        }
        engine.stop();
        engine.stop_recording();
        let takes = (0..5_000)
            .find_map(|_| {
                device.render();
                std::thread::sleep(std::time::Duration::from_millis(1));
                engine.finish_recording()
            })
            .unwrap()
            .unwrap();
        assert_eq!(takes.len(), 1);
        assert_eq!(place_takes(&mut session, &takes).unwrap(), 1);

        // The take began at bar 2 by the transport; heard back 256 frames late, it moves that much earlier.
        let project = session.project();
        let start = Tick(takes[0].timeline_tick(&project.tempo, project.settings.sample_rate));
        assert!(start < Tick(3840) && start > Tick(3800), "{start:?}");
        let channel = project.channels.values().find(|c| c.name.ends_with(" take 1")).unwrap().id;
        let clip = project.tracks.values().flat_map(|t| t.clips.iter()).find(|c| c.source == ClipSource::Audio(channel)).unwrap();
        assert_eq!(clip.start, start);
        let _ = std::fs::remove_dir_all(dir);
    }
}
