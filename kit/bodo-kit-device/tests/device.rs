//! Tests for the device layer, mostly through the virtual device so they run without audio hardware.

#![allow(clippy::unwrap_used, clippy::print_stdout, reason = "test helpers outside #[test] functions")]

use std::time::{Duration, Instant};

use bodo_kit_core::{MAX_BLOCK_FRAMES, SampleRate};
use bodo_kit_device::{Block, DeviceConfig, Pacing, VirtualConfig, VirtualDevice, devices, hosts, open, open_virtual};

fn config(buffer_frames: u32) -> VirtualConfig {
    VirtualConfig { buffer_frames, pacing: Pacing::Freewheel, ..VirtualConfig::default() }
}

#[test]
fn renders_interleaved_output_with_continuous_positions() {
    // Left carries the frame position, right its negative.
    let mut device = VirtualDevice::new(config(8), |block: &mut Block<'_>| {
        let start = block.position as f32;
        for (i, s) in block.output.channel_mut(0).iter_mut().enumerate() {
            *s = start + i as f32;
        }
        for (i, s) in block.output.channel_mut(1).iter_mut().enumerate() {
            *s = -(start + i as f32);
        }
    });
    assert_eq!(device.render()[..6], [0.0, -0.0, 1.0, -1.0, 2.0, -2.0]);
    assert_eq!(device.render()[..2], [8.0, -8.0]);
    assert_eq!(device.status().callbacks, 2);
    assert_eq!(device.status().frames, 16);
}

#[test]
fn large_buffers_are_split_into_blocks() {
    let mut device = VirtualDevice::new(config(5000), move |block: &mut Block<'_>| {
        assert!(block.frames <= MAX_BLOCK_FRAMES);
        block.output.clear();
        block.output.channel_mut(0).fill(block.frames as f32);
    });
    let out = device.render();
    assert_eq!(out[0], MAX_BLOCK_FRAMES as f32);
    assert_eq!(out[2 * MAX_BLOCK_FRAMES], (5000 - MAX_BLOCK_FRAMES) as f32);
}

#[test]
fn an_overrunning_callback_counts_as_a_dropout() {
    let rate = SampleRate::HZ_48000;
    let mut slow = true;
    let mut device = VirtualDevice::new(VirtualConfig { sample_rate: rate, ..config(48) }, move |block: &mut Block<'_>| {
        block.output.clear();
        if slow {
            // 48 frames at 48 kHz is a 1 ms deadline; spin for 3 ms.
            let start = Instant::now();
            while start.elapsed() < Duration::from_millis(3) {
                std::hint::spin_loop();
            }
            slow = false;
        }
    });
    device.render();
    let status = device.status();
    assert_eq!(status.xruns, 1);
    assert!(status.load > 1.0, "{status:?}");
    device.render();
    assert_eq!(device.status().xruns, 1, "a fast callback is no dropout");
}

#[test]
fn callbacks_run_with_denormals_flushed() {
    let mut device = VirtualDevice::new(config(4), |block: &mut Block<'_>| {
        let tiny = std::hint::black_box(f32::MIN_POSITIVE);
        block.output.channel_mut(0).fill(tiny * std::hint::black_box(0.5));
        block.output.channel_mut(1).fill(1.0);
    });
    assert_eq!(device.render()[0], 0.0);
}

#[test]
fn virtual_streams_run_on_their_own_thread_until_dropped() {
    let freewheel = open_virtual(config(64), |block: &mut Block<'_>| block.output.clear()).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    let fast = freewheel.status().callbacks;
    drop(freewheel);

    let paced = open_virtual(VirtualConfig { buffer_frames: 480, ..VirtualConfig::default() }, |block: &mut Block<'_>| block.output.clear()).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    let status = paced.status();
    drop(paced);
    // 480 frames at 48 kHz is 10 ms, so about 20 callbacks; allow for a busy machine.
    assert!((5..=40).contains(&status.callbacks), "{status:?}");
    assert!(fast > status.callbacks * 10, "freewheeling ran {fast} callbacks");
}

#[test]
fn listing_devices_works_without_hardware() {
    // CI machines often have no audio device; listing must still not fail outright.
    let hosts = hosts();
    assert!(hosts.iter().filter(|h| h.is_default).count() <= 1);
    if let Ok(list) = devices(None) {
        for device in list {
            assert!(!device.name.is_empty());
        }
    }
}

/// Prints every host and device, so a CI log shows what the machine offers. Needs an audio device.
#[test]
#[ignore = "needs an audio device"]
fn prints_the_devices() {
    for host in hosts() {
        println!("host {} (default: {})", host.name, host.is_default);
        match devices(Some(&host.name)) {
            Ok(list) => list.iter().for_each(|d| println!("  {d:?}")),
            Err(err) => println!("  could not list devices: {err}"),
        }
    }
}

/// Plays a two-second tone through `config` and checks that audio keeps flowing in real time. `BODO_TEST_BUFFER`
/// overrides the buffer size, in frames.
fn play_tone(config: &DeviceConfig) {
    let mut config = config.clone();
    if let Some(frames) = std::env::var("BODO_TEST_BUFFER").ok().and_then(|b| b.parse().ok()) {
        config.buffer_frames = Some(frames);
    }
    let mut phase = 0.0f32;
    let stream = open(&config, move |block: &mut Block<'_>| {
        let step = 440.0 * std::f32::consts::TAU / block.rate.hz() as f32;
        for i in 0..block.frames {
            let sample = phase.sin() * 0.1;
            phase = (phase + step) % std::f32::consts::TAU;
            block.output.channel_mut(0)[i] = sample;
            block.output.channel_mut(1)[i] = sample;
        }
    })
    .unwrap();
    let rate = u64::from(stream.rate().hz());
    std::thread::sleep(Duration::from_secs(1));
    let first = stream.status();
    std::thread::sleep(Duration::from_secs(1));
    let second = stream.status();
    println!("after 1 s: {first:?}");
    println!("after 2 s: {second:?}");
    let latency_ms = second.output_latency.0 as f64 * 1000.0 / rate as f64;
    println!("{:?} at {:?} frames: output latency {latency_ms:.1} ms", config.host, config.buffer_frames);
    assert!(!second.disconnected);
    // In the second second the device must have asked for at least half a second of audio.
    assert!(second.frames - first.frames >= rate / 2, "audio stalled: {first:?} then {second:?}");
}

/// Plays a tone on the default output device, with its default buffer size. Run it by hand, or in the CI job that
/// installs a virtual sound card: `cargo test -p bodo-kit-device -- --ignored`.
#[test]
#[ignore = "needs an audio device"]
fn plays_a_tone_on_the_default_device() {
    play_tone(&DeviceConfig { buffer_frames: None, ..DeviceConfig::default() });
}

/// Plays a tone in WASAPI exclusive mode on the default output device, at the shortest period it allows.
#[cfg(windows)]
#[test]
#[ignore = "needs an audio device"]
fn plays_a_tone_in_wasapi_exclusive_mode() {
    let exclusive = DeviceConfig { host: Some(String::from("WASAPI Exclusive")), buffer_frames: None, ..DeviceConfig::default() };
    assert!(hosts().iter().any(|h| Some(&h.name) == exclusive.host.as_ref()));
    play_tone(&exclusive);
}
