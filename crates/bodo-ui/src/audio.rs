//! The audio device and engine, kept in step with the session (project.md sections 6.2 and 6.4).
//!
//! After every frame's commands, [`Audio::sync`] hands the engine the session's events (play, stop, seek, record, a
//! new project, the metronome and count-in) and its new snapshot when it changed, and reports the playhead back to the
//! session. MIDI inputs play channels live through a router that follows the channels' bindings and the selection, and
//! a finished MIDI recording becomes notes of the current pattern in one undo step. When no device can be opened the
//! app keeps working without sound and says why.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use bodo_engine::{Call, Event, PlayMode, Session};
use bodo_kit_core::{MAX_BLOCK_FRAMES, Seconds, Tick, rt_safe};
use bodo_kit_device::midi::{self, MidiInput};
use bodo_kit_device::{AudioCallback, AudioStream, Block, DeviceConfig, StreamStatus};
use bodo_mix::{AudioEngine, Engine, EngineConfig, LiveInput, MidiRouter, TakeTarget};
use bodo_model::ChannelId;
use serde_json::json;

/// The audio and MIDI devices the user chose.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AudioPrefs {
    /// The driver API, such as CoreAudio or WASAPI; the platform default when empty.
    pub host: Option<String>,
    /// The output device; the host's default when empty.
    pub output: Option<String>,
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

/// The audio thread's side: the engine, then the meter.
struct Callback {
    engine: AudioEngine,
    meter: Arc<Meter>,
}

impl AudioCallback for Callback {
    /// RT-safe.
    #[rt_safe]
    fn process(&mut self, block: &mut Block<'_>) {
        self.engine.render(block.frames, block.output);
        let channels = block.output.channels();
        let left = block.output.channel(0);
        let right = block.output.channel(1.min(channels.saturating_sub(1)));
        self.meter.record(&left[..block.frames.min(left.len())], &right[..block.frames.min(right.len())]);
    }
}

struct Running {
    stream: AudioStream,
    engine: Engine,
    live: LiveInput,
    router: Arc<MidiRouter>,
    /// The open MIDI inputs; dropping one closes it.
    midi: Vec<MidiInput>,
    /// The project revision and selected channel the router last followed.
    routed: Option<(u64, Option<ChannelId>)>,
    /// Where the notes of the recording under way go.
    take: Option<TakeTarget>,
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
        let (engine, audio) = match bodo_mix::engine(project, &session.sample_roots(), config) {
            Ok(pair) => pair,
            Err(error) => {
                self.status = format!("The audio engine could not start: {error}");
                return;
            }
        };
        let device = DeviceConfig {
            host: prefs.host.clone(),
            output: prefs.output.clone(),
            sample_rate: rate,
            buffer_frames: Some(prefs.buffer_frames.unwrap_or(256)),
            ..DeviceConfig::default()
        };
        match bodo_kit_device::open(&device, Callback { engine: audio, meter: Arc::clone(&self.meter) }) {
            Ok(stream) => {
                engine.set_thread_group(stream.thread_group());
                engine.set_metronome(session.transport().metronome);
                engine.set_count_in(session.transport().count_in);
                let live = engine.live_input();
                let router = MidiRouter::new(live.clone());
                router.update(project, session.editor().channel);
                let midi = open_midi(prefs, &router);
                let routed = Some((session.revision(), session.editor().channel));
                self.status = format!("{} kHz, {} frames", f64::from(rate.hz()) / 1000.0, prefs.buffer_frames.unwrap_or(256));
                self.running = Some(Running { stream, engine, live, router, midi, routed, take: None });
            }
            Err(error) => {
                tracing::warn!(%error, "no audio device");
                self.status = format!("No sound: {error}");
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
        let Some(running) = self.running.as_mut() else { return };
        let mut reopen = false;
        for event in events {
            match event {
                Event::Play => running.engine.play(),
                Event::Stop => running.engine.stop(),
                Event::Seek(tick) => running.engine.seek(tick),
                Event::ProjectReplaced => reopen = true,
                Event::Saved(_) => running.engine.set_sample_roots(session.sample_roots()),
                Event::Record => {
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
    use bodo_engine::Env;
    use bodo_mix::RecordedNote;

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
}
