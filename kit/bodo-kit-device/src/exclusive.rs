//! WASAPI exclusive mode on Windows: low latency with any sound card, no ASIO driver needed.
//!
//! In shared mode Windows mixes every program's audio, which adds tens of milliseconds. In exclusive mode Bodo owns
//! the device and hands it buffers directly, so the latency comes down to the buffer size, much as with an ASIO
//! driver. It appears as its own host, [`HOST`], next to cpal's.
//!
//! WASAPI objects belong to the thread that created them, so a dedicated audio thread opens the device, reports
//! back, and then loops: wait for the device's event, render a buffer through the shared [`Driver`], convert it to
//! the device's sample format and write it. Output only for now; recording in exclusive mode comes with recording.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use bodo_kit_core::{MAX_BLOCK_FRAMES, SampleRate};
use wasapi::{AudioClient, Device, DeviceEnumerator, Direction, SampleType, StreamMode, WasapiError, WaveFormat};

use crate::driver::{AudioCallback, Driver, Stats};
use crate::hardware::DeviceInfo;
use crate::{AudioStream, DeviceConfig, DeviceError, StreamInner};

/// The name of the exclusive-mode host.
pub(crate) const HOST: &str = "WASAPI Exclusive";

/// HRESULTs worth telling apart.
const DEVICE_IN_USE: u32 = 0x8889_000A;
const EXCLUSIVE_NOT_ALLOWED: u32 = 0x8889_000E;
const BUFFER_SIZE_NOT_ALIGNED: u32 = 0x8889_0019;

/// Sample rates a device is checked for.
const RATES: [u32; 6] = [44_100, 48_000, 88_200, 96_000, 176_400, 192_000];

/// The sample formats tried, best first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    Float32,
    Int32,
    Int24In32,
    Int24,
    Int16,
}

impl Format {
    const ALL: [Format; 5] = [Format::Float32, Format::Int32, Format::Int24In32, Format::Int24, Format::Int16];

    fn wave(self, rate: SampleRate, channels: usize) -> WaveFormat {
        let (store, valid, kind) = match self {
            Format::Float32 => (32, 32, SampleType::Float),
            Format::Int32 => (32, 32, SampleType::Int),
            Format::Int24In32 => (32, 24, SampleType::Int),
            Format::Int24 => (24, 24, SampleType::Int),
            Format::Int16 => (16, 16, SampleType::Int),
        };
        WaveFormat::new(store, valid, &kind, rate.hz() as usize, channels, None)
    }

    fn bytes(self) -> usize {
        match self {
            Format::Float32 | Format::Int32 | Format::Int24In32 => 4,
            Format::Int24 => 3,
            Format::Int16 => 2,
        }
    }

    /// Writes `samples` into `out` in this format, little-endian. Integer formats clamp at full scale; floats pass
    /// through and the device clips them.
    ///
    /// RT-safe. Complexity: O(n) in the samples.
    fn encode(self, samples: &[f32], out: &mut [u8]) {
        let width = self.bytes();
        for (sample, bytes) in samples.iter().zip(out.chunks_exact_mut(width)) {
            let clamped = f64::from(sample.clamp(-1.0, 1.0));
            let scaled = |bits: u32| (clamped * (f64::from(1u32 << (bits - 1)) - 1.0)).round() as i32;
            match self {
                Format::Float32 => bytes.copy_from_slice(&sample.to_le_bytes()),
                Format::Int32 => bytes.copy_from_slice(&((clamped * f64::from(i32::MAX)).round() as i32).to_le_bytes()),
                Format::Int24In32 => bytes.copy_from_slice(&(scaled(24) << 8).to_le_bytes()),
                Format::Int24 => bytes.copy_from_slice(&scaled(24).to_le_bytes()[..3]),
                Format::Int16 => bytes.copy_from_slice(&(scaled(16) as i16).to_le_bytes()),
            }
        }
    }
}

fn hresult(err: &WasapiError) -> Option<u32> {
    match err {
        WasapiError::Windows(err) => Some(err.code().0 as u32),
        _ => None,
    }
}

fn error(err: WasapiError) -> DeviceError {
    match hresult(&err) {
        Some(DEVICE_IN_USE) => DeviceError::Busy,
        Some(EXCLUSIVE_NOT_ALLOWED) => {
            DeviceError::Unsupported(String::from("exclusive mode is turned off for this device in the Windows sound settings"))
        }
        _ => match err {
            WasapiError::DeviceNotFound(name) => DeviceError::NoSuchDevice(name),
            err => DeviceError::Backend(err.to_string()),
        },
    }
}

/// The channel counts worth trying on `device`: what the engine needs, then what the device mixes at, if more.
fn channel_counts(device: &Device, needed: usize) -> Vec<usize> {
    let mix = device.get_device_format().map_or(needed, |f| usize::from(f.get_nchannels()));
    if mix > needed { vec![needed, mix] } else { vec![needed] }
}

/// The best format and channel count `client` takes in exclusive mode at `rate`.
fn negotiate(client: &AudioClient, rate: SampleRate, counts: &[usize]) -> Option<(Format, usize, WaveFormat)> {
    counts.iter().find_map(|&channels| {
        Format::ALL
            .iter()
            .find_map(|&format| client.is_supported_exclusive_with_quirks(&format.wave(rate, channels)).ok().map(|wave| (format, channels, wave)))
    })
}

/// Runs `job` on a fresh thread with COM initialized for it, and returns its result.
fn on_com_thread<T: Send + 'static>(job: impl FnOnce() -> Result<T, DeviceError> + Send + 'static) -> Result<T, DeviceError> {
    thread::Builder::new()
        .name(String::from("bodo-wasapi"))
        .spawn(move || {
            let _ = wasapi::initialize_mta();
            job()
        })
        .map_err(DeviceError::Thread)?
        .join()
        .unwrap_or_else(|_| Err(DeviceError::Backend(String::from("the WASAPI thread failed"))))
}

/// The output devices that can run in exclusive mode, with the common rates each accepts.
///
/// Talks to the OS. Complexity: O(d) device queries for d devices, each trying every rate and format.
pub(crate) fn devices() -> Result<Vec<DeviceInfo>, DeviceError> {
    on_com_thread(|| {
        let enumerator = DeviceEnumerator::new().map_err(error)?;
        let default = enumerator.get_default_device(&Direction::Render).ok().and_then(|d| d.get_friendlyname().ok());
        let collection = enumerator.get_device_collection(&Direction::Render).map_err(error)?;
        let mut list = Vec::new();
        for device in &collection {
            let Ok(device) = device else { continue };
            let Ok(name) = device.get_friendlyname() else { continue };
            let Ok(client) = device.get_iaudioclient() else { continue };
            let counts = channel_counts(&device, 2);
            let sample_rates: Vec<u32> =
                RATES.into_iter().filter(|&hz| SampleRate::new(hz).is_some_and(|rate| negotiate(&client, rate, &counts).is_some())).collect();
            if sample_rates.is_empty() {
                continue;
            }
            // The shortest period, in frames at the device's own rate.
            let mix_rate = device.get_device_format().map_or(48_000, |f| f.get_samplespersec());
            let buffer_sizes = client
                .get_device_period()
                .ok()
                .map(|(_, min)| ((min as f64 * f64::from(mix_rate) / 10_000_000.0).ceil() as u32, MAX_BLOCK_FRAMES as u32));
            list.push(DeviceInfo {
                is_default_output: default.as_deref() == Some(name.as_str()),
                is_default_input: false,
                name,
                output_channels: counts.iter().copied().max().and_then(|c| u16::try_from(c).ok()).unwrap_or(2),
                input_channels: 0,
                sample_rates,
                buffer_sizes,
            });
        }
        Ok(list)
    })
}

/// Opens the output device in exclusive mode and starts calling `callback` on a dedicated audio thread.
///
/// Spawns the thread, talks to the OS and allocates the stream's buffers.
pub(crate) fn open<C: AudioCallback>(config: &DeviceConfig, callback: C) -> Result<AudioStream, DeviceError> {
    if !config.input_map.is_empty() {
        return Err(DeviceError::Unsupported(String::from("recording in exclusive mode is not available yet")));
    }
    let needed = config.output_map.iter().copied().max().map_or(2, |m| m + 1).max(2);
    let rate = config.sample_rate;
    let name = config.output.clone();
    let requested = config.buffer_frames;
    let output_map = config.output_map.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let (ready_tx, ready_rx) = mpsc::channel::<Result<Arc<Stats>, DeviceError>>();
    let handle = thread::Builder::new()
        .name(String::from("bodo-wasapi-exclusive"))
        .spawn(move || {
            let _ = wasapi::initialize_mta();
            let opened = (|| {
                let enumerator = DeviceEnumerator::new().map_err(error)?;
                let device = match &name {
                    Some(name) => enumerator.get_device_collection(&Direction::Render).and_then(|c| c.get_device_with_name(name)).map_err(error)?,
                    None => enumerator.get_default_device(&Direction::Render).map_err(error)?,
                };
                let mut client = device.get_iaudioclient().map_err(error)?;
                let (format, channels, wave) = negotiate(&client, rate, &channel_counts(&device, needed))
                    .ok_or_else(|| DeviceError::Unsupported(format!("the device takes no exclusive format at {} Hz", rate.hz())))?;
                let (_, min_period) = client.get_device_period().map_err(error)?;
                let wanted = requested.map_or(min_period, |frames| wasapi::calculate_period_100ns(i64::from(frames), i64::from(rate.hz())));
                // Intel HD Audio devices need buffers in multiples of 128 bytes.
                let period = client.calculate_aligned_period_near(wanted.max(min_period), Some(128), &wave).map_err(error)?;
                if let Err(err) = client.initialize_client(&wave, &Direction::Render, &StreamMode::EventsExclusive { period_hns: period }) {
                    if hresult(&err) != Some(BUFFER_SIZE_NOT_ALIGNED) {
                        return Err(error(err));
                    }
                    // The device wants another size: take the one it proposes, on a fresh client.
                    let frames = client.get_buffer_size().map_err(error)?;
                    let period = wasapi::calculate_period_100ns(i64::from(frames), i64::from(rate.hz()));
                    client = device.get_iaudioclient().map_err(error)?;
                    client.initialize_client(&wave, &Direction::Render, &StreamMode::EventsExclusive { period_hns: period }).map_err(error)?;
                }
                let event = client.set_get_eventhandle().map_err(error)?;
                let render = client.get_audiorenderclient().map_err(error)?;
                let frames = client.get_buffer_size().map_err(error)?;
                Ok((client, event, render, format, channels, frames))
            })();
            let (client, event, render, format, channels, buffer_frames) = match opened {
                Ok(parts) => parts,
                Err(err) => {
                    let _ = ready_tx.send(Err(err));
                    return;
                }
            };
            let mut driver = Driver::new(callback, rate, channels, &output_map, Vec::new(), None);
            let frames = buffer_frames as usize;
            let mut samples = vec![0.0f32; frames * channels];
            let mut bytes = vec![0u8; frames * channels * format.bytes()];
            // Start with a buffer of silence, as WASAPI requires.
            if let Err(err) = render.write_to_device(frames, &bytes, None).and_then(|()| client.start_stream()) {
                let _ = ready_tx.send(Err(error(err)));
                return;
            }
            // With two buffers in flight, a buffer written now plays after the one already queued.
            driver.stats.output_latency.store(buffer_frames, Ordering::Relaxed);
            let _ = ready_tx.send(Ok(Arc::clone(&driver.stats)));
            let _priority = audio_thread_priority::promote_current_thread_to_real_time(buffer_frames, rate.hz())
                .inspect_err(|err| tracing::warn!(%err, "could not promote the exclusive-mode thread to real-time priority"));
            let period = Duration::from_secs_f64(f64::from(buffer_frames) / f64::from(rate.hz()));
            let mut last_wake: Option<Instant> = None;
            while !thread_stop.load(Ordering::Relaxed) {
                // A second without a request means the device is gone.
                if event.wait_for_event(1000).is_err() {
                    driver.stats.disconnected.store(true, Ordering::Relaxed);
                    break;
                }
                // Two buffers are in flight, so waking more than two periods after the last request means the device
                // ran dry.
                let now = Instant::now();
                let late = last_wake.is_some_and(|last| now.duration_since(last) > period * 2);
                last_wake = Some(now);
                driver.render(&mut samples, channels, late);
                format.encode(&samples, &mut bytes);
                if render.write_to_device(frames, &bytes, None).is_err() {
                    driver.stats.disconnected.store(true, Ordering::Relaxed);
                    break;
                }
            }
            let _ = client.stop_stream();
        })
        .map_err(DeviceError::Thread)?;
    match ready_rx.recv() {
        Ok(Ok(stats)) => {
            Ok(AudioStream { stats, input_stats: None, thread_group: None, inner: StreamInner::Thread { stop, handle: Some(handle) }, rate })
        }
        Ok(Err(err)) => {
            let _ = handle.join();
            Err(err)
        }
        Err(_) => {
            let _ = handle.join();
            Err(DeviceError::Backend(String::from("the WASAPI thread stopped")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_every_format() {
        let samples = [1.0f32, -1.0, 0.5];
        let mut out = vec![0u8; 3 * 4];
        Format::Int32.encode(&samples, &mut out);
        assert_eq!(i32::from_le_bytes([out[0], out[1], out[2], out[3]]), i32::MAX);
        assert_eq!(i32::from_le_bytes([out[4], out[5], out[6], out[7]]), -i32::MAX);
        let mut out = vec![0u8; 3 * 2];
        Format::Int16.encode(&samples, &mut out);
        assert_eq!(i16::from_le_bytes([out[2], out[3]]), -32767);
        let mut out = vec![0u8; 3 * 3];
        Format::Int24.encode(&samples, &mut out);
        assert_eq!(&out[0..3], &[0xFF, 0xFF, 0x7F]);
        let mut out = vec![0u8; 3 * 4];
        Format::Int24In32.encode(&samples, &mut out);
        assert_eq!(i32::from_le_bytes([out[8], out[9], out[10], out[11]]) >> 8, 4_194_304);
        let mut out = vec![0u8; 4];
        Format::Float32.encode(&[2.0], &mut out);
        assert_eq!(f32::from_le_bytes([out[0], out[1], out[2], out[3]]), 2.0, "floats pass through unclamped");
    }
}
