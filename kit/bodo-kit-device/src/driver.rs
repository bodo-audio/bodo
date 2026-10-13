//! The part of every device that runs on the audio thread: blocks, channel maps, timing and dropout counting.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::Instant;

use bodo_kit_core::queue::Consumer;
use bodo_kit_core::{Frames, MAX_BLOCK_FRAMES, SampleRate, rt, rt_safe};
use bodo_kit_dsp::AudioBuffer;

/// One block handed to an [`AudioCallback`].
#[derive(Debug)]
pub struct Block<'a> {
    /// Frames in this block, at most [`MAX_BLOCK_FRAMES`].
    pub frames: usize,
    /// Recorded input, one channel per configured input channel, `frames` long. Silent when the device has no input
    /// or the input fell behind.
    pub input: &'a AudioBuffer,
    /// The output to fill, one channel per configured output channel, `frames` long. Write every frame.
    pub output: &'a mut AudioBuffer,
    /// Frames rendered before this block, since the stream started.
    pub position: u64,
    /// The stream's sample rate.
    pub rate: SampleRate,
}

/// The engine side of a device: called for every block on the audio thread.
pub trait AudioCallback: Send + 'static {
    /// Fills `block.output`. Called on the audio thread, so it must be RT-safe.
    fn process(&mut self, block: &mut Block<'_>);
}

impl<F: FnMut(&mut Block<'_>) + Send + 'static> AudioCallback for F {
    fn process(&mut self, block: &mut Block<'_>) {
        self(block);
    }
}

/// Counters a stream updates as it runs, shared with whoever holds the stream.
#[derive(Debug, Default)]
pub(crate) struct Stats {
    pub(crate) callbacks: AtomicU64,
    pub(crate) frames: AtomicU64,
    pub(crate) xruns: AtomicU64,
    pub(crate) input_underruns: AtomicU64,
    /// Time spent in the last callback as a fraction of its deadline, in thousandths.
    pub(crate) load_permille: AtomicU32,
    pub(crate) peak_load_permille: AtomicU32,
    pub(crate) output_latency: AtomicU32,
    pub(crate) input_latency: AtomicU32,
    pub(crate) disconnected: AtomicBool,
}

/// A snapshot of a stream's counters.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StreamStatus {
    /// Callbacks run so far.
    pub callbacks: u64,
    /// Frames rendered so far.
    pub frames: u64,
    /// Dropouts: callbacks that overran their deadline, or that the device reported arriving late.
    pub xruns: u64,
    /// Blocks where recorded input had not arrived in time.
    pub input_underruns: u64,
    /// The last callback's processing time as a fraction of its deadline: 1.0 means it used all of it.
    pub load: f32,
    /// The highest load seen.
    pub peak_load: f32,
    /// Time from rendering a frame to hearing it, as the device reports it.
    pub output_latency: Frames,
    /// Time from a sound arriving to the callback seeing it, as the device reports it.
    pub input_latency: Frames,
    /// Whether the device went away. The stream is dead and must be reopened.
    pub disconnected: bool,
}

impl Stats {
    pub(crate) fn snapshot(&self) -> StreamStatus {
        StreamStatus {
            callbacks: self.callbacks.load(Ordering::Relaxed),
            frames: self.frames.load(Ordering::Relaxed),
            xruns: self.xruns.load(Ordering::Relaxed),
            input_underruns: self.input_underruns.load(Ordering::Relaxed),
            load: self.load_permille.load(Ordering::Relaxed) as f32 / 1000.0,
            peak_load: self.peak_load_permille.load(Ordering::Relaxed) as f32 / 1000.0,
            output_latency: Frames(i64::from(self.output_latency.load(Ordering::Relaxed))),
            input_latency: Frames(i64::from(self.input_latency.load(Ordering::Relaxed))),
            disconnected: self.disconnected.load(Ordering::Relaxed),
        }
    }
}

/// Runs an [`AudioCallback`] for interleaved device buffers.
pub(crate) struct Driver<C> {
    callback: C,
    input: AudioBuffer,
    output: AudioBuffer,
    /// For each device output channel, the engine output channel that feeds it.
    output_route: Vec<Option<usize>>,
    /// Recorded input from a separate input stream, interleaved, and its device channel count.
    input_queue: Option<(Consumer<f32>, usize)>,
    /// For each engine input channel, the device input channel it reads.
    input_map: Vec<usize>,
    position: u64,
    rate: SampleRate,
    pub(crate) stats: Arc<Stats>,
}

impl<C: AudioCallback> Driver<C> {
    /// Allocates the block buffers. `output_map[i]` is the device channel engine output `i` plays on, and
    /// `input_map[i]` the device channel engine input `i` reads from `input_queue`, which carries interleaved frames
    /// of the given device channel count. Without a queue the inputs are silent.
    pub(crate) fn new(
        callback: C,
        rate: SampleRate,
        device_outputs: usize,
        output_map: &[usize],
        input_map: Vec<usize>,
        input_queue: Option<(Consumer<f32>, usize)>,
    ) -> Self {
        let mut output_route = vec![None; device_outputs];
        for (engine, &device) in output_map.iter().enumerate() {
            if let Some(slot) = output_route.get_mut(device) {
                *slot = Some(engine);
            }
        }
        Self {
            callback,
            input: AudioBuffer::for_blocks(input_map.len()),
            output: AudioBuffer::for_blocks(output_map.len()),
            output_route,
            input_queue,
            input_map,
            position: 0,
            rate,
            stats: Arc::new(Stats::default()),
        }
    }

    /// Renders an interleaved device buffer of `device_channels` channels, in blocks of at most
    /// [`MAX_BLOCK_FRAMES`]. `late` says the device reported this callback as late.
    ///
    /// RT-safe as long as the callback is.
    #[rt_safe]
    pub(crate) fn render(&mut self, out: &mut [f32], device_channels: usize, late: bool) {
        let start = Instant::now();
        let total = out.len() / device_channels.max(1);
        rt::flush_denormals(|| {
            let mut done = 0;
            while done < total {
                let frames = (total - done).min(MAX_BLOCK_FRAMES);
                self.read_input(frames);
                self.output.set_frames(frames);
                let mut block = Block { frames, input: &self.input, output: &mut self.output, position: self.position, rate: self.rate };
                self.callback.process(&mut block);
                let chunk = out.get_mut(done * device_channels..(done + frames) * device_channels).unwrap_or(&mut []);
                for (frame, samples) in chunk.chunks_mut(device_channels.max(1)).enumerate() {
                    for (device_channel, sample) in samples.iter_mut().enumerate() {
                        *sample = match self.output_route.get(device_channel).copied().flatten() {
                            Some(engine) => self.output.channel(engine).get(frame).copied().unwrap_or(0.0),
                            None => 0.0,
                        };
                    }
                }
                done += frames;
                self.position += frames as u64;
            }
        });
        self.account(start, total, late);
    }

    /// Fills the input buffer from the input queue, or with silence.
    fn read_input(&mut self, frames: usize) {
        self.input.set_frames(frames);
        let Some((queue, device_channels)) = self.input_queue.as_mut() else {
            self.input.clear();
            return;
        };
        let device_channels = *device_channels;
        if queue.len() < frames * device_channels {
            self.input.clear();
            self.stats.input_underruns.fetch_add(1, Ordering::Relaxed);
            return;
        }
        for frame in 0..frames {
            for device_channel in 0..device_channels {
                let sample = queue.pop().unwrap_or(0.0);
                for (engine, &mapped) in self.input_map.iter().enumerate() {
                    if mapped == device_channel
                        && let Some(slot) = self.input.channel_mut(engine).get_mut(frame)
                    {
                        *slot = sample;
                    }
                }
            }
        }
    }

    /// Updates the counters after a callback that started at `start` and rendered `frames` frames.
    fn account(&self, start: Instant, frames: usize, late: bool) {
        let budget = frames as f64 / f64::from(self.rate.hz());
        let load = if budget > 0.0 { start.elapsed().as_secs_f64() / budget } else { 0.0 };
        let permille = (load * 1000.0).min(f64::from(u32::MAX)) as u32;
        self.stats.load_permille.store(permille, Ordering::Relaxed);
        self.stats.peak_load_permille.fetch_max(permille, Ordering::Relaxed);
        if load > 1.0 || late {
            self.stats.xruns.fetch_add(1, Ordering::Relaxed);
        }
        self.stats.callbacks.fetch_add(1, Ordering::Relaxed);
        self.stats.frames.fetch_add(frames as u64, Ordering::Relaxed);
    }
}
