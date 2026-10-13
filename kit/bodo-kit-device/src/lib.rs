//! Audio devices for Bodo Kit (project.md section 6.2).
//!
//! - [`hosts`] and [`devices`] list what this machine offers. On Windows that includes "WASAPI Exclusive", which
//!   takes over a sound card for low latency without an ASIO driver.
//! - [`open`] starts a stream on a real device with a [`DeviceConfig`]: host, devices, sample rate, buffer size and
//!   channel maps.
//! - [`open_virtual`] and [`VirtualDevice`] run the same engine without hardware, for tests, CI and timing runs.
//!
//! Every callback runs with denormals flushed to zero and inside the real-time checks, in blocks of at most
//! `MAX_BLOCK_FRAMES`. Each stream counts dropouts (callbacks that overran their deadline, or that the device
//! reported late), measures load and reports latency through [`AudioStream::status`]. A device that disappears
//! marks its stream disconnected; the project is unaffected and the stream can be reopened.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use bodo_kit_core::SampleRate;
use bodo_kit_core::pool::ThreadGroup;

mod driver;
#[cfg(windows)]
mod exclusive;
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod ffi;
mod hardware;
mod virtual_device;

pub use driver::{AudioCallback, Block, StreamStatus};
pub use hardware::{DeviceInfo, HostInfo, devices, hosts};
pub use virtual_device::{Pacing, VirtualConfig, VirtualDevice};

use driver::{Stats, StreamStatus as Status};

/// Which device to open and how.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(rename_all = "camelCase"))]
pub struct DeviceConfig {
    /// The host (driver API) by name, or `None` for the platform default.
    pub host: Option<String>,
    /// The output device by name, or `None` for the host's default.
    pub output: Option<String>,
    /// The input device by name, or `None` for the host's default. Only used when `input_map` isn't empty.
    pub input: Option<String>,
    /// The sample rate.
    pub sample_rate: SampleRate,
    /// Frames per callback, from 32 to 4096, or `None` for the device's default.
    pub buffer_frames: Option<u32>,
    /// The device channel each engine output channel plays on: `[0, 1]` is stereo on the first two outputs.
    pub output_map: Vec<usize>,
    /// The device channel each engine input channel records from; empty for no input.
    pub input_map: Vec<usize>,
}

impl Default for DeviceConfig {
    /// The default devices at 48 kHz with 256-frame buffers, stereo out, no input.
    fn default() -> Self {
        Self {
            host: None,
            output: None,
            input: None,
            sample_rate: SampleRate::HZ_48000,
            buffer_frames: Some(256),
            output_map: vec![0, 1],
            input_map: Vec::new(),
        }
    }
}

/// Why a device could not be listed or opened.
#[derive(Debug, thiserror::Error)]
pub enum DeviceError {
    /// No host has that name on this machine.
    #[error("no audio host named {0}")]
    NoHost(String),
    /// The host has no default device.
    #[error("no audio device is available")]
    NoDevice,
    /// No device has that name.
    #[error("no audio device named {0}")]
    NoSuchDevice(String),
    /// Another program holds the device exclusively.
    #[error("the audio device is busy")]
    Busy,
    /// The device went away.
    #[error("the audio device was disconnected")]
    Disconnected,
    /// The device can't do what was asked.
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// The driver reported an error.
    #[error("audio driver error: {0}")]
    Backend(String),
    /// A thread could not be started.
    #[error("could not start the audio thread")]
    Thread(#[source] std::io::Error),
}

/// A running stream. Dropping it stops the stream; drop it off the audio thread.
pub struct AudioStream {
    stats: Arc<Stats>,
    input_stats: Option<Arc<Stats>>,
    inner: StreamInner,
    rate: SampleRate,
    thread_group: Option<Arc<dyn ThreadGroup>>,
}

pub(crate) enum StreamInner {
    /// Boxed: cpal's streams are large on some platforms.
    Hardware { _streams: Box<HardwareStreams> },
    /// A stream driven by a thread of our own: the virtual device, or WASAPI exclusive mode.
    Thread { stop: Arc<AtomicBool>, handle: Option<JoinHandle<()>> },
}

/// The cpal streams of a hardware stream, kept alive until the stream is dropped.
pub(crate) struct HardwareStreams {
    pub(crate) _output: cpal::Stream,
    pub(crate) _input: Option<cpal::Stream>,
}

impl std::fmt::Debug for AudioStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioStream").field("rate", &self.rate).field("status", &self.status()).finish_non_exhaustive()
    }
}

impl AudioStream {
    /// The stream's counters: dropouts, load, latency, and whether the device went away.
    pub fn status(&self) -> Status {
        let mut status = self.stats.snapshot();
        if let Some(input) = &self.input_stats {
            status.input_latency = input.snapshot().input_latency;
        }
        status
    }

    /// The sample rate the stream runs at.
    pub fn rate(&self) -> SampleRate {
        self.rate
    }

    /// The group of threads the OS schedules with the device's audio thread, which RT workers should join: the
    /// device's IO workgroup on macOS, `None` elsewhere.
    pub fn thread_group(&self) -> Option<Arc<dyn ThreadGroup>> {
        self.thread_group.clone()
    }
}

impl Drop for AudioStream {
    fn drop(&mut self) {
        if let StreamInner::Thread { stop, handle } = &mut self.inner {
            stop.store(true, Ordering::Relaxed);
            if let Some(handle) = handle.take() {
                let _ = handle.join();
            }
        }
    }
}

/// Opens a real device and starts calling `callback` from the audio thread.
///
/// Talks to the OS and allocates the stream's buffers.
pub fn open<C: AudioCallback>(config: &DeviceConfig, callback: C) -> Result<AudioStream, DeviceError> {
    if config.output_map.is_empty() {
        return Err(DeviceError::Unsupported(String::from("at least one output channel is needed")));
    }
    hardware::open(config, callback)
}

/// Starts a virtual device on its own thread, calling `callback` until the stream is dropped.
///
/// Spawns a thread and allocates the buffers.
pub fn open_virtual<C: AudioCallback>(config: VirtualConfig, callback: C) -> Result<AudioStream, DeviceError> {
    virtual_device::spawn(config, callback)
}
