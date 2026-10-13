//! The engine: [`Engine`] on the control thread, [`AudioEngine`] on the audio thread (project.md section 6.4).
//!
//! The control side compiles plans and song indexes and sends them, with transport commands, through bounded queues.
//! The audio side switches to whatever arrived at the start of a block, runs the sequencer and the graph, and copies
//! the master into the device's output. Neither side ever waits for the other.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};

use bodo_kit_core::handoff::{self, Receiver, Sender};
use bodo_kit_core::pool::{RtPoolConfig, RtPriority};
use bodo_kit_core::queue::{Consumer, Producer, spsc};
use bodo_kit_core::{MAX_BLOCK_FRAMES, SampleRate, Tick, rt_safe};
use bodo_kit_device::{AudioCallback, Block};
use bodo_kit_dsp::AudioBuffer;
use bodo_kit_graph::{GraphRunner, LatencyReport, Plan, PlanSender, runner};
use bodo_model::Project;

use crate::MixError;
use crate::compile::Compiler;
use crate::context::BlockContext;
use crate::nodes::MixNode;
use crate::sequence::{PlayMode, SongIndex, Transport, all_notes_off, emit, frame_at};

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
    rate: SampleRate,
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
}

impl std::fmt::Debug for AudioEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioEngine").field("transport", &self.transport).finish_non_exhaustive()
    }
}

/// Builds the two sides of an engine for `project`, whose sample paths are relative to `base`. The engine renders at
/// the project's sample rate.
///
/// Compiles the first plan, loads samples and spawns the workers.
pub fn engine(project: &Project, base: &Path, config: EngineConfig) -> Result<(Engine, AudioEngine), MixError> {
    let rate = project.settings.sample_rate;
    let max_frames = config.max_frames.clamp(1, MAX_BLOCK_FRAMES);
    let mut compiler = Compiler::new(base.to_path_buf(), max_frames, rate);
    let plan = compiler.update(project)?.unwrap_or_else(|| Plan::empty(max_frames));
    let pool = RtPoolConfig {
        priority: config.realtime_buffer.map(|buffer_frames| RtPriority { buffer_frames, sample_rate: rate }),
        ..RtPoolConfig::new(config.workers)
    };
    let (graph, plans) = runner(plan, BlockContext::new(rate), pool)?;
    let song = SongIndex::build(project);
    let (songs_tx, songs_rx) = handoff::handoff(4);
    let (commands_tx, commands_rx) = spsc(64);
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
        rate,
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
    };
    Ok((control, audio))
}

impl Engine {
    /// Brings the engine up to date with `project`: live parameters at once, and a new plan and song index when
    /// they changed. The audio side switches over at its next block.
    ///
    /// Allocates. Complexity: O(V + E) plus O(N log N) in notes and clips for the song index.
    pub fn set_project(&mut self, project: &Project) -> Result<(), MixError> {
        if let Some(plan) = self.compiler.update(project)? {
            self.pending_plan = Some(Box::new(plan));
        }
        let song = SongIndex::build(project);
        if song != self.song {
            self.song = song.clone();
            self.pending_song = Some(Box::new(song));
        }
        self.flush();
        Ok(())
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

    /// Stops playing and releases every note.
    pub fn stop(&mut self) {
        self.command(Command::Stop);
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
            done += rendered;
        }
        for out in output.channels_mut() {
            out[done..].fill(0.0);
        }
    }

    /// Runs one block of at most `max_frames` and advances the transport.
    fn render_block(&mut self, frames: usize) -> usize {
        let mut jumped = false;
        while let Some(command) = self.commands.pop() {
            match command {
                Command::Play => self.transport.playing = true,
                Command::Stop => {
                    self.transport.playing = false;
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
        let (song, transport, rate) = (&*self.song, self.transport, self.rate);
        let mut dropped = 0;
        let rendered = self.runner.process(frames, |context| {
            context.clear();
            context.frames = frames;
            context.playing = transport.playing;
            if jumped {
                all_notes_off(song, context);
            }
            if transport.playing {
                emit(song, &transport, frames, rate, context);
            }
            context.finish();
            dropped = context.dropped_events;
        });
        if self.transport.playing {
            self.transport.frame += rendered as i64;
        }
        self.shared.playing.store(self.transport.playing, Ordering::Relaxed);
        self.shared.frame.store(self.transport.frame, Ordering::Relaxed);
        self.shared.dropped_events.store(dropped, Ordering::Relaxed);
        rendered
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
