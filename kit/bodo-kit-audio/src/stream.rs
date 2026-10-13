//! Streaming long files from disk without blocking the audio thread (project.md section 6.7).
//!
//! A [`DiskThread`] decodes ahead for every open stream. Each stream owns a fixed set of chunk buffers, allocated
//! when it opens, that circulate between two bounded queues: the disk thread fills empty chunks and queues them, and
//! the [`StreamReader`] on the audio thread copies frames out and hands the chunks back. Nothing is allocated or
//! freed while streaming, and the reader never waits: when the data isn't there yet it plays silence and counts an
//! underrun.
//!
//! Seeking bumps a generation number. The disk thread notices within one poll interval, seeks its decoder and fills
//! new chunks; the reader drops any chunk from an older generation.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use bodo_kit_core::queue::{Consumer, Producer, spsc};
use bodo_kit_core::{SampleRate, rt_safe};

use crate::AudioError;
use crate::decode::{AudioInfo, Decoder};

/// How long the disk thread sleeps when every stream is full.
const POLL: Duration = Duration::from_millis(5);

/// Buffering settings for a stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamConfig {
    /// Frames per chunk.
    pub chunk_frames: usize,
    /// Chunks per stream. Together with `chunk_frames` this sets how far ahead the disk thread reads.
    pub chunks: usize,
}

impl Default for StreamConfig {
    /// 2048-frame chunks, 48 of them: about two seconds at 48 kHz.
    fn default() -> Self {
        Self { chunk_frames: 2048, chunks: 48 }
    }
}

/// A buffer of interleaved frames travelling between the disk thread and a reader.
struct Chunk {
    data: Box<[f32]>,
    frames: usize,
    generation: u32,
    /// The last chunk before the end of the file.
    end: bool,
}

/// What a reader and the disk thread share.
#[derive(Debug, Default)]
struct Shared {
    seek_target: AtomicU64,
    seek_generation: AtomicU32,
    underruns: AtomicU64,
    closed: AtomicBool,
}

/// The audio-thread side of a stream.
pub struct StreamReader {
    filled: Consumer<Chunk>,
    empty: Producer<Chunk>,
    current: Option<Chunk>,
    offset: usize,
    generation: u32,
    position: u64,
    at_end: bool,
    info: AudioInfo,
    shared: Arc<Shared>,
}

impl std::fmt::Debug for StreamReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamReader").field("info", &self.info).field("position", &self.position).finish_non_exhaustive()
    }
}

impl StreamReader {
    /// The file's rate, channels and length.
    pub fn info(&self) -> AudioInfo {
        self.info
    }

    /// The sample rate.
    pub fn rate(&self) -> SampleRate {
        self.info.rate
    }

    /// The frame the next read starts at.
    pub fn position(&self) -> u64 {
        self.position
    }

    /// Whether the reader has passed the end of the file.
    pub fn at_end(&self) -> bool {
        self.at_end
    }

    /// How often a read found no data ready, since the stream opened.
    pub fn underruns(&self) -> u64 {
        self.shared.underruns.load(Ordering::Relaxed)
    }

    /// Whether at least one chunk is decoded and waiting.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn is_ready(&self) -> bool {
        self.current.is_some() || !self.filled.is_empty()
    }

    /// Fills `out` with the next frames, one slice per channel, all the same length. Channels the file doesn't have
    /// are left silent, and so is anything the disk thread hasn't decoded yet or that lies past the end. Returns the
    /// frames that came from the file.
    ///
    /// RT-safe. Complexity: O(channels * frames).
    #[rt_safe]
    pub fn read(&mut self, out: &mut [&mut [f32]]) -> usize {
        let frames = out.iter().map(|c| c.len()).min().unwrap_or(0);
        let channels = self.info.channels;
        let mut written = 0;
        while written < frames && !self.at_end {
            if self.current.is_none() {
                match self.next_chunk() {
                    Some(chunk) => {
                        self.current = Some(chunk);
                        self.offset = 0;
                    }
                    None => {
                        self.shared.underruns.fetch_add(1, Ordering::Relaxed);
                        break;
                    }
                }
            }
            let Some(chunk) = self.current.as_ref() else { break };
            let take = (chunk.frames - self.offset).min(frames - written);
            for (index, channel) in out.iter_mut().enumerate() {
                if index >= channels {
                    continue;
                }
                for frame in 0..take {
                    channel[written + frame] = chunk.data.get((self.offset + frame) * channels + index).copied().unwrap_or(0.0);
                }
            }
            for channel in out.iter_mut().skip(channels) {
                channel[written..written + take].fill(0.0);
            }
            written += take;
            self.offset += take;
            if self.offset >= chunk.frames {
                let end = chunk.end;
                self.recycle();
                if end {
                    self.at_end = true;
                }
            }
        }
        for channel in out.iter_mut() {
            channel[written..frames].fill(0.0);
        }
        self.position += written as u64;
        written
    }

    /// Moves to `frame`. The disk thread catches up within a few milliseconds; reads play silence until it does.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn seek(&mut self, frame: u64) {
        self.recycle();
        self.generation = self.generation.wrapping_add(1);
        self.shared.seek_target.store(frame, Ordering::Relaxed);
        self.shared.seek_generation.store(self.generation, Ordering::Release);
        self.position = frame;
        self.at_end = false;
    }

    /// The next chunk of the current generation, returning stale ones on the way.
    fn next_chunk(&mut self) -> Option<Chunk> {
        while let Some(chunk) = self.filled.pop() {
            if chunk.generation == self.generation {
                return Some(chunk);
            }
            self.give_back(chunk);
        }
        None
    }

    fn recycle(&mut self) {
        if let Some(chunk) = self.current.take() {
            self.give_back(chunk);
        }
    }

    fn give_back(&mut self, chunk: Chunk) {
        // Every chunk of the stream fits in the queue, so this always succeeds.
        if let Err(full) = self.empty.push(chunk) {
            core::mem::forget(full);
        }
    }
}

impl Drop for StreamReader {
    fn drop(&mut self) {
        self.shared.closed.store(true, Ordering::Release);
    }
}

/// The disk-thread side of a stream.
struct StreamTask {
    decoder: Decoder,
    filled: Producer<Chunk>,
    empty: Consumer<Chunk>,
    pending: Vec<Vec<f32>>,
    generation: u32,
    finished: bool,
    shared: Arc<Shared>,
    path: PathBuf,
}

impl StreamTask {
    /// Fills up to `budget` chunks. Returns whether it did any work.
    fn service(&mut self, budget: usize) -> bool {
        let seek = self.shared.seek_generation.load(Ordering::Acquire);
        let mut busy = false;
        if seek != self.generation {
            self.generation = seek;
            for channel in &mut self.pending {
                channel.clear();
            }
            let target = self.shared.seek_target.load(Ordering::Relaxed);
            self.finished = match self.decoder.seek(target) {
                Ok(_) => false,
                Err(err) => {
                    tracing::warn!(path = %self.path.display(), %err, "seek failed");
                    true
                }
            };
            busy = true;
        }
        for _ in 0..budget {
            if self.finished {
                break;
            }
            let Some(mut chunk) = self.empty.pop() else { break };
            self.fill(&mut chunk);
            if let Err(full) = self.filled.push(chunk) {
                core::mem::forget(full);
            }
            busy = true;
        }
        busy
    }

    fn fill(&mut self, chunk: &mut Chunk) {
        let channels = self.pending.len();
        let capacity = chunk.data.len() / channels.max(1);
        while self.pending.first().map_or(0, Vec::len) < capacity {
            match self.decoder.decode_next(&mut self.pending) {
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(err) => {
                    tracing::warn!(path = %self.path.display(), %err, "decoding failed; ending the stream");
                    break;
                }
            }
        }
        let frames = self.pending.first().map_or(0, Vec::len).min(capacity);
        for (index, channel) in self.pending.iter_mut().enumerate() {
            for (frame, sample) in channel.drain(..frames).enumerate() {
                if let Some(slot) = chunk.data.get_mut(frame * channels + index) {
                    *slot = sample;
                }
            }
        }
        chunk.frames = frames;
        chunk.generation = self.generation;
        chunk.end = frames < capacity;
        if chunk.end {
            self.finished = true;
        }
    }
}

enum Command {
    Open(Box<StreamTask>),
    Stop,
}

/// The thread that decodes ahead for every open stream.
#[derive(Debug)]
pub struct DiskThread {
    commands: mpsc::Sender<Command>,
    handle: Option<JoinHandle<()>>,
}

impl DiskThread {
    /// Starts the thread.
    pub fn spawn() -> Result<Self, AudioError> {
        let (commands, receiver) = mpsc::channel();
        let handle = thread::Builder::new()
            .name(String::from("bodo-disk"))
            .spawn(move || run(&receiver))
            .map_err(|err| AudioError::Io { path: PathBuf::new(), source: err })?;
        Ok(Self { commands, handle: Some(handle) })
    }

    /// Opens `path` for streaming and returns its reader. The disk thread starts filling it at once.
    ///
    /// Allocates and does I/O. Complexity: O(chunks * chunk_frames) for the buffers.
    pub fn open(&self, path: &Path, config: StreamConfig) -> Result<StreamReader, AudioError> {
        let decoder = Decoder::open(path)?;
        let info = decoder.info();
        let chunks = config.chunks.max(2);
        let (filled_tx, filled_rx) = spsc(chunks);
        let (mut empty_tx, empty_rx) = spsc(chunks);
        for _ in 0..chunks {
            let chunk =
                Chunk { data: vec![0.0; config.chunk_frames.max(1) * info.channels].into_boxed_slice(), frames: 0, generation: 0, end: false };
            if empty_tx.push(chunk).is_err() {
                return Err(AudioError::Unsupported(String::from("stream queue too small")));
            }
        }
        let shared = Arc::new(Shared::default());
        let task = StreamTask {
            decoder,
            filled: filled_tx,
            empty: empty_rx,
            pending: (0..info.channels).map(|_| Vec::with_capacity(config.chunk_frames * 2)).collect(),
            generation: 0,
            finished: false,
            shared: Arc::clone(&shared),
            path: path.to_path_buf(),
        };
        self.commands.send(Command::Open(Box::new(task))).map_err(|_| AudioError::Unsupported(String::from("the disk thread has stopped")))?;
        Ok(StreamReader { filled: filled_rx, empty: empty_tx, current: None, offset: 0, generation: 0, position: 0, at_end: false, info, shared })
    }
}

impl Drop for DiskThread {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Stop);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn run(commands: &mpsc::Receiver<Command>) {
    let mut tasks: Vec<Box<StreamTask>> = Vec::new();
    loop {
        loop {
            match commands.try_recv() {
                Ok(Command::Open(task)) => tasks.push(task),
                Ok(Command::Stop) | Err(mpsc::TryRecvError::Disconnected) => return,
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }
        tasks.retain(|task| !task.shared.closed.load(Ordering::Acquire));
        let mut busy = false;
        for task in &mut tasks {
            busy |= task.service(4);
        }
        if !busy {
            match commands.recv_timeout(POLL) {
                Ok(Command::Open(task)) => tasks.push(task),
                Ok(Command::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }
}
