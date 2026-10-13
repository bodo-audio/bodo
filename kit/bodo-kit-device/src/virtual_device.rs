//! A device without hardware, for tests, CI and headless timing runs.
//!
//! It drives an [`AudioCallback`] through the same [`Driver`] as a real device, so channel maps, block splitting,
//! denormal flushing, the real-time checks and dropout counting all behave the same. [`VirtualDevice`] renders one
//! buffer at a time on the caller's thread; [`open_virtual`](crate::open_virtual) runs it on its own thread, either
//! paced like real hardware or as fast as possible.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use bodo_kit_core::SampleRate;

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
    /// Input channels. Virtual input is silent.
    pub input_channels: usize,
    /// How callbacks are spaced when the device runs on its own thread.
    pub pacing: Pacing,
}

impl Default for VirtualConfig {
    /// 48 kHz, 128 frames, stereo out, no input, paced in real time.
    fn default() -> Self {
        Self { sample_rate: SampleRate::HZ_48000, buffer_frames: 128, output_channels: 2, input_channels: 0, pacing: Pacing::RealTime }
    }
}

/// A virtual device rendered one buffer at a time on the caller's thread.
pub struct VirtualDevice<C> {
    driver: Driver<C>,
    buffer: Vec<f32>,
    channels: usize,
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
        Self {
            driver: Driver::new(callback, config.sample_rate, channels, &output_map, input_map, None),
            buffer: vec![0.0; config.buffer_frames as usize * channels],
            channels,
        }
    }

    /// Renders one buffer and returns it, interleaved.
    ///
    /// RT-safe as long as the callback is.
    pub fn render(&mut self) -> &[f32] {
        self.driver.render(&mut self.buffer, self.channels, false);
        &self.buffer
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
                device.driver.render(&mut device.buffer, device.channels, late);
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
    Ok(AudioStream { stats, input_stats: None, inner: StreamInner::Thread { stop, handle: Some(handle) }, rate: config.sample_rate })
}
