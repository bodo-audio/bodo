//! Real audio devices through cpal: CoreAudio, WASAPI and ASIO, ALSA, and others cpal supports.
//!
//! Output and input are separate cpal streams. Input reaches the output callback through a bounded queue, so on a
//! single device, where both share one clock, a duplex stream costs at most one extra buffer of latency. Devices with
//! different clocks would drift; matching them is left to the user's OS (an aggregate device on macOS).

use std::time::Duration;

use bodo_kit_core::MAX_BLOCK_FRAMES;
use bodo_kit_core::queue::spsc;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::driver::{AudioCallback, Driver, Stats};
use crate::{AudioStream, DeviceConfig, DeviceError, HardwareStreams, StreamInner};

/// An audio host (driver API) on this machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostInfo {
    /// The host's name, such as "CoreAudio", "WASAPI", "ASIO" or "ALSA".
    pub name: String,
    /// Whether this is the platform's default host.
    pub is_default: bool,
}

/// An audio device and what it supports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    /// The device's name.
    pub name: String,
    /// The most output channels it offers; zero for input-only devices.
    pub output_channels: u16,
    /// The most input channels it offers; zero for output-only devices.
    pub input_channels: u16,
    /// Common sample rates it supports.
    pub sample_rates: Vec<u32>,
    /// The range of buffer sizes it accepts, in frames, when it says.
    pub buffer_sizes: Option<(u32, u32)>,
    /// Whether this is the host's default output device.
    pub is_default_output: bool,
    /// Whether this is the host's default input device.
    pub is_default_input: bool,
}

/// Sample rates a device is checked for.
const COMMON_RATES: [u32; 8] = [22_050, 32_000, 44_100, 48_000, 88_200, 96_000, 176_400, 192_000];
/// How long cpal may take to open a stream.
const OPEN_TIMEOUT: Duration = Duration::from_secs(5);

/// The audio hosts available here.
pub fn hosts() -> Vec<HostInfo> {
    let default = cpal::default_host().id();
    cpal::available_hosts().into_iter().map(|id| HostInfo { name: id.name().to_owned(), is_default: id == default }).collect()
}

/// The devices of `host`, or of the default host.
///
/// Talks to the OS; may take a while on some hosts.
pub fn devices(host: Option<&str>) -> Result<Vec<DeviceInfo>, DeviceError> {
    let host = find_host(host)?;
    let default_output = host.default_output_device().and_then(|d| device_name(&d));
    let default_input = host.default_input_device().and_then(|d| device_name(&d));
    let mut list = Vec::new();
    for device in host.devices().map_err(cpal_error)? {
        let Some(name) = device_name(&device) else { continue };
        let mut info = DeviceInfo {
            is_default_output: default_output.as_deref() == Some(name.as_str()),
            is_default_input: default_input.as_deref() == Some(name.as_str()),
            name,
            output_channels: 0,
            input_channels: 0,
            sample_rates: Vec::new(),
            buffer_sizes: None,
        };
        let mut ranges = Vec::new();
        if let Ok(configs) = device.supported_output_configs() {
            for range in configs {
                info.output_channels = info.output_channels.max(range.channels());
                ranges.push((range.min_sample_rate(), range.max_sample_rate(), *range.buffer_size()));
            }
        }
        if let Ok(configs) = device.supported_input_configs() {
            for range in configs {
                info.input_channels = info.input_channels.max(range.channels());
                ranges.push((range.min_sample_rate(), range.max_sample_rate(), *range.buffer_size()));
            }
        }
        info.sample_rates = COMMON_RATES.into_iter().filter(|&rate| ranges.iter().any(|&(lo, hi, _)| (lo..=hi).contains(&rate))).collect();
        info.buffer_sizes = ranges.iter().find_map(|&(_, _, size)| match size {
            cpal::SupportedBufferSize::Range { min, max } => Some((min, max)),
            cpal::SupportedBufferSize::Unknown => None,
        });
        list.push(info);
    }
    Ok(list)
}

/// Opens the output (and, if configured, input) device and starts calling `callback`.
///
/// Talks to the OS and allocates the stream's buffers.
pub(crate) fn open<C: AudioCallback>(config: &DeviceConfig, callback: C) -> Result<AudioStream, DeviceError> {
    let host = find_host(config.host.as_deref())?;
    let output = match &config.output {
        Some(name) => find_device(&host, name)?,
        None => host.default_output_device().ok_or(DeviceError::NoDevice)?,
    };
    let device_outputs = config.output_map.iter().copied().max().map_or(0, |m| m + 1).max(1);
    let device_outputs = u16::try_from(device_outputs).map_err(|_| DeviceError::Unsupported(String::from("too many output channels")))?;
    let rate = config.sample_rate;
    let buffer_size = match config.buffer_frames {
        Some(frames) => cpal::BufferSize::Fixed(frames.clamp(32, MAX_BLOCK_FRAMES as u32)),
        None => cpal::BufferSize::Default,
    };
    let stream_config = cpal::StreamConfig { channels: device_outputs, sample_rate: rate.hz(), buffer_size };

    let mut input_stream = None;
    let mut input = None;
    if !config.input_map.is_empty() {
        let device = match &config.input {
            Some(name) => find_device(&host, name)?,
            None => host.default_input_device().ok_or(DeviceError::NoDevice)?,
        };
        let device_inputs = config.input_map.iter().copied().max().map_or(1, |m| m + 1);
        let input_config = cpal::StreamConfig {
            channels: u16::try_from(device_inputs).map_err(|_| DeviceError::Unsupported(String::from("too many input channels")))?,
            sample_rate: rate.hz(),
            buffer_size,
        };
        // Room for several device buffers of input, so jitter between the two callbacks never drops samples.
        let (mut producer, consumer) = spsc::<f32>(MAX_BLOCK_FRAMES * 4 * device_inputs);
        let stats_slot: Arc<Stats> = Arc::default();
        let input_stats = Arc::clone(&stats_slot);
        let stream = device
            .build_input_stream::<f32, _, _>(
                input_config,
                move |data: &[f32], info: &cpal::InputCallbackInfo| {
                    let ts = info.timestamp();
                    let latency = ts.callback.duration_since(ts.capture).as_secs_f64() * f64::from(rate.hz());
                    input_stats.input_latency.store(latency as u32, Ordering::Relaxed);
                    for &sample in data {
                        // Overflow means the output side stalled; dropping input is the only RT-safe choice.
                        producer.push_or_drop(sample);
                    }
                },
                |err| tracing::warn!(%err, "audio input error"),
                Some(OPEN_TIMEOUT),
            )
            .map_err(cpal_error)?;
        input = Some((consumer, device_inputs, (), stats_slot));
        input_stream = Some(stream);
    }

    let (input_queue, input_stats) = match input {
        Some((consumer, channels, _, stats)) => (Some((consumer, channels)), Some(stats)),
        None => (None, None),
    };
    let mut driver = Driver::new(callback, rate, usize::from(device_outputs), &config.output_map, config.input_map.clone(), input_queue);
    let stats = Arc::clone(&driver.stats);
    let error_stats = Arc::clone(&stats);
    let expected = config.buffer_frames.map(|f| Duration::from_secs_f64(f64::from(f) / f64::from(rate.hz())));
    let mut last_callback: Option<cpal::StreamInstant> = None;
    let channels = usize::from(device_outputs);
    let output_stream = output
        .build_output_stream::<f32, _, _>(
            stream_config,
            move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
                let ts = info.timestamp();
                let latency = ts.playback.duration_since(ts.callback).as_secs_f64() * f64::from(rate.hz());
                driver.stats.output_latency.store(latency as u32, Ordering::Relaxed);
                // A gap of more than one and a half buffers since the last callback means the device skipped one.
                let late = match (last_callback, expected) {
                    (Some(last), Some(expected)) => ts.callback.duration_since(last) > expected.mul_f64(1.5),
                    _ => false,
                };
                last_callback = Some(ts.callback);
                driver.render(data, channels, late);
            },
            move |err| {
                if err.kind() == cpal::ErrorKind::DeviceNotAvailable {
                    error_stats.disconnected.store(true, Ordering::Relaxed);
                }
                tracing::warn!(%err, "audio output error");
            },
            Some(OPEN_TIMEOUT),
        )
        .map_err(cpal_error)?;
    output_stream.play().map_err(cpal_error)?;
    if let Some(stream) = &input_stream {
        stream.play().map_err(cpal_error)?;
    }
    // Input latency is measured by the input stream and merged in when the status is read.
    Ok(AudioStream {
        stats,
        input_stats,
        inner: StreamInner::Hardware { _streams: Box::new(HardwareStreams { _output: output_stream, _input: input_stream }) },
        rate,
    })
}

fn find_host(name: Option<&str>) -> Result<cpal::Host, DeviceError> {
    let Some(name) = name else { return Ok(cpal::default_host()) };
    let id =
        cpal::available_hosts().into_iter().find(|id| id.name().eq_ignore_ascii_case(name)).ok_or_else(|| DeviceError::NoHost(name.to_owned()))?;
    cpal::host_from_id(id).map_err(cpal_error)
}

fn find_device(host: &cpal::Host, name: &str) -> Result<cpal::Device, DeviceError> {
    host.devices()
        .map_err(cpal_error)?
        .find(|device| device_name(device).as_deref() == Some(name))
        .ok_or_else(|| DeviceError::NoSuchDevice(name.to_owned()))
}

fn device_name(device: &cpal::Device) -> Option<String> {
    device.description().ok().map(|d| d.name().to_owned())
}

fn cpal_error(err: cpal::Error) -> DeviceError {
    match err.kind() {
        cpal::ErrorKind::DeviceNotAvailable => DeviceError::Disconnected,
        cpal::ErrorKind::DeviceBusy => DeviceError::Busy,
        _ => DeviceError::Backend(err.to_string()),
    }
}
