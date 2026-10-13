//! The engine: [`Engine`] on the control thread, [`AudioEngine`] on the audio thread (project.md section 6.4).
//!
//! The control side compiles plans and song indexes and sends them, with transport commands, through bounded queues.
//! The audio side switches to whatever arrived at the start of a block, takes in notes played live, runs the
//! sequencer and the graph, copies the master into the device's output, adds direct hardware outputs and the
//! metronome, and feeds the scope. Neither side ever waits for the other.
//!
//! The control side also reads what the interface shows: every insert's meter ([`Engine::meter`]), its share of the
//! CPU ([`Engine::insert_cpu`]), its latency ([`Engine::insert_latency`]) and the master's last moments
//! ([`Engine::scope`]).

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::time::Instant;

use bodo_kit_core::TempoMap;
use bodo_kit_core::handoff::{self, Receiver, Sender};
use bodo_kit_core::pool::{RtPoolConfig, RtPriority, ThreadGroup, ThreadGroupSlot};
use bodo_kit_core::queue::{Consumer, Producer, spsc};
use bodo_kit_core::{MAX_BLOCK_FRAMES, SampleRate, Tick, rt_safe};
use bodo_kit_device::{AudioCallback, Block};
use bodo_kit_dsp::AudioBuffer;
use bodo_kit_graph::{GraphRunner, LatencyReport, Plan, PlanSender, runner};
use bodo_model::meter::MeterMap;
use bodo_model::{InsertId, Pattern, PatternId, PlaylistTrack, Project, SampleRoots, TrackId};
use imbl::{OrdMap, Vector};

use crate::compile::{AutomationTargets, Compiler, StemSource, hardware_channels};
use crate::context::{BlockContext, EventKind, NoteEvent};
use crate::input::{LIVE_CAPACITY, LiveEvent, LiveInput};
use crate::meter::{MeterReading, Scope};
use crate::metronome::{Metronome, count_in_beats};
use crate::midi_record::{CAPTURE_CAPACITY, Capture, CapturedNote, CapturedTake};
use crate::nodes::MixNode;
use crate::record::{Compensation, Recorder, Take};
use crate::sequence::{PlayMode, SongIndex, Transport, all_notes_off, emit, frame_at};
use crate::{MixError, automation, keys};

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
    /// Promote the workers to real-time priority for this device buffer size, in frames. A real-time engine also lets
    /// samplers that keep their sample on disk stream it; without one, as in offline renders, every sample loads.
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
    /// Loop Song mode between two ticks, or stop looping.
    Loop(Option<(i64, i64)>),
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
    loop_record: AtomicBool,
}

/// An insert's latency, as delay compensation decided it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InsertLatency {
    /// How late its input is, in frames, so it lines up with the latest signal reaching it.
    pub input: u32,
    /// The latency its own slots report, plus a positive track latency.
    pub own: u32,
    /// How late its output is: its input latency plus its own.
    pub output: u32,
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
    /// How late the master's output is, in frames, as delay compensation decided it for the running plan.
    pub latency: u32,
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
    /// Where the Song loop wrapped in the take being collected: before which note, at which frame.
    capture_wraps: Vec<(usize, i64)>,
    recorder: Recorder,
    targets: Sender<Box<AutomationTargets>>,
    pending_targets: Option<Box<AutomationTargets>>,
    scope: Arc<Scope>,
    /// Each insert's processing time and the time it was read, for [`Engine::insert_cpu`].
    cpu: HashMap<InsertId, (u64, Instant)>,
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
    /// What the song index reads from channels: each channel's swing and automation lane.
    channels: Vec<(u64, u32, usize)>,
}

/// What the song index reads from each channel: its key, its swing, and its automation lane by address.
///
/// Complexity: O(c) in channels.
fn channel_inputs(project: &Project) -> impl Iterator<Item = (u64, u32, usize)> + '_ {
    project.channels.values().map(|channel| {
        let lane = match &channel.kind {
            bodo_model::channel::ChannelKind::Automation(lane) => Arc::as_ptr(lane) as usize,
            // A layer's children decide where its notes play.
            bodo_model::channel::ChannelKind::Layer { children } => {
                children.iter().fold(0x9E37_79B9usize, |hash, child| hash.rotate_left(7) ^ keys::channel(*child) as usize) | 1
            }
            _ => 0,
        };
        (keys::channel(channel.id), channel.swing.to_bits(), lane)
    })
}

impl SongInputs {
    fn of(project: &Project) -> Self {
        Self {
            tempo: Arc::clone(&project.tempo),
            meter: Arc::clone(&project.meter),
            patterns: project.patterns.clone(),
            tracks: project.tracks.clone(),
            track_order: project.track_order.clone(),
            channels: channel_inputs(project).collect(),
        }
    }

    fn same_as(&self, project: &Project) -> bool {
        Arc::ptr_eq(&self.tempo, &project.tempo)
            && Arc::ptr_eq(&self.meter, &project.meter)
            && self.patterns.ptr_eq(&project.patterns)
            && self.tracks.ptr_eq(&project.tracks)
            // imbl keeps tiny vectors inline, where pointers never match; comparing those by value is cheap.
            && (self.track_order.ptr_eq(&project.track_order) || self.track_order == project.track_order)
            && self.channels.iter().copied().eq(channel_inputs(project))
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
    /// The Song mode loop, start and end in ticks.
    loop_region: Option<(i64, i64)>,
    /// The playhead jumped back to the loop's start at the end of the last block.
    wrapped: bool,
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
    /// Automation targets of the running plan.
    targets: Box<AutomationTargets>,
    target_updates: Receiver<Box<AutomationTargets>>,
    scope: Arc<Scope>,
    /// While loop recording in Pattern mode: where the pattern pass that recording began in starts, which every later
    /// pass's take starts at too.
    pass_origin: Option<i64>,
    /// The next block starts a new loop pass whose takes start at this frame.
    pass_start: Option<i64>,
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
    build(project, roots, config, false)
}

/// Builds an engine, tapping every signal entering the master when `stems` is set.
pub(crate) fn build(project: &Project, roots: &SampleRoots, config: EngineConfig, stems: bool) -> Result<(Engine, AudioEngine), MixError> {
    let rate = project.settings.sample_rate;
    let max_frames = config.max_frames.clamp(1, MAX_BLOCK_FRAMES);
    let mut compiler = Compiler::new(roots.clone(), max_frames, rate, stems);
    compiler.set_streaming(config.realtime_buffer.is_some());
    let plan = compiler.update(project)?.unwrap_or_else(|| Plan::empty(max_frames));
    let targets = compiler.take_targets().unwrap_or_default();
    let (targets_tx, targets_rx) = handoff::handoff(4);
    let scope = Arc::new(Scope::default());
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
        capture_wraps: Vec::new(),
        recorder: Recorder::default(),
        targets: targets_tx,
        pending_targets: None,
        scope: Arc::clone(&scope),
        cpu: HashMap::new(),
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
        loop_region: None,
        wrapped: false,
        targets,
        target_updates: targets_rx,
        scope,
        pass_origin: None,
        pass_start: None,
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
        if let Some(targets) = self.compiler.take_targets() {
            self.pending_targets = Some(targets);
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

    /// Sends anything still waiting and drops whatever the audio side retired. Once the edges and nodes an edit removed
    /// have faded out, it compiles the plan without them. Call it regularly.
    pub fn flush(&mut self) {
        self.plans.collect();
        self.songs.collect();
        self.targets.collect();
        self.compiler.captures_mut().collect();
        self.compiler.collect();
        if self.pending_plan.is_none() {
            match self.compiler.expire_ghosts(Instant::now()) {
                Ok(Some(plan)) => self.pending_plan = Some(Box::new(plan)),
                Ok(None) => {}
                Err(error) => tracing::warn!(%error, "could not drop the faded-out parts of the mix graph"),
            }
        }
        if let Some(targets) = self.compiler.take_targets() {
            self.pending_targets = Some(targets);
        }
        if let Some(plan) = self.pending_plan.take()
            && let Err(plan) = self.plans.send(*plan)
        {
            self.pending_plan = Some(plan);
        }
        if self.pending_plan.is_none()
            && let Some(targets) = self.pending_targets.take()
            && let Err(full) = self.targets.send(targets)
        {
            self.pending_targets = Some(full.0);
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
        self.set_recording(true);
        self.command(Command::Record);
    }

    /// Stops playing, ends a recording and releases every note, live ones included.
    pub fn stop(&mut self) {
        self.command(Command::Stop);
        self.set_recording(false);
    }

    /// Recompiles with delay compensation bypassed, or not, for inserts that bypass it while recording.
    fn set_recording(&mut self, recording: bool) {
        match self.compiler.set_recording(recording) {
            Ok(Some(plan)) => {
                self.pending_plan = Some(Box::new(plan));
                self.flush();
            }
            Ok(None) => {}
            Err(error) => tracing::warn!(%error, "could not switch delay compensation for recording"),
        }
    }

    /// Turns loop recording on or off: while it is on and the transport loops (the Song loop, or the pattern in
    /// Pattern mode), every pass of a recording becomes its own take, and MIDI notes of every pass add up.
    pub fn set_loop_record(&self, on: bool) {
        self.shared.loop_record.store(on, Ordering::Relaxed);
    }

    /// What the meter of `insert` read since the last call, which clears its peaks; `None` for an insert the engine
    /// doesn't know yet.
    pub fn meter(&self, insert: InsertId) -> Option<MeterReading> {
        self.compiler.strips().get(&insert).map(|live| live.meter.read())
    }

    /// Clears the clip latch of `insert`'s meter.
    pub fn clear_clip(&self, insert: InsertId) {
        if let Some(live) = self.compiler.strips().get(&insert) {
            live.meter.clear_clip();
        }
    }

    /// Clears the clip latch of every meter.
    ///
    /// Complexity: O(i) in inserts.
    pub fn clear_clips(&self) {
        for live in self.compiler.strips().values() {
            live.meter.clear_clip();
        }
    }

    /// The share of real time `insert`'s three stages spent processing since the last call for it: 0.01 is 1%.
    /// The first call for an insert, and the first after a recompile, returns 0.
    pub fn insert_cpu(&mut self, insert: InsertId) -> f32 {
        let Some(times) = self.compiler.times() else { return 0.0 };
        let nanos: u64 = [keys::insert_input(insert), keys::insert_fx(insert), keys::insert(insert)].iter().filter_map(|&key| times.nanos(key)).sum();
        let now = Instant::now();
        let load = match self.cpu.insert(insert, (nanos, now)) {
            Some((before, then)) if nanos >= before => {
                let wall = now.duration_since(then).as_nanos() as f64;
                if wall > 0.0 { ((nanos - before) as f64 / wall) as f32 } else { 0.0 }
            }
            _ => 0.0,
        };
        load.max(0.0)
    }

    /// Copies the latest `out.len()` frames of the master output, mixed to mono, into `out`, for an oscilloscope.
    ///
    /// Complexity: O(n) in `out.len()`.
    pub fn scope(&self, out: &mut [f32]) {
        self.scope.read(out);
    }

    /// The latency of `insert` as delay compensation decided it for the running plan.
    pub fn insert_latency(&self, insert: InsertId) -> Option<InsertLatency> {
        let report = self.compiler.latency();
        let input = report.nodes.get(&keys::insert_input(insert))?.input;
        let output = report.nodes.get(&keys::insert(insert))?.output;
        Some(InsertLatency { input, own: output.saturating_sub(input), output })
    }

    /// The latency of the whole mix: how late the master's output is.
    pub fn total_latency(&self) -> u32 {
        self.compiler.latency().total
    }

    /// The stems of an engine built for stems, by tap index, each with the compensation delay of its way into the
    /// master.
    pub(crate) fn stems(&self) -> Vec<(StemSource, u32)> {
        let report = self.compiler.latency();
        self.compiler.stem_sources().iter().map(|s| (s.clone(), report.edges.get(&s.edge).copied().unwrap_or(0))).collect()
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
                    self.capture_wraps.clear();
                }
                Capture::Note(note) => self.captured.push(note),
                Capture::Wrap { from } => self.capture_wraps.push((self.captured.len(), from)),
                Capture::End { frame } => {
                    let start = self.capture_start.take().unwrap_or(frame);
                    let wraps = std::mem::take(&mut self.capture_wraps);
                    let end = if wraps.is_empty() { frame.max(start) } else { frame };
                    return Some(CapturedTake { start, end, notes: std::mem::take(&mut self.captured), wraps });
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

    /// Loops Song mode from `start` to `end`: when the playhead reaches `end` it continues at `start`, to the frame,
    /// and notes are released as on a jump. `None`, or an empty region, stops looping.
    pub fn set_loop(&mut self, region: Option<(Tick, Tick)>) {
        let region = region.filter(|(start, end)| end > start).map(|(start, end)| (start.0, end.0));
        self.command(Command::Loop(region));
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
            latency: self.total_latency(),
        }
    }

    /// Starts recording every armed insert of `project` into WAV files in `folder`, from the next block the transport
    /// plays. `compensation` is the device's round trip, which moves the takes back on the timeline. Returns how many
    /// inserts record.
    ///
    /// Creates the files and starts the disk thread. Complexity: O(i) in inserts.
    pub fn start_recording(&mut self, project: &Project, folder: &Path, compensation: Compensation) -> Result<usize, MixError> {
        Ok(self.recorder.start(project, folder, compensation, self.compiler.captures_mut())?)
    }

    /// Stops recording; [`Engine::finish_recording`] returns the takes once the audio side has let go and the files
    /// are written.
    pub fn stop_recording(&mut self) {
        self.recorder.stop(self.compiler.captures_mut());
    }

    /// The takes of a stopped recording, once they are written; `None` while recording or finishing. Call it every
    /// frame after [`Engine::stop_recording`]; the audio side must keep running until the takes come back.
    pub fn finish_recording(&mut self) -> Option<Result<Vec<Take>, MixError>> {
        self.recorder.finish(self.compiler.captures_mut()).map(|done| done.map_err(MixError::from))
    }

    /// Whether a recording is running or finishing.
    pub fn is_recording(&self) -> bool {
        self.recorder.is_active()
    }

    /// The MIDI Out channels' messages, for a thread that sends them to their ports.
    pub fn midi_out(&self) -> crate::midi_out::MidiOutTap {
        self.compiler.midi_out()
    }

    /// What delay compensation decided for the current graph.
    pub fn latency(&self) -> &LatencyReport {
        self.compiler.latency()
    }
}

impl AudioEngine {
    /// Renders `frames` frames into `output`, stereo, in pieces of at most the configured block size, with silent
    /// input.
    ///
    /// RT-safe. Complexity: per block, O(V + E) for the graph plus the sequencer's queries and the processors' work.
    #[rt_safe]
    pub fn render(&mut self, frames: usize, output: &mut AudioBuffer) {
        self.render_with_input(frames, None, output);
    }

    /// Renders `frames` frames into `output`, with the device's `input` for inserts that monitor or record it.
    ///
    /// RT-safe. Complexity: per block, O(V + E) for the graph plus the sequencer's queries and the processors' work.
    #[rt_safe]
    pub fn render_with_input(&mut self, frames: usize, input: Option<&AudioBuffer>, output: &mut AudioBuffer) {
        output.set_frames(frames);
        let mut done = 0;
        while done < frames {
            let len = (frames - done).min(self.max_frames);
            let rendered = self.render_block(len, input, done);
            if rendered == 0 {
                break;
            }
            // The master goes to the first two device outputs; any further outputs start silent for direct outs.
            let scope = &self.scope;
            let copied = self.runner.read_output(|master| {
                for (channel, out) in output.channels_mut().enumerate() {
                    let n = rendered.min(out.len().saturating_sub(done));
                    let Some(out) = out.get_mut(done..done + n) else { continue };
                    if channel < 2 {
                        let source = master.channel(channel.min(master.channels().saturating_sub(1)));
                        out.copy_from_slice(source.get(..n).unwrap_or(&[]));
                    } else {
                        out.fill(0.0);
                    }
                }
                if master.channels() >= 2 {
                    let n = rendered.min(master.frames());
                    scope.write(master.channel(0).get(..n).unwrap_or(&[]), master.channel(1).get(..n).unwrap_or(&[]));
                }
            });
            if copied.is_none() {
                for out in output.channels_mut() {
                    if let Some(out) = out.get_mut(done..done + rendered) {
                        out.fill(0.0);
                    }
                }
            }
            self.add_hardware_outputs(output, done, rendered);
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

    /// Adds the inserts with direct hardware outputs into those device outputs, for `frames` frames from `at`.
    ///
    /// RT-safe. Complexity: O(t * frames) in taps.
    fn add_hardware_outputs(&self, output: &mut AudioBuffer, at: usize, frames: usize) {
        for tap in 0..self.runner.tap_count() {
            self.runner.read_tap(tap, |source, tag| {
                let Some((left, right)) = hardware_channels(tag) else { return };
                if source.channels() < 2 {
                    return;
                }
                let (from_l, from_r) = (source.channel(0), source.channel(1));
                let mut add = |channel: usize, f: &dyn Fn(usize) -> f32| {
                    if channel < output.channels() {
                        let out = output.channel_mut(channel);
                        for i in 0..frames.min(from_l.len()) {
                            if let Some(o) = out.get_mut(at + i) {
                                *o += f(i);
                            }
                        }
                    }
                };
                match right {
                    Some(right) => {
                        add(left, &|i| from_l.get(i).copied().unwrap_or(0.0));
                        add(right, &|i| from_r.get(i).copied().unwrap_or(0.0));
                    }
                    None => add(left, &|i| 0.5 * (from_l.get(i).copied().unwrap_or(0.0) + from_r.get(i).copied().unwrap_or(0.0))),
                }
            });
        }
    }

    /// Calls `read` with each stem tap's index and audio from the last block. Only an engine built for stems has them.
    ///
    /// RT-safe as long as `read` is. Complexity: O(t) in taps.
    pub(crate) fn read_stems(&self, mut read: impl FnMut(usize, &AudioBuffer)) {
        for tap in 0..self.runner.tap_count() {
            self.runner.read_tap(tap, |buffer, tag| {
                if hardware_channels(tag).is_none() {
                    read(tag as usize, buffer);
                }
            });
        }
    }

    /// The frames from `frame` to the end of the pattern loop, in Pattern mode while loop recording; `None` otherwise.
    ///
    /// RT-safe. Complexity: O(log m) in tempo points.
    fn pattern_pass(&self) -> Option<(i64, i64)> {
        let PlayMode::Pattern(pattern) = self.transport.mode else { return None };
        if !self.capturing || !self.shared.loop_record.load(Ordering::Relaxed) {
            return None;
        }
        let length = frame_at(&self.song, Tick(self.song.pattern_length(pattern)?), self.rate).0;
        if length <= 0 {
            return None;
        }
        let start = self.transport.frame.div_euclid(length) * length;
        Some((start, start + length))
    }

    /// Runs one block of at most `max_frames`, with `input` from frame `offset` on, and advances the transport. A block
    /// never spans the end of a count-in. In Song mode with a loop, a block ends at the loop's end and the next one
    /// starts at the loop's start; while loop recording in Pattern mode, a block ends where the pattern loops.
    fn render_block(&mut self, frames: usize, input: Option<&AudioBuffer>, offset: usize) -> usize {
        self.target_updates.update(&mut self.targets);
        // A loop wrap at the end of the last block is a jump for this one.
        let mut jumped = std::mem::take(&mut self.wrapped);
        let pass_start = self.pass_start.take();
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
                Command::Loop(region) => self.loop_region = region,
            }
        }
        self.songs.update(&mut self.song);
        let counting = self.transport.playing && self.count_in_left > 0;
        let mut frames = if counting { frames.min(usize::try_from(self.count_in_left).unwrap_or(frames)) } else { frames };
        let mut wrap_to = None;
        if let Some((start, end)) = self.loop_frames().filter(|_| !counting) {
            if self.transport.frame == end {
                self.transport.frame = start;
                jumped = true;
            } else if self.transport.frame < end {
                let left = usize::try_from(end - self.transport.frame).unwrap_or(usize::MAX);
                if left <= frames {
                    frames = left;
                    wrap_to = Some(start);
                }
            }
        }
        // Loop recording in Pattern mode: stop the block where the pattern loops, so the next pass starts its takes.
        let mut pattern_wrap = None;
        if let Some((pass, end)) = self.pattern_pass().filter(|_| self.transport.playing && !counting) {
            let left = usize::try_from(end - self.transport.frame).unwrap_or(usize::MAX);
            if left > 0 && left <= frames {
                frames = left;
                pattern_wrap = Some(*self.pass_origin.get_or_insert(pass));
            }
        }
        let playing = self.transport.playing && !counting;
        if playing && self.capturing && !self.capture_started && self.captures.push(Capture::Start { frame: self.transport.frame }).is_ok() {
            self.capture_started = true;
        }
        self.take_live(playing);
        if jumped {
            self.metronome.silence();
        }

        let (song, transport, rate) = (&*self.song, self.transport, self.rate);
        let (live, live_channels, targets) = (&self.live_events, &self.live_channels, &*self.targets);
        let seconds = bodo_kit_core::Seconds(transport.frame.max(0) as f64 / f64::from(rate.hz()));
        let tempo = song.tempo().bpm_at(Tick(song.tempo().ticks_at(seconds).round() as i64));
        let mut dropped = 0;
        let rendered = self.runner.process(frames, |context| {
            context.clear();
            context.frames = frames;
            context.playing = playing;
            context.frame = transport.frame;
            context.set_position(song.tempo(), transport.frame);
            context.tempo = tempo;
            context.pass_start = pass_start.filter(|_| playing);
            context.set_input(input, offset, frames);
            if jumped {
                // Back to the project's own values until a lane plays again.
                context.params.clear();
                all_notes_off(song, context);
                for &channel in live_channels {
                    context.push(channel, NoteEvent { frame: 0, kind: EventKind::AllOff, key: 0, velocity: 0.0, voice: 0 });
                }
            }
            if playing {
                automation::apply(&song.automation, song.tempo(), &transport, rate, &mut context.params, &targets.delays);
                emit(song, &transport, frames, rate, context);
            }
            // Automated send levels go straight to their edges; the others keep the project's level.
            for (key, gain, base) in &targets.routes {
                gain.set(context.params.get(*key).unwrap_or(*base));
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
            if let Some(start) = wrap_to
                && rendered == frames
            {
                if self.capturing {
                    let _ = self.captures.push(Capture::Wrap { from: self.transport.frame });
                }
                self.transport.frame = start;
                self.wrapped = true;
                if self.capturing && self.shared.loop_record.load(Ordering::Relaxed) {
                    self.pass_start = Some(start);
                }
            }
            if let Some(origin) = pattern_wrap
                && rendered == frames
            {
                self.pass_start = Some(origin);
            }
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
        self.pass_origin = None;
        self.pass_start = None;
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
        self.pass_origin = None;
        self.pass_start = None;
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
            // A layer plays its children.
            for &target in crate::layers::targets(&self.song.layers, channel).keys() {
                if self.live_events.len() < self.live_events.capacity() {
                    self.live_events.push((target, event));
                }
            }
        }
    }

    /// The loop's start and end frames, while Song mode plays a loop.
    ///
    /// RT-safe. Complexity: O(log m) in tempo points.
    fn loop_frames(&self) -> Option<(i64, i64)> {
        let (start, end) = self.loop_region?;
        if !self.transport.playing || self.transport.mode != PlayMode::Song {
            return None;
        }
        let (start, end) = (frame_at(&self.song, Tick(start), self.rate).0, frame_at(&self.song, Tick(end), self.rate).0);
        (end > start).then_some((start, end))
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
        self.render_with_input(block.frames, Some(block.input), block.output);
    }
}
