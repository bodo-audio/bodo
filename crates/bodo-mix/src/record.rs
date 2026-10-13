//! Recording armed inserts to disk (project.md section 9.6).
//!
//! When recording starts, the control side gives each armed insert's strip a [`Capture`]: one end of a lock-free
//! ring holding several seconds of stereo audio. While the transport plays, the strip copies its record tap into the
//! ring on the audio thread; a disk thread drains every ring into a WAV file. A ring that fills up drops the block and
//! counts an overflow; the audio thread never waits. When recording stops, the strips hand their captures back, the
//! disk thread finishes the files, and each insert's file becomes a [`Take`].
//!
//! **Latency compensation.** A performer plays along with what they hear, and what they hear left the engine
//! `output` latency earlier; their sound then takes the `input` latency to reach the engine. So audio that arrives at
//! transport frame `t` belongs on the timeline at `t - (input + output)`, and that is where a take starts
//! ([`Take::timeline_frame`]). The two latencies come from the audio stream ([`Compensation::from_status`]).
//!
//! **Taps.** The strip is three graph nodes, so each tap is recorded by the node that has it: Input and PreFx by the
//! input stage, PostFx by the FX stage, PostFader by the output stage ([`CaptureStage`]).
//!
//! **Formats.** 32-bit float or 24-bit WAV, 24-bit FLAC, or 32-bit float Wave64 ([`RecordFormat`]).
//!
//! **Loop recording.** When the transport wraps while loop recording, the strips mark where the new pass starts in
//! their rings, and the disk thread starts a new file there: every pass becomes its own take, starting where the loop
//! starts.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use bodo_kit_audio::{AudioError, FlacWriter, W64Format, W64Writer, WavFormat, WavWriter};
use bodo_kit_core::handoff::{self, Receiver, Sender};
use bodo_kit_core::queue::{Consumer, Producer, spsc};
use bodo_kit_core::{Frames, SampleRate, Seconds, TempoMap, rt_safe};
use bodo_kit_device::StreamStatus;
use bodo_model::mixer::{RecordFormat, RecordTap};
use bodo_model::{InsertId, Project};

use crate::context::BlockContext;

/// Seconds of stereo audio each insert's ring holds; the disk thread drains it every few milliseconds.
const RING_SECONDS: usize = 4;
/// How often the disk thread looks for new audio.
const DISK_POLL: Duration = Duration::from_millis(5);

/// Why recording could not start or finish.
#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    /// Recording is already running.
    #[error("recording is already running")]
    AlreadyRecording,
    /// No insert is armed and able to record.
    #[error("no insert is armed for recording")]
    NothingArmed,
    /// A take file could not be created or written.
    #[error("could not write the take {path}: {message}")]
    File {
        /// The take's file.
        path: PathBuf,
        /// What went wrong.
        message: String,
    },
    /// The disk thread could not start, or stopped unexpectedly.
    #[error("the recording disk thread failed: {0}")]
    Thread(String),
}

/// How far a take moves back on the timeline to make up for the audio device's latency.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Compensation {
    /// Time from a sound reaching the input to the engine seeing it.
    pub input: Frames,
    /// Time from the engine rendering a frame to the listener hearing it.
    pub output: Frames,
}

impl Compensation {
    /// The latencies `status` reports.
    pub fn from_status(status: &StreamStatus) -> Self {
        Self { input: status.input_latency, output: status.output_latency }
    }

    /// The whole round trip: what a take moves back by.
    pub fn total(self) -> Frames {
        Frames(self.input.0 + self.output.0)
    }
}

/// A finished recording of one insert.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Take {
    /// The insert it recorded.
    pub insert: InsertId,
    /// The WAV file.
    pub path: PathBuf,
    /// The transport frame at which its first frame arrived in the engine.
    pub start_frame: i64,
    /// How many frames it holds.
    pub frames: u64,
    /// The latency it makes up for.
    pub compensation: Compensation,
    /// Blocks dropped because the ring was full; a take with overflows has gaps.
    pub overflows: u64,
}

impl Take {
    /// The timeline frame its first frame belongs to, after latency compensation. Negative when it began before the
    /// song.
    pub fn timeline_frame(&self) -> i64 {
        self.start_frame - self.compensation.total().0
    }

    /// [`Take::timeline_frame`] in ticks under `tempo` at `rate`: where its clip starts. Negative positions count back
    /// from the start at the song's first tempo.
    ///
    /// Complexity: O(log n) in tempo points.
    pub fn timeline_tick(&self, tempo: &TempoMap, rate: SampleRate) -> i64 {
        let frame = self.timeline_frame();
        let seconds = |frames: i64| Seconds(frames as f64 / f64::from(rate.hz()));
        if frame >= 0 { tempo.ticks_at(seconds(frame)).round() as i64 } else { -(tempo.ticks_at(seconds(-frame)).round() as i64) }
    }
}

/// Which stage of a strip records a tap.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CaptureStage {
    /// The input stage: the Input and PreFx taps.
    Input,
    /// The FX stage: the PostFx tap.
    Fx,
    /// The output stage: the PostFader tap.
    Output,
}

impl CaptureStage {
    /// The stage that has `tap`.
    pub fn of(tap: RecordTap) -> Self {
        match tap {
            RecordTap::Input | RecordTap::PreFx => Self::Input,
            RecordTap::PostFx => Self::Fx,
            RecordTap::PostFader => Self::Output,
        }
    }
}

/// How many loop passes one recording can mark; later passes join the last take.
const PASS_CAPACITY: usize = 256;

/// The audio thread's end of one insert's recording.
#[derive(Debug)]
pub struct Capture {
    producer: Producer<f32>,
    tap: RecordTap,
    /// The transport frame of the first block captured, or `i64::MIN` before it.
    first_frame: Arc<AtomicI64>,
    overflows: Arc<AtomicU64>,
    /// Where loop passes start: the frames pushed before each, and the timeline frame it starts at.
    passes: Producer<(u64, i64)>,
    /// Frames pushed so far.
    pushed: u64,
}

/// What a strip holds: a capture while recording, `None` otherwise.
pub type CaptureSlot = Option<Box<Capture>>;

/// The receiver and the current capture of a strip.
#[derive(Debug, Default)]
struct CaptureState {
    receiver: Option<Receiver<CaptureSlot>>,
    current: CaptureSlot,
}

/// A strip's side of recording: it picks up captures the control side sends and feeds the current one.
///
/// The state sits in a mutex that is never locked. Graph processors must be `Sync`, ring handles are not, and the
/// strip always has the state to itself, so [`Mutex::get_mut`] borrows it with no lock and no system call.
#[derive(Debug, Default)]
pub struct StripCapture {
    state: Mutex<CaptureState>,
}

impl StripCapture {
    /// A strip capture that takes captures from `receiver`.
    pub fn new(receiver: Receiver<CaptureSlot>) -> Self {
        Self { state: Mutex::new(CaptureState { receiver: Some(receiver), current: None }) }
    }

    /// The state, borrowed exclusively without locking.
    fn state(&mut self) -> &mut CaptureState {
        self.state.get_mut().unwrap_or_else(PoisonError::into_inner)
    }

    /// Switches to the capture the control side sent last, handing the old one back. Call at the start of a block.
    ///
    /// RT-safe.
    #[rt_safe]
    pub fn update(&mut self) {
        let state = self.state();
        if let Some(receiver) = state.receiver.as_mut() {
            receiver.update(&mut state.current);
        }
    }

    /// Records `left` and `right` if the capture taps `stage` and the transport plays. A full ring drops the block
    /// and counts an overflow.
    ///
    /// RT-safe. Complexity: O(n) in the frames.
    #[rt_safe]
    pub fn offer(&mut self, stage: RecordTap, left: &[f32], right: &[f32], context: &BlockContext) {
        let Some(capture) = self.state().current.as_mut() else { return };
        if capture.tap != stage || !context.playing {
            return;
        }
        let frames = left.len().min(right.len());
        if let Some(start) = context.pass_start
            && capture.pushed > 0
        {
            capture.passes.push_or_drop((capture.pushed, start));
        }
        if capture.producer.free_slots() < frames * 2 {
            capture.overflows.fetch_add(1, Ordering::Relaxed);
            return;
        }
        if capture.first_frame.load(Ordering::Relaxed) == i64::MIN {
            capture.first_frame.store(context.frame, Ordering::Relaxed);
        }
        for (l, r) in left.iter().zip(right.iter()) {
            capture.producer.push_or_drop(*l);
            capture.producer.push_or_drop(*r);
        }
        capture.pushed += frames as u64;
    }
}

/// The control side's links to every strip stage's capture slot.
#[derive(Debug, Default)]
pub(crate) struct CaptureLinks {
    senders: HashMap<(InsertId, CaptureStage), Sender<CaptureSlot>>,
}

impl CaptureLinks {
    /// A new link for `stage` of the strip of `insert`, replacing any older one, and the receiver its node keeps.
    pub(crate) fn link(&mut self, insert: InsertId, stage: CaptureStage) -> Receiver<CaptureSlot> {
        let (sender, receiver) = handoff::handoff(4);
        self.senders.insert((insert, stage), sender);
        receiver
    }

    /// Forgets the links of inserts `keep` rejects.
    pub(crate) fn retain(&mut self, keep: impl Fn(&InsertId) -> bool) {
        self.senders.retain(|(id, _), _| keep(id));
    }

    /// Sends `slot` to `stage` of the strip of `insert`. Returns false when there is no such strip or its queue is
    /// full.
    fn send(&mut self, insert: InsertId, stage: CaptureStage, slot: CaptureSlot) -> bool {
        self.senders.get_mut(&(insert, stage)).is_some_and(|sender| sender.send(slot).is_ok())
    }

    /// Drops the captures strips handed back, which closes their rings so the disk thread can finish.
    pub(crate) fn collect(&mut self) {
        for sender in self.senders.values_mut() {
            sender.collect();
        }
    }
}

/// One insert being recorded, as the control side sees it.
#[derive(Debug)]
struct Lane {
    insert: InsertId,
    stage: CaptureStage,
    first_frame: Arc<AtomicI64>,
    overflows: Arc<AtomicU64>,
}

/// A file the disk thread wrote: its path, the timeline frame it starts at (`None` for a lane's first file, which
/// starts at the lane's first captured frame), and its length.
#[derive(Debug)]
struct Written {
    path: PathBuf,
    start: Option<i64>,
    frames: u64,
}

/// What the disk thread wrote, per lane.
type WrittenLanes = Result<Vec<Vec<Written>>, RecordError>;

/// A recording in progress.
#[derive(Debug)]
struct Active {
    lanes: Vec<Lane>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<WrittenLanes>>,
    compensation: Compensation,
    stopping: bool,
}

/// The control side of recording.
#[derive(Debug, Default)]
pub(crate) struct Recorder {
    active: Option<Active>,
}

/// `name` made safe for a file name.
fn file_name(name: &str) -> String {
    let clean: String = name.chars().map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_') { c } else { '_' }).collect();
    let clean = clean.trim();
    if clean.is_empty() { String::from("Insert") } else { clean.to_owned() }
}

/// The file extension of recordings in `format`.
pub fn extension(format: RecordFormat) -> &'static str {
    match format {
        RecordFormat::Float32 | RecordFormat::Int24 => "wav",
        RecordFormat::Flac => "flac",
        RecordFormat::W64 => "w64",
    }
}

/// The first `<folder>/<name>_<n>.<extension>` that doesn't exist yet, counting from 1.
///
/// Does I/O. Complexity: O(n) in the takes already there.
pub fn next_take_path(folder: &Path, name: &str, extension: &str) -> PathBuf {
    let name = file_name(name);
    (1..)
        .map(|n| folder.join(format!("{name}_{n}.{extension}")))
        .find(|path| !path.exists())
        .unwrap_or_else(|| folder.join(format!("{name}.{extension}")))
}

/// A take file being written, in any of the recording formats.
#[derive(Debug)]
enum TakeWriter {
    Wav(WavWriter),
    W64(W64Writer),
    Flac(FlacWriter),
}

impl TakeWriter {
    /// Creates `path` for a stereo take at `rate` in `format`.
    ///
    /// Does I/O.
    fn create(path: &Path, rate: SampleRate, format: RecordFormat) -> Result<Self, AudioError> {
        Ok(match format {
            RecordFormat::Float32 => Self::Wav(WavWriter::create(path, rate, 2, WavFormat::Float32)?),
            RecordFormat::Int24 => Self::Wav(WavWriter::create(path, rate, 2, WavFormat::Int24)?),
            RecordFormat::Flac => Self::Flac(FlacWriter::create(path, rate, 2)?),
            RecordFormat::W64 => Self::W64(W64Writer::create(path, rate, 2, W64Format::Float32)?),
        })
    }

    fn write(&mut self, channels: &[&[f32]], frames: usize) -> Result<(), AudioError> {
        match self {
            Self::Wav(w) => w.write(channels, frames),
            Self::W64(w) => w.write(channels, frames),
            Self::Flac(w) => w.write(channels, frames),
        }
    }

    fn finish(self) -> Result<(), AudioError> {
        match self {
            Self::Wav(w) => w.finish(),
            Self::W64(w) => w.finish(),
            Self::Flac(w) => w.finish(),
        }
    }
}

/// What the disk thread needs for one lane.
#[derive(Debug)]
struct DiskLane {
    samples: Consumer<f32>,
    passes: Consumer<(u64, i64)>,
    writer: Option<TakeWriter>,
    current: Written,
    done: Vec<Written>,
    /// Frames taken from the ring so far.
    consumed: u64,
    /// The next pass mark, once read.
    next_pass: Option<(u64, i64)>,
    folder: PathBuf,
    name: String,
    format: RecordFormat,
    rate: SampleRate,
}

impl DiskLane {
    fn file_error(path: &Path) -> impl Fn(AudioError) -> RecordError + '_ {
        move |e| RecordError::File { path: path.to_path_buf(), message: e.to_string() }
    }

    /// Writes `left` and `right`, starting a new file at every pass mark they cross.
    fn write(&mut self, mut left: &[f32], mut right: &[f32]) -> Result<(), RecordError> {
        while !left.is_empty() {
            if self.next_pass.is_none() {
                self.next_pass = self.passes.pop();
            }
            let until = self.next_pass.map_or(left.len() as u64, |(at, _)| at.saturating_sub(self.consumed).min(left.len() as u64)) as usize;
            if until > 0 {
                let (now_l, rest_l) = left.split_at(until);
                let (now_r, rest_r) = right.split_at(until.min(right.len()));
                if let Some(writer) = self.writer.as_mut() {
                    writer.write(&[now_l, now_r], until).map_err(Self::file_error(&self.current.path))?;
                }
                self.current.frames += until as u64;
                self.consumed += until as u64;
                left = rest_l;
                right = rest_r;
            }
            if let Some((at, start)) = self.next_pass
                && at <= self.consumed
            {
                self.next_pass = None;
                self.split(start)?;
            }
        }
        Ok(())
    }

    /// Finishes the current file and starts the next pass's at timeline frame `start`.
    fn split(&mut self, start: i64) -> Result<(), RecordError> {
        if let Some(writer) = self.writer.take() {
            writer.finish().map_err(Self::file_error(&self.current.path))?;
        }
        let path = next_take_path(&self.folder, &self.name, extension(self.format));
        let writer = TakeWriter::create(&path, self.rate, self.format).map_err(Self::file_error(&path))?;
        let finished = std::mem::replace(&mut self.current, Written { path, start: Some(start), frames: 0 });
        self.done.push(finished);
        self.writer = Some(writer);
        Ok(())
    }

    /// Finishes the last file and returns every file written.
    fn finish(mut self) -> Result<Vec<Written>, RecordError> {
        if let Some(writer) = self.writer.take() {
            writer.finish().map_err(Self::file_error(&self.current.path))?;
        }
        self.done.push(self.current);
        Ok(self.done)
    }
}

impl Recorder {
    /// Whether a recording is running or finishing.
    pub(crate) fn is_active(&self) -> bool {
        self.active.is_some()
    }

    /// Starts recording every armed insert of `project` into `folder`, through `links`. Returns how many inserts record.
    ///
    /// Creates the files and rings, and starts the disk thread. Complexity: O(i) in inserts.
    pub(crate) fn start(
        &mut self,
        project: &Project,
        folder: &Path,
        compensation: Compensation,
        links: &mut CaptureLinks,
    ) -> Result<usize, RecordError> {
        if self.active.is_some() {
            return Err(RecordError::AlreadyRecording);
        }
        std::fs::create_dir_all(folder).map_err(|e| RecordError::File { path: folder.to_path_buf(), message: e.to_string() })?;
        let rate = project.settings.sample_rate;
        let capacity = RING_SECONDS * rate.hz() as usize * 2;
        let mut lanes = Vec::new();
        let mut disk: Vec<DiskLane> = Vec::new();
        let armed = project.mixer.order.iter().filter_map(|id| project.mixer.inserts.get(id)).chain(std::iter::once(&project.mixer.master));
        for insert in armed.filter(|i| i.armed) {
            let path = next_take_path(folder, &insert.name, extension(insert.record_format));
            let writer = TakeWriter::create(&path, rate, insert.record_format).map_err(DiskLane::file_error(&path))?;
            let (producer, samples) = spsc(capacity);
            let (passes_tx, passes) = spsc(PASS_CAPACITY);
            let first_frame = Arc::new(AtomicI64::new(i64::MIN));
            let overflows = Arc::new(AtomicU64::new(0));
            let capture = Capture {
                producer,
                tap: insert.record_tap,
                first_frame: Arc::clone(&first_frame),
                overflows: Arc::clone(&overflows),
                passes: passes_tx,
                pushed: 0,
            };
            let stage = CaptureStage::of(insert.record_tap);
            if !links.send(insert.id, stage, Some(Box::new(capture))) {
                tracing::warn!(insert = %insert.id, "the insert has no strip yet; it isn't recorded");
                let _ = writer.finish();
                let _ = std::fs::remove_file(&path);
                continue;
            }
            disk.push(DiskLane {
                samples,
                passes,
                writer: Some(writer),
                current: Written { path, start: None, frames: 0 },
                done: Vec::new(),
                consumed: 0,
                next_pass: None,
                folder: folder.to_path_buf(),
                name: insert.name.clone(),
                format: insert.record_format,
                rate,
            });
            lanes.push(Lane { insert: insert.id, stage, first_frame, overflows });
        }
        if lanes.is_empty() {
            return Err(RecordError::NothingArmed);
        }
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name(String::from("bodo-recorder"))
            .spawn(move || write_takes(disk, &thread_stop))
            .map_err(|e| RecordError::Thread(e.to_string()))?;
        let count = lanes.len();
        self.active = Some(Active { lanes, stop, thread: Some(thread), compensation, stopping: false });
        Ok(count)
    }

    /// Asks every strip to hand its capture back; [`Recorder::finish`] then completes the takes.
    pub(crate) fn stop(&mut self, links: &mut CaptureLinks) {
        let Some(active) = self.active.as_mut() else { return };
        if active.stopping {
            return;
        }
        for lane in &active.lanes {
            if !links.send(lane.insert, lane.stage, None) {
                tracing::warn!(insert = %lane.insert, "could not ask the strip to stop recording");
            }
        }
        active.stop.store(true, Ordering::Relaxed);
        active.stopping = true;
    }

    /// The takes, once a stopped recording has finished writing; `None` while it is still running or finishing.
    /// A lane recorded over several loop passes gives one take per pass.
    pub(crate) fn finish(&mut self, links: &mut CaptureLinks) -> Option<Result<Vec<Take>, RecordError>> {
        links.collect();
        let active = self.active.as_mut()?;
        if !active.stopping || !active.thread.as_ref().is_some_and(JoinHandle::is_finished) {
            return None;
        }
        let active = self.active.take()?;
        let written = match active.thread.map(JoinHandle::join) {
            Some(Ok(written)) => written,
            Some(Err(_)) | None => Err(RecordError::Thread(String::from("the disk thread stopped unexpectedly"))),
        };
        Some(written.map(|files| {
            let mut takes = Vec::new();
            for (lane, files) in active.lanes.into_iter().zip(files) {
                let first = lane.first_frame.load(Ordering::Relaxed);
                let overflows = lane.overflows.load(Ordering::Relaxed);
                for file in files {
                    let start = file.start.unwrap_or(first);
                    if file.frames == 0 || start == i64::MIN {
                        let _ = std::fs::remove_file(&file.path);
                        continue;
                    }
                    takes.push(Take {
                        insert: lane.insert,
                        path: file.path,
                        start_frame: start,
                        frames: file.frames,
                        compensation: active.compensation,
                        overflows,
                    });
                }
            }
            takes
        }))
    }
}

/// The disk thread: drains every ring into its files until recording stops and every ring is closed and empty,
/// starting a new file at each loop pass. Returns the files written for each lane.
fn write_takes(mut lanes: Vec<DiskLane>, stop: &AtomicBool) -> WrittenLanes {
    let (mut left, mut right) = (Vec::new(), Vec::new());
    loop {
        let stopping = stop.load(Ordering::Relaxed);
        let mut busy = false;
        for lane in &mut lanes {
            let frames = lane.samples.len() / 2;
            if frames == 0 {
                continue;
            }
            busy = true;
            left.clear();
            right.clear();
            for _ in 0..frames {
                left.push(lane.samples.pop().unwrap_or(0.0));
                right.push(lane.samples.pop().unwrap_or(0.0));
            }
            lane.write(&left, &right)?;
        }
        let closed = lanes.iter().all(|lane| lane.samples.is_abandoned() && lane.samples.len() < 2);
        if stopping && closed {
            break;
        }
        if !busy {
            std::thread::sleep(DISK_POLL);
        }
    }
    lanes.into_iter().map(DiskLane::finish).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_get_numbered_names_that_are_safe_on_disk() {
        let dir = std::env::temp_dir().join(format!("bodo-record-names-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = next_take_path(&dir, "Vox: lead/double", "wav");
        assert_eq!(first.file_name().and_then(|n| n.to_str()), Some("Vox_ lead_double_1.wav"));
        std::fs::write(&first, b"").unwrap();
        assert_eq!(next_take_path(&dir, "Vox: lead/double", "wav").file_name().and_then(|n| n.to_str()), Some("Vox_ lead_double_2.wav"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_full_ring_drops_the_block_and_counts_it() {
        let (producer, mut consumer) = spsc(8);
        let first_frame = Arc::new(AtomicI64::new(i64::MIN));
        let overflows = Arc::new(AtomicU64::new(0));
        let capture = Capture {
            producer,
            tap: RecordTap::Input,
            first_frame: Arc::clone(&first_frame),
            overflows: Arc::clone(&overflows),
            passes: spsc(4).0,
            pushed: 0,
        };
        let mut strip = StripCapture { state: Mutex::new(CaptureState { receiver: None, current: Some(Box::new(capture)) }) };
        let mut context = BlockContext::new(SampleRate::HZ_48000);
        let block = [0.5f32; 4];
        strip.offer(RecordTap::Input, &block, &block, &context);
        assert_eq!(overflows.load(Ordering::Relaxed), 0, "the transport is stopped: nothing is captured");
        assert_eq!(consumer.len(), 0);
        context.playing = true;
        context.frame = 480;
        strip.offer(RecordTap::PostFader, &block, &block, &context);
        assert_eq!(consumer.len(), 0, "another tap isn't captured");
        strip.offer(RecordTap::Input, &block, &block, &context);
        assert_eq!((consumer.len(), first_frame.load(Ordering::Relaxed)), (8, 480));
        strip.offer(RecordTap::Input, &block, &block, &context);
        assert_eq!((consumer.len(), overflows.load(Ordering::Relaxed)), (8, 1), "a full ring drops the block whole");
        assert_eq!(consumer.pop(), Some(0.5));
    }

    #[test]
    fn compensation_moves_takes_back_by_the_round_trip() {
        let take = Take {
            insert: InsertId::from_u128(1),
            path: PathBuf::from("x.wav"),
            start_frame: 1000,
            frames: 10,
            compensation: Compensation { input: Frames(100), output: Frames(300) },
            overflows: 0,
        };
        assert_eq!(take.timeline_frame(), 600);
        let tempo = TempoMap::constant(120.0).unwrap();
        // 120 BPM: a beat (960 ticks) is half a second, 24 000 frames at 48 kHz.
        let early = Take { start_frame: 0, compensation: Compensation { input: Frames(0), output: Frames(24_000) }, ..take };
        assert_eq!(early.timeline_tick(&tempo, SampleRate::HZ_48000), -960);
    }
}
