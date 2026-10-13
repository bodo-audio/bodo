//! A device without hardware, for tests, CI and headless timing runs.
//!
//! It drives an [`AudioCallback`] through the same [`Driver`] as a real device, so channel maps, block splitting,
//! denormal flushing, the real-time checks and dropout counting all behave the same. [`VirtualDevice`] renders one
//! buffer at a time on the caller's thread; [`open_virtual`](crate::open_virtual) runs it on its own thread, either
//! paced like real hardware or as fast as possible.
//!
//! A loopback can play the outputs straight back into the inputs after a delay, as a cable from an output to an input
//! would, and the device can report any input and output latency. Together they measure a whole recording round trip
//! without hardware.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use bodo_kit_core::SampleRate;
use bodo_kit_core::queue::{Producer, spsc};

use crate::driver::{AudioCallback, Driver, StreamStatus};
use crate::{AudioStream, DeviceError, StreamInner};

/// How a virtual device running on its own thread spaces its callbacks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Pacing {
    /// One callback per buffer period, as real hardware would. Use it to measure dropouts without a sound card.
    #[default]
    RealTime,
    /// Callbacks back to back, as fast as the engine can go.
    Freewheel,
}

/// Settings of a virtual device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VirtualConfig {
    /// The sample rate.
    pub sample_rate: SampleRate,
    /// Frames per callback.
    pub buffer_frames: u32,
    /// Output channels.
    pub output_channels: usize,
    /// Input channels. They are silent unless `loopback` feeds them.
    pub input_channels: usize,
    /// How callbacks are spaced when the device runs on its own thread.
    pub pacing: Pacing,
    /// Feeds output channel `n` back to input channel `n` after this many frames, or `None` for silent inputs. The
    /// delay is at least one buffer, as it is on real hardware, so smaller values are raised to `buffer_frames`.
    pub loopback: Option<u32>,
    /// The input latency the stream reports, in frames.
    pub input_latency: u32,
    /// The output latency the stream reports, in frames.
    pub output_latency: u32,
}

impl Default for VirtualConfig {
    /// 48 kHz, 128 frames, stereo out, no input, paced in real time, no loopback and no reported latency.
    fn default() -> Self {
        Self {
            sample_rate: SampleRate::HZ_48000,
            buffer_frames: 128,
            output_channels: 2,
            input_channels: 0,
            pacing: Pacing::RealTime,
            loopback: None,
            input_latency: 0,
            output_latency: 0,
        }
    }
}

/// A virtual device rendered one buffer at a time on the caller's thread.
pub struct VirtualDevice<C> {
    driver: Driver<C>,
    buffer: Vec<f32>,
    channels: usize,
    /// Where rendered output goes back in as input, and how many input channels there are.
    loopback: Option<(Producer<f32>, usize)>,
}

impl<C> std::fmt::Debug for VirtualDevice<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VirtualDevice").field("channels", &self.channels).finish_non_exhaustive()
    }
}

impl<C: AudioCallback> VirtualDevice<C> {
    /// A device that calls `callback` for each rendered buffer.
    ///
    /// Allocates the buffers.
    pub fn new(config: VirtualConfig, callback: C) -> Self {
        let channels = config.output_channels.max(1);
        let output_map: Vec<usize> = (0..config.output_channels).collect();
        let input_map: Vec<usize> = (0..config.input_channels).collect();
        let inputs = config.input_channels;
        let (input_queue, loopback) = match config.loopback {
            Some(delay) if inputs > 0 => {
                // The queue holds the delay plus one buffer in flight; it starts with the delay's worth of silence.
                let delay = delay.max(config.buffer_frames) as usize;
                let (mut producer, consumer) = spsc((delay + 2 * config.buffer_frames as usize) * inputs);
                for _ in 0..delay * inputs {
                    producer.push_or_drop(0.0);
                }
                (Some((consumer, inputs)), Some((producer, inputs)))
            }
            _ => (None, None),
        };
        let driver = Driver::new(callback, config.sample_rate, channels, &output_map, input_map, input_queue);
        driver.stats.input_latency.store(config.input_latency, Ordering::Relaxed);
        driver.stats.output_latency.store(config.output_latency, Ordering::Relaxed);
        Self { driver, buffer: vec![0.0; config.buffer_frames as usize * channels], channels, loopback }
    }

    /// Renders one buffer and returns it, interleaved.
    ///
    /// RT-safe as long as the callback is.
    pub fn render(&mut self) -> &[f32] {
        self.render_buffer(false);
        &self.buffer
    }

    /// Renders one buffer, reporting it late when `late` is set, and feeds the loopback.
    ///
    /// RT-safe as long as the callback is. Complexity: O(frames * channels).
    fn render_buffer(&mut self, late: bool) {
        self.driver.render(&mut self.buffer, self.channels, late);
        let Some((producer, inputs)) = self.loopback.as_mut() else { return };
        for frame in self.buffer.chunks(self.channels) {
            for input in 0..*inputs {
                producer.push_or_drop(frame.get(input).copied().unwrap_or(0.0));
            }
        }
    }

    /// The counters so far.
    pub fn status(&self) -> StreamStatus {
        self.driver.stats.snapshot()
    }
}

/// Runs a virtual device on its own thread until the returned stream is dropped.
pub(crate) fn spawn<C: AudioCallback>(config: VirtualConfig, callback: C) -> Result<AudioStream, DeviceError> {
    let mut device = VirtualDevice::new(config, callback);
    let stats = Arc::clone(&device.driver.stats);
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let period = Duration::from_secs_f64(f64::from(config.buffer_frames) / f64::from(config.sample_rate.hz()));
    let handle = thread::Builder::new()
        .name(String::from("bodo-virtual-device"))
        .spawn(move || {
            let _priority = audio_thread_priority::promote_current_thread_to_real_time(config.buffer_frames, config.sample_rate.hz())
                .inspect_err(|err| tracing::debug!(%err, "virtual device runs without real-time priority"))
                .ok();
            let mut deadline = Instant::now() + period;
            while !thread_stop.load(Ordering::Relaxed) {
                let late = config.pacing == Pacing::RealTime && Instant::now() > deadline + period;
                device.render_buffer(late);
                if config.pacing == Pacing::RealTime {
                    let now = Instant::now();
                    if deadline > now {
                        thread::sleep(deadline - now);
                    } else if now > deadline + period {
                        // Fell more than a buffer behind: start a fresh schedule rather than racing to catch up.
                        deadline = now;
                    }
                    deadline += period;
                }
            }
        })
        .map_err(DeviceError::Thread)?;
    Ok(AudioStream {
        stats,
        input_stats: None,
        thread_group: None,
        inner: StreamInner::Thread { stop, handle: Some(handle) },
        rate: config.sample_rate,
    })
}
