//! Time-stretched copies of samples (project.md section 6.7), rendered ahead of time on the control thread with
//! Signalsmith Stretch, so the audio thread only reads them.

use std::collections::HashMap;
use std::sync::Arc;

use bodo_kit_audio::AudioData;
use signalsmith_stretch::Stretch;

use crate::audio_clips::key_ratio;

/// `data` played `ratio` times as fast at the same pitch: its length divided by `ratio`.
///
/// Allocates. Complexity: O(n log n) in the sample's frames.
pub fn stretch(data: &AudioData, ratio: f64) -> AudioData {
    stretch_and_shift(data, ratio, 0.0)
}

/// `data` played `ratio` times as fast and `semitones` higher, each without changing the other.
///
/// Allocates. Complexity: O(n log n) in the sample's frames.
pub fn stretch_and_shift(data: &AudioData, ratio: f64, semitones: f32) -> AudioData {
    let channels = data.channel_count().max(1);
    let frames = data.frames();
    let ratio = ratio.clamp(0.01, 100.0);
    let out_frames = ((frames as f64) / ratio).round() as usize;
    if frames == 0 || out_frames == 0 || data.channels.is_empty() {
        return AudioData { rate: data.rate, channels: vec![Vec::new(); data.channels.len()] };
    }
    let mut input = vec![0.0f32; frames * channels];
    for (c, channel) in data.channels.iter().enumerate() {
        for (i, sample) in channel.iter().enumerate() {
            if let Some(slot) = input.get_mut(i * channels + c) {
                *slot = *sample;
            }
        }
    }
    let mut output = vec![0.0f32; out_frames * channels];
    let mut stretcher = Stretch::preset_default(channels as u32, data.rate.hz());
    if semitones != 0.0 {
        stretcher.set_transpose_factor_semitones(semitones, None);
    }
    stretcher.exact(&input, &mut output);
    let channels_out = (0..data.channels.len()).map(|c| output.iter().skip(c).step_by(channels).copied().collect()).collect();
    AudioData { rate: data.rate, channels: channels_out }
}

/// Stretched copies, by the sample they come from and the ratio key.
#[derive(Debug, Default)]
pub(crate) struct StretchCache {
    /// Each entry keeps its source alive, so a pointer never names two samples.
    rendered: HashMap<(usize, u32), (Arc<AudioData>, Arc<AudioData>)>,
}

impl StretchCache {
    /// `data` stretched by the ratio `key` stands for, rendered the first time it is asked for.
    ///
    /// Allocates and renders on first use. Complexity: O(n log n) in the sample's frames then, O(1) after.
    pub(crate) fn get(&mut self, data: &Arc<AudioData>, key: u32) -> Arc<AudioData> {
        let entry =
            self.rendered.entry((Arc::as_ptr(data) as usize, key)).or_insert_with(|| (Arc::clone(data), Arc::new(stretch(data, key_ratio(key)))));
        Arc::clone(&entry.1)
    }

    /// Forgets copies nothing plays any more.
    pub(crate) fn trim(&mut self) {
        self.rendered.retain(|_, (_, stretched)| Arc::strong_count(stretched) > 1);
    }
}

#[cfg(test)]
mod tests {
    use bodo_kit_core::SampleRate;

    use super::*;

    /// Zero crossings per second of `samples`, ignoring the edges where the stretcher ramps in and out.
    fn frequency(samples: &[f32], rate: f64) -> f64 {
        let middle = &samples[samples.len() / 8..samples.len() * 7 / 8];
        let crossings = middle.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count();
        crossings as f64 / 2.0 / (middle.len() as f64 / rate)
    }

    #[test]
    fn stretching_changes_the_length_and_keeps_the_pitch() {
        let rate = 48_000.0;
        let tone: Vec<f32> = (0..48_000).map(|i| (2.0 * std::f64::consts::PI * 440.0 * i as f64 / rate).sin() as f32 * 0.5).collect();
        let data = AudioData { rate: SampleRate::HZ_48000, channels: vec![tone] };
        let faster = stretch(&data, 2.0);
        assert_eq!(faster.frames(), 24_000);
        let pitch = frequency(&faster.channels[0], rate);
        assert!((pitch - 440.0).abs() < 20.0, "{pitch} Hz");
        let slower = stretch(&data, 0.5);
        assert_eq!(slower.frames(), 96_000);
        assert!((frequency(&slower.channels[0], rate) - 440.0).abs() < 20.0);
    }

    #[test]
    fn the_cache_renders_once_and_forgets_unused_copies() {
        let data = Arc::new(AudioData { rate: SampleRate::HZ_48000, channels: vec![vec![0.0; 4800]] });
        let mut cache = StretchCache::default();
        let a = cache.get(&data, 20_000);
        let b = cache.get(&data, 20_000);
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(a.frames(), 2400);
        drop((a, b));
        cache.trim();
        assert!(cache.rendered.is_empty());
    }
}
