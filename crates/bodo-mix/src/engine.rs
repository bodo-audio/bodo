//! The engine: [`Engine`] on the control thread, [`AudioEngine`] on the audio thread (project.md section 6.4).
//!
//! The control side compiles plans and song indexes and sends them, with transport commands, through bounded queues.
//! The audio side switches to whatever arrived at the start of a block, takes in notes played live, runs the
//! sequencer and the graph, copies the master into the device's output and adds the metronome. Neither side ever
//! waits for the other.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};

use bodo_kit_core::TempoMap;
use bodo_kit_core::handoff::{self, Receiver, Sender};
use bodo_kit_core::pool::{RtPoolConfig, RtPriority, ThreadGroup, ThreadGroupSlot};
use bodo_kit_core::queue::{Consumer, Producer, spsc};
use bodo_kit_core::{MAX_BLOCK_FRAMES, SampleRate, Tick, rt_safe};
use bodo_kit_device::{AudioCallback, Block};
use bodo_kit_dsp::AudioBuffer;
use bodo_kit_graph::{GraphRunner, LatencyReport, Plan, PlanSender, runner};
use bodo_model::meter::MeterMap;
use bodo_model::{Pattern, PatternId, PlaylistTrack, Project, SampleRoots, TrackId};
use imbl::{OrdMap, Vector};

use crate::MixError;
use crate::compile::Compiler;
use crate::context::{BlockContext, EventKind, NoteEvent};
use crate::input::{LIVE_CAPACITY, LiveEvent, LiveInput};
use crate::metronome::{Metronome, count_in_beats};
use crate::midi_record::{CAPTURE_CAPACITY, Capture, CapturedNote, CapturedTake};
use crate::nodes::MixNode;
use crate::sequence::{PlayMode, SongIndex, Transport, all_notes_off, emit, frame_at};

/// Marks the voices of live notes, so they never collide with the sequencer's.
const LIVE_VOICE: u64 = 1 << 63;
/// The most channels with live notes a stop releases.
const LIVE_CHANNELS: usize = 256;

/// How to build an engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EngineConfig {
    /// RT worker threads besides the audio thread.
    pub workers: usize,
    /// The largest block the engine renders at once; larger requests are rendered in pieces.
    pub max_frames: usize,
    /// Promote the workers to real-time priority for this device buffer size, in frames.
    pub realtime_buffer: Option<u32>,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self { workers: 0, max_frames: MAX_BLOCK_FRAMES, realtime_buffer: None }
    }
}

/// A transport command for the audio thread.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Command {
    Play,
    /// Play and capture live notes, after a bar of count-in clicks when count-in is on.
    Record,
    Stop,
    Seek(i64),
    Mode(PlayMode),
}

/// What the audio side reports back.
#[derive(Debug, Default)]
struct Shared {
    playing: AtomicBool,
    frame: AtomicI64,
    dropped_events: AtomicU64,
    metronome: AtomicBool,
    count_in: AtomicBool,
    counting_in: AtomicBool,
    recording: AtomicBool,
}

/// A snapshot of the engine's state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EngineStatus {
    /// Whether the transport is playing.
    pub playing: bool,
    /// The playhead in frames from tick 0.
    pub frame: i64,
    /// Note events dropped because a block had too many.
    pub dropped_events: u64,
    /// Whether a count-in is clicking before the playhead moves.
    pub counting_in: bool,
    /// Whether live notes are being recorded.
    pub recording: bool,
}

/// The control-thread side of the engine.
pub struct Engine {
    compiler: Compiler,
    plans: PlanSender<MixNode>,
    pending_plan: Option<Box<Plan<MixNode>>>,
    songs: Sender<Box<SongIndex>>,
    pending_song: Option<Box<SongIndex>>,
    commands: Producer<Command>,
    shared: Arc<Shared>,
    song: SongIndex,
    song_inputs: SongInputs,
    rate: SampleRate,
    thread_group: ThreadGroupSlot,
    live: LiveInput,
    captures: Consumer<Capture>,
    capture_start: Option<i64>,
    captured: Vec<CapturedNote>,
}

/// What a song index is built from. Snapshots share unchanged parts, so comparing pointers tells cheaply whether an
/// edit can have changed the index.
#[derive(Clone)]
struct SongInputs {
    tempo: Arc<TempoMap>,
    meter: Arc<MeterMap>,
    patterns: OrdMap<PatternId, Arc<Pattern>>,
    tracks: OrdMap<TrackId, Arc<PlaylistTrack>>,
    track_order: Vector<TrackId>,
}

impl SongInputs {
    fn of(project: &Project) -> Self {
        Self {
            tempo: Arc::clone(&project.tempo),
            meter: Arc::clone(&project.meter),
            patterns: project.patterns.clone(),
            tracks: project.tracks.clone(),
            track_order: project.track_order.clone(),
        }
    }

    fn same_as(&self, project: &Project) -> bool {
        Arc::ptr_eq(&self.tempo, &project.tempo)
            && Arc::ptr_eq(&self.meter, &project.meter)
            && self.patterns.ptr_eq(&project.patterns)
            && self.tracks.ptr_eq(&project.tracks)
            // imbl keeps tiny vectors inline, where pointers never match; comparing those by value is cheap.
            && (self.track_order.ptr_eq(&project.track_order) || self.track_order == project.track_order)
    }
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine").field("status", &self.status()).finish_non_exhaustive()
    }
}

/// The audio-thread side of the engine. Hand it to a device as its callback.
pub struct AudioEngine {
    runner: GraphRunner<MixNode, BlockContext>,
    song: Box<SongIndex>,
    songs: Receiver<Box<SongIndex>>,
    commands: Consumer<Command>,
    transport: Transport,
    shared: Arc<Shared>,
    rate: SampleRate,
    max_frames: usize,
    /// Notes played live, and this block's share of them as events. Capacity [`LIVE_CAPACITY`].
    live: LiveInput,
    live_events: Vec<(u64, NoteEvent)>,
    /// Channels with live notes since the last stop. Capacity [`LIVE_CHANNELS`].
    live_channels: Vec<u64>,
    metronome: Metronome,
    /// What the last block was, for the metronome.
    block: BlockKind,
    /// Count-in frames still to click, the count-in's length, and its beat in frames.
    count_in_left: i64,
    count_in_total: i64,
    count_in_beat: i64,
    /// Whether live notes are captured, and whether the take's start was sent.
    capturing: bool,
    capture_started: bool,
    captures: Producer<Capture>,
}

/// What a block played, for the metronome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BlockKind {
    /// Nothing moved.
    Still,
    /// The song played from this timeline frame.
    Song(i64),
    /// The count-in clicked, this many frames in.
    CountIn(i64),
}

impl std::fmt::Debug for AudioEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioEngine").field("transport", &self.transport).finish_non_exhaustive()
    }
}

/// Builds the two sides of an engine for `project`, whose sample paths resolve through `roots`. The engine renders at
/// the project's sample rate.
///
/// Compiles the first plan, loads samples and spawns the workers.
pub fn engine(project: &Project, roots: &SampleRoots, config: EngineConfig) -> Result<(Engine, AudioEngine), MixError> {
    let rate = project.settings.sample_rate;
    let max_frames = config.max_frames.clamp(1, MAX_BLOCK_FRAMES);
    let mut compiler = Compiler::new(roots.clone(), max_frames, rate);
    let plan = compiler.update(project)?.unwrap_or_else(|| Plan::empty(max_frames));
    let pool = RtPoolConfig {
        priority: config.realtime_buffer.map(|buffer_frames| RtPriority { buffer_frames, sample_rate: rate }),
        ..RtPoolConfig::new(config.workers)
    };
    let (graph, plans) = runner(plan, BlockContext::new(rate), pool)?;
    let thread_group = graph.thread_group();
    let song = SongIndex::build(project);
    let (songs_tx, songs_rx) = handoff::handoff(4);
    let (commands_tx, commands_rx) = spsc(64);
    let (captures_tx, captures_rx) = spsc(CAPTURE_CAPACITY);
    let live = LiveInput::new(LIVE_CAPACITY);
    let shared = Arc::new(Shared::default());
    let control = Engine {
        compiler,
        plans,
        pending_plan: None,
        songs: songs_tx,
        pending_song: None,
        commands: commands_tx,
        shared: Arc::clone(&shared),
        song: song.clone(),
        song_inputs: SongInputs::of(project),
        rate,
        thread_group,
        live: live.clone(),
        captures: captures_rx,
        capture_start: None,
        captured: Vec::new(),
    };
    let audio = AudioEngine {
        runner: graph,
        song: Box::new(song),
        songs: songs_rx,
        commands: commands_rx,
        transport: Transport::default(),
        shared,
        rate,
        max_frames,
        live,
        live_events: Vec::with_capacity(LIVE_CAPACITY),
        live_channels: Vec::with_capacity(LIVE_CHANNELS),
        metronome: Metronome::new(rate),
        block: BlockKind::Still,
        count_in_left: 0,
        count_in_total: 0,
        count_in_beat: 1,
        capturing: false,
        capture_started: false,
        captures: captures_tx,
    };
    Ok((control, audio))
}

impl Engine {
    /// Brings the engine up to date with `project`: live parameters at once, and a new plan and song index when
    /// they changed. The audio side switches over at its next block.
    ///
    /// Allocates. Complexity: O(V + E), plus O(N log N) in notes and clips when the patterns, playlist, tempo or
    /// meter changed and the song index is rebuilt.
    pub fn set_project(&mut self, project: &Project) -> Result<(), MixError> {
        if let Some(plan) = self.compiler.update(project)? {
            self.pending_plan = Some(Box::new(plan));
        }
        if !self.song_inputs.same_as(project) {
            self.song_inputs = SongInputs::of(project);
            let song = SongIndex::build(project);
            if song != self.song {
                self.song = song.clone();
                self.pending_song = Some(Box::new(song));
            }
        }
        self.flush();
        Ok(())
    }

    /// Resolves sample paths through `roots` from the next [`Engine::set_project`] on, for example after the project
    /// was saved to another folder. Samples that were missing are looked for again.
    pub fn set_sample_roots(&mut self, roots: SampleRoots) {
        self.compiler.set_roots(roots);
    }

    /// Sends anything still waiting and drops whatever the audio side retired. Call it regularly.
    pub fn flush(&mut self) {
        self.plans.collect();
        self.songs.collect();
        if let Some(plan) = self.pending_plan.take()
            && let Err(plan) = self.plans.send(*plan)
        {
            self.pending_plan = Some(plan);
        }
        if let Some(song) = self.pending_song.take()
            && let Err(full) = self.songs.send(song)
        {
            self.pending_song = Some(full.0);
        }
    }

    /// Starts playing.
    pub fn play(&mut self) {
        self.command(Command::Play);
    }

    /// Starts playing to record: live notes are captured until [`Engine::stop`], and with count-in on a bar of clicks
    /// plays first while the playhead waits. Collect the take with [`Engine::take_midi_recording`].
    pub fn record(&mut self) {
        self.command(Command::Record);
    }

    /// Stops playing, ends a recording and releases every note, live ones included.
    pub fn stop(&mut self) {
        self.command(Command::Stop);
    }

    /// Turns the metronome on or off; it clicks while the transport plays.
    pub fn set_metronome(&self, on: bool) {
        self.shared.metronome.store(on, Ordering::Relaxed);
    }

    /// Turns count-in before recording on or off.
    pub fn set_count_in(&self, on: bool) {
        self.shared.count_in.store(on, Ordering::Relaxed);
    }

    /// Where notes played live go: from MIDI inputs, the computer keyboard, or anything else. Clones feed this engine.
    pub fn live_input(&self) -> LiveInput {
        self.live.clone()
    }

    /// The MIDI take of the last recording, once the audio side has finished it; `None` until then. Turn it into notes
    /// with [`crate::midi_record::place`].
    ///
    /// Complexity: O(e) in events captured since the last call.
    pub fn take_midi_recording(&mut self) -> Option<CapturedTake> {
        while let Some(capture) = self.captures.pop() {
            match capture {
                Capture::Start { frame } => {
                    self.capture_start = Some(frame);
                    self.captured.clear();
                }
                Capture::Note(note) => self.captured.push(note),
                Capture::End { frame } => {
                    let start = self.capture_start.take().unwrap_or(frame);
                    return Some(CapturedTake { start, end: frame.max(start), notes: std::mem::take(&mut self.captured) });
                }
            }
        }
        None
    }

    /// Moves the playhead to `tick`.
    pub fn seek(&mut self, tick: Tick) {
        let frame = frame_at(&self.song, tick, self.rate).0;
        self.command(Command::Seek(frame));
    }

    /// Plays the playlist, or loops one pattern.
    pub fn set_mode(&mut self, mode: PlayMode) {
        self.command(Command::Mode(mode));
    }

    /// Puts the RT workers in the device's thread group, from `AudioStream::thread_group`, so the OS schedules
    /// them with the audio thread. Call it after opening a device, and with `None` when switching to one without a group.
    pub fn set_thread_group(&self, group: Option<Arc<dyn ThreadGroup>>) {
        self.thread_group.set(group);
    }

    fn command(&mut self, command: Command) {
        if self.commands.push(command).is_err() {
            tracing::warn!(?command, "the audio thread isn't taking transport commands");
        }
    }

    /// The engine's state as of the audio side's last block.
    pub fn status(&self) -> EngineStatus {
        EngineStatus {
            playing: self.shared.playing.load(Ordering::Relaxed),
            frame: self.shared.frame.load(Ordering::Relaxed),
            dropped_events: self.shared.dropped_events.load(Ordering::Relaxed),
            counting_in: self.shared.counting_in.load(Ordering::Relaxed),
            recording: self.shared.recording.load(Ordering::Relaxed),
        }
    }

    /// What delay compensation decided for the current graph.
    pub fn latency(&self) -> &LatencyReport {
        self.compiler.latency()
    }
}

impl AudioEngine {
    /// Renders `frames` frames into `output`, stereo, in pieces of at most the configured block size.
    ///
    /// RT-safe. Complexity: per block, O(V + E) for the graph plus the sequencer's queries and the processors' work.
    #[rt_safe]
    pub fn render(&mut self, frames: usize, output: &mut AudioBuffer) {
        output.set_frames(frames);
        let mut done = 0;
        while done < frames {
            let len = (frames - done).min(self.max_frames);
            let rendered = self.render_block(len);
            if rendered == 0 {
                break;
            }
            let copied = self.runner.read_output(|master| {
                for (channel, out) in output.channels_mut().enumerate() {
                    let source = master.channel(channel.min(master.channels().saturating_sub(1)));
                    let n = rendered.min(source.len()).min(out.len().saturating_sub(done));
                    out[done..done + n].copy_from_slice(&source[..n]);
                }
            });
            if copied.is_none() {
                for out in output.channels_mut() {
                    out[done..done + rendered].fill(0.0);
                }
            }
            match self.block {
                BlockKind::Song(first) if self.shared.metronome.load(Ordering::Relaxed) => {
                    self.metronome.song(&self.song, self.rate, first, rendered, output, done);
                }
                BlockKind::CountIn(elapsed) => self.metronome.count_in(elapsed, self.count_in_beat, rendered, output, done),
                _ => {}
            }
            done += rendered;
        }
        for out in output.channels_mut() {
            out[done..].fill(0.0);
        }
    }

    /// Runs one block of at most `max_frames` and advances the transport. A block never spans the end of a count-in.
    fn render_block(&mut self, frames: usize) -> usize {
        let mut jumped = false;
        while let Some(command) = self.commands.pop() {
            match command {
                Command::Play => self.transport.playing = true,
                Command::Record => self.start_recording(),
                Command::Stop => {
                    self.transport.playing = false;
                    self.count_in_left = 0;
                    self.end_recording();
                    jumped = true;
                }
                Command::Seek(frame) => {
                    self.transport.frame = frame;
                    jumped = true;
                }
                Command::Mode(mode) => {
                    self.transport.mode = mode;
                    jumped = true;
                }
            }
        }
        self.songs.update(&mut self.song);
        let counting = self.transport.playing && self.count_in_left > 0;
        let frames = if counting { frames.min(usize::try_from(self.count_in_left).unwrap_or(frames)) } else { frames };
        let playing = self.transport.playing && !counting;
        if playing && self.capturing && !self.capture_started && self.captures.push(Capture::Start { frame: self.transport.frame }).is_ok() {
            self.capture_started = true;
        }
        self.take_live(playing);
        if jumped {
            self.metronome.silence();
        }

        let (song, transport, rate) = (&*self.song, self.transport, self.rate);
        let (live, live_channels) = (&self.live_events, &self.live_channels);
        let mut dropped = 0;
        let rendered = self.runner.process(frames, |context| {
            context.clear();
            context.frames = frames;
            context.playing = playing;
            if jumped {
                all_notes_off(song, context);
                for &channel in live_channels {
                    context.push(channel, NoteEvent { frame: 0, kind: EventKind::AllOff, key: 0, velocity: 0.0, voice: 0 });
                }
            }
            if playing {
                emit(song, &transport, frames, rate, context);
            }
            for &(channel, event) in live {
                context.push(channel, event);
            }
            context.finish();
            dropped = context.dropped_events;
        });
        if jumped {
            self.live_channels.clear();
        }
        for &(channel, event) in &self.live_events {
            if event.kind == EventKind::On && !self.live_channels.contains(&channel) && self.live_channels.len() < self.live_channels.capacity() {
                self.live_channels.push(channel);
            }
        }

        self.block = if counting {
            BlockKind::CountIn(self.count_in_total - self.count_in_left)
        } else if playing {
            BlockKind::Song(self.transport.frame)
        } else {
            BlockKind::Still
        };
        if counting {
            self.count_in_left -= rendered as i64;
        } else if playing {
            self.transport.frame += rendered as i64;
        }
        self.shared.playing.store(self.transport.playing, Ordering::Relaxed);
        self.shared.frame.store(self.transport.frame, Ordering::Relaxed);
        self.shared.dropped_events.store(dropped, Ordering::Relaxed);
        self.shared.counting_in.store(self.count_in_left > 0 && self.transport.playing, Ordering::Relaxed);
        self.shared.recording.store(self.capturing, Ordering::Relaxed);
        rendered
    }

    /// Starts playing to record, with a count-in when it is on.
    fn start_recording(&mut self) {
        self.transport.playing = true;
        self.capturing = true;
        self.capture_started = false;
        if self.shared.count_in.load(Ordering::Relaxed) {
            let seconds = bodo_kit_core::Seconds(self.transport.frame as f64 / f64::from(self.rate.hz()));
            let tick = Tick(self.song.tempo().ticks_at(seconds).round() as i64);
            let (beat, beats) = count_in_beats(&self.song, self.rate, tick);
            self.count_in_beat = beat.max(1);
            self.count_in_total = self.count_in_beat * i64::from(beats.max(1));
            self.count_in_left = self.count_in_total;
        }
    }

    /// Ends a recording, marking the end of the take for the control side.
    fn end_recording(&mut self) {
        if self.capturing {
            let frame = self.transport.frame;
            if !self.capture_started {
                let _ = self.captures.push(Capture::Start { frame });
            }
            let _ = self.captures.push(Capture::End { frame });
        }
        self.capturing = false;
        self.capture_started = false;
    }

    /// Takes the notes played live since the last block as this block's events, and captures them while recording.
    fn take_live(&mut self, playing: bool) {
        self.live_events.clear();
        while self.live_events.len() < self.live_events.capacity() {
            let Some(event) = self.live.pop() else { break };
            let (channel, event) = match event {
                LiveEvent::Note { channel, key, velocity, on } => {
                    if playing && self.capturing {
                        let note = CapturedNote { channel, key, velocity, on, frame: self.transport.frame };
                        let _ = self.captures.push(Capture::Note(note));
                    }
                    let kind = if on { EventKind::On } else { EventKind::Off };
                    (channel, NoteEvent { frame: 0, kind, key, velocity, voice: LIVE_VOICE | u64::from(key) })
                }
                LiveEvent::AllOff { channel } => (channel, NoteEvent { frame: 0, kind: EventKind::AllOff, key: 0, velocity: 0.0, voice: 0 }),
            };
            self.live_events.push((channel, event));
        }
    }

    /// How many internal faults the graph has seen; zero unless something is badly wrong.
    pub fn faults(&self) -> u64 {
        self.runner.faults()
    }
}

impl AudioCallback for AudioEngine {
    /// Renders the device's block.
    ///
    /// RT-safe.
    #[rt_safe]
    fn process(&mut self, block: &mut Block<'_>) {
        self.render(block.frames, block.output);
    }
}
