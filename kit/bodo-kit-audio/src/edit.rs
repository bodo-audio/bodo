//! Edits rendered into decoded audio: the sampler's precomputed effects (trim, DC removal, normalizing, polarity,
//! channel swap, a stereo fade, reversing, fades and loop crossfades).
//!
//! Each works in place on an [`AudioData`] and runs off the audio thread.

use crate::AudioData;

/// Removes the frames at both ends whose every channel stays below `threshold`. Returns how many frames came off the
/// start. Silence everywhere leaves a single frame.
///
/// Complexity: O(n) in the samples.
pub fn trim_silence(audio: &mut AudioData, threshold: f32) -> usize {
    let frames = audio.frames();
    let loud = |i: usize| audio.channels.iter().any(|c| c[i].abs() >= threshold);
    let first = (0..frames).find(|i| loud(*i)).unwrap_or(0);
    let last = (0..frames).rev().find(|i| loud(*i)).map_or(first + 1, |i| i + 1).max(first + 1).min(frames);
    for channel in &mut audio.channels {
        channel.truncate(last);
        channel.drain(..first);
    }
    first
}

/// Subtracts each channel's mean, removing any DC offset.
///
/// Complexity: O(n) in the samples.
pub fn remove_dc(audio: &mut AudioData) {
    for channel in &mut audio.channels {
        if channel.is_empty() {
            continue;
        }
        let mean = (channel.iter().map(|s| f64::from(*s)).sum::<f64>() / channel.len() as f64) as f32;
        channel.iter_mut().for_each(|s| *s -= mean);
    }
}

/// Scales every channel by the same gain so the loudest sample is at full scale. Silence stays silent.
///
/// Complexity: O(n) in the samples.
pub fn normalize(audio: &mut AudioData) {
    let peak = audio.channels.iter().flatten().fold(0.0f32, |p, s| p.max(s.abs()));
    if peak > 1e-9 {
        let gain = 1.0 / peak;
        audio.channels.iter_mut().flatten().for_each(|s| *s *= gain);
    }
}

/// Inverts the waveform.
///
/// Complexity: O(n) in the samples.
pub fn invert(audio: &mut AudioData) {
    audio.channels.iter_mut().flatten().for_each(|s| *s = -*s);
}

/// Swaps the first two channels; a mono sample is left as it is.
///
/// Complexity: O(1).
pub fn swap_channels(audio: &mut AudioData) {
    if audio.channels.len() >= 2 {
        audio.channels.swap(0, 1);
    }
}

/// Turns the audio into a stereo pair whose sound moves from the left to the right over its length, with a
/// constant-power sweep of the channels' sum.
///
/// Allocates the second channel of a mono sample. Complexity: O(n) in the samples.
pub fn fade_stereo(audio: &mut AudioData) {
    let frames = audio.frames();
    if frames == 0 {
        return;
    }
    let count = audio.channels.len() as f32;
    let mono: Vec<f32> = (0..frames).map(|i| audio.channels.iter().map(|c| c[i]).sum::<f32>() / count).collect();
    let span = (frames.max(2) - 1) as f32;
    let mut left = Vec::with_capacity(frames);
    let mut right = Vec::with_capacity(frames);
    for (i, sample) in mono.iter().enumerate() {
        let angle = core::f32::consts::FRAC_PI_2 * i as f32 / span;
        // Constant power: a centered sound keeps the level a mono sum of both sides would have.
        left.push(sample * angle.cos() * core::f32::consts::SQRT_2);
        right.push(sample * angle.sin() * core::f32::consts::SQRT_2);
    }
    audio.channels = vec![left, right];
}

/// Reverses the audio in time.
///
/// Complexity: O(n) in the samples.
pub fn reverse(audio: &mut AudioData) {
    audio.channels.iter_mut().for_each(|c| c.reverse());
}

/// The gain of a raised-cosine fade at `t`, from 0 at the start to 1 at the end.
fn ease(t: f32) -> f32 {
    0.5 - 0.5 * (core::f32::consts::PI * t.clamp(0.0, 1.0)).cos()
}

/// Fades in over the first `frames` frames.
///
/// Complexity: O(frames) per channel.
pub fn fade_in(audio: &mut AudioData, frames: usize) {
    let frames = frames.min(audio.frames());
    for channel in &mut audio.channels {
        for (i, sample) in channel.iter_mut().take(frames).enumerate() {
            *sample *= ease(i as f32 / frames as f32);
        }
    }
}

/// Fades out over the last `frames` frames, reaching silence on the last one.
///
/// Complexity: O(frames) per channel.
pub fn fade_out(audio: &mut AudioData, frames: usize) {
    let total = audio.frames();
    let frames = frames.min(total);
    for channel in &mut audio.channels {
        for i in 0..frames {
            let at = total - frames + i;
            channel[at] *= ease(1.0 - (i + 1) as f32 / frames as f32);
        }
    }
}

/// Smooths a loop from `start` to `end` (exclusive): the last `frames` frames before `end` crossfade into the
/// audio just before `start`, so the jump back to `start` is seamless. `frames` is limited by the room before `start`
/// and half the loop.
///
/// Complexity: O(frames) per channel.
pub fn loop_crossfade(audio: &mut AudioData, start: usize, end: usize, frames: usize) {
    let end = end.min(audio.frames());
    if end <= start {
        return;
    }
    let frames = frames.min(start).min((end - start) / 2);
    if frames == 0 {
        return;
    }
    for channel in &mut audio.channels {
        for i in 0..frames {
            // Equal-power weights: the loop's own tail fades out while the audio leading into the loop start fades in.
            let t = (i + 1) as f32 / frames as f32;
            let (out_gain, in_gain) = ((core::f32::consts::FRAC_PI_2 * t).cos(), (core::f32::consts::FRAC_PI_2 * t).sin());
            let tail = end - frames + i;
            let lead = start - frames + i;
            channel[tail] = channel[tail] * out_gain + channel[lead] * in_gain;
        }
    }
}

#[cfg(test)]
mod tests {
    use bodo_kit_core::SampleRate;

    use super::*;

    fn audio(channels: Vec<Vec<f32>>) -> AudioData {
        AudioData { rate: SampleRate::HZ_48000, channels }
    }

    #[test]
    fn trimming_cuts_quiet_ends_and_reports_the_start() {
        let mut a = audio(vec![vec![0.0, 0.0001, 0.5, -0.3, 0.0, 0.0]]);
        assert_eq!(trim_silence(&mut a, 0.001), 2);
        assert_eq!(a.channels[0], [0.5, -0.3]);
        let mut silent = audio(vec![vec![0.0; 4]]);
        trim_silence(&mut silent, 0.001);
        assert_eq!(silent.frames(), 1);
    }

    #[test]
    fn dc_removal_normalizing_inverting_and_swapping() {
        let mut a = audio(vec![vec![1.25, 0.75, 1.0], vec![0.1, 0.2, 0.3]]);
        remove_dc(&mut a);
        assert!(a.channels[0].iter().sum::<f32>().abs() < 1e-6);
        normalize(&mut a);
        assert!((a.channels.iter().flatten().fold(0.0f32, |p, s| p.max(s.abs())) - 1.0).abs() < 1e-6);
        let before = a.clone();
        invert(&mut a);
        invert(&mut a);
        assert_eq!(a, before, "inverting twice is a null");
        swap_channels(&mut a);
        assert_eq!(a.channels[0], before.channels[1]);
    }

    #[test]
    fn a_stereo_fade_starts_left_and_ends_right() {
        let mut a = audio(vec![vec![1.0; 101]]);
        fade_stereo(&mut a);
        assert_eq!(a.channel_count(), 2);
        assert!(a.channels[1][0].abs() < 1e-6 && a.channels[0][100].abs() < 1e-5);
        let middle = a.channels[0][50].powi(2) + a.channels[1][50].powi(2);
        assert!((middle - 2.0).abs() < 1e-4, "constant power: {middle}");
    }

    #[test]
    fn fades_reach_silence_at_the_ends() {
        let mut a = audio(vec![vec![1.0; 100]]);
        fade_in(&mut a, 10);
        fade_out(&mut a, 10);
        assert_eq!(a.channels[0][0], 0.0);
        assert!(a.channels[0][99].abs() < 1e-6);
        assert_eq!(a.channels[0][50], 1.0);
        reverse(&mut a);
        assert_eq!(a.channels[0][99], 0.0);
    }

    #[test]
    fn a_crossfaded_loop_end_meets_its_start() {
        let ramp: Vec<f32> = (0..1000).map(|i| i as f32).collect();
        let mut a = audio(vec![ramp]);
        loop_crossfade(&mut a, 400, 800, 100);
        // The last frame of the loop now equals the frame just before the loop start, so the jump back is smooth.
        assert!((a.channels[0][799] - 399.0).abs() < 1e-3, "{}", a.channels[0][799]);
        assert_eq!(a.channels[0][699], 699.0, "frames before the crossfade are untouched");
    }
}
