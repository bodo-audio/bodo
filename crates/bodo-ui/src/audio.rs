//! The audio device and engine, kept in step with the session (project.md sections 6.2 and 6.4).
//!
//! After every frame's commands, [`Audio::sync`] hands the engine the session's events (play, stop, seek, a new
//! project) and its new snapshot when it changed, and reports the playhead back to the session. When no device can
//! be opened the app keeps working without sound and says why.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use bodo_engine::{Event, PlayMode, Session};
use bodo_kit_core::{MAX_BLOCK_FRAMES, Seconds, Tick, rt_safe};
use bodo_kit_device::{AudioCallback, AudioStream, Block, DeviceConfig, StreamStatus};
use bodo_mix::{AudioEngine, Engine, EngineConfig};

/// The audio device the user chose.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AudioPrefs {
    /// The driver API, such as CoreAudio or WASAPI; the platform default when empty.
    pub host: Option<String>,
    /// The output device; the host's default when empty.
    pub output: Option<String>,
    /// Frames per callback; 256 when empty.
    pub buffer_frames: Option<u32>,
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
                self.status = format!("{} kHz, {} frames", f64::from(rate.hz()) / 1000.0, prefs.buffer_frames.unwrap_or(256));
                self.running = Some(Running { stream, engine });
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
                // Recording, looping, the metronome and count-in arrive with their engine features in M3; until then
                // recording only plays.
                Event::Record => running.engine.play(),
                Event::Loop(_) | Event::Metronome(_) | Event::CountIn(_) => {}
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
        running.engine.flush();
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
