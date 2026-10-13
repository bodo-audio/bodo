//! What a sampler plays (project.md section 10.2): its sample with the precomputed effects, the reverse and any
//! time stretch rendered in, and the key zones its file's markers make.
//!
//! This runs on the control thread. Renders are cached by the source sample and the settings that shape them, so
//! turning a knob that doesn't change the sample (the filter, an envelope, a loop point without a crossfade) never
//! renders it again, and the audio thread only ever reads finished audio.

use std::collections::HashMap;
use std::sync::Arc;

use bodo_kit_audio::{AudioData, SampleMarkers, edit};
use bodo_kit_core::SampleRate;
use bodo_model::channel::{LoopMode, SamplerParams, SamplerStretchMode};

use crate::stretch::stretch_and_shift;

/// Where a key plays: the start and end (exclusive) of a slice or region, in frames of the played sample.
pub type Zone = (f64, f64);

/// What a sampler plays: the sample as rendered, and its key zones (empty when keys transpose the whole sample).
#[derive(Clone, Debug, Default)]
pub struct SamplerData {
    /// The sample with its precomputed effects, at the project's rate unless the sampler doesn't resample.
    pub sample: Option<Arc<AudioData>>,
    /// The slices or regions, one per key from the root key up.
    pub zones: Arc<[Zone]>,
    /// The sample streamed from disk, in place of `sample`, when the sampler keeps it there.
    pub disk: Option<Arc<crate::disk_sample::DiskSample>>,
}

/// Silence below this level is trimmed: -60 dB.
const TRIM_THRESHOLD: f32 = 0.001;

/// Renders what `params` plays from `base`, whose markers (at `base`'s rate) are `markers`. `bpm` is the project's
/// tempo, for a stretch fitted to a length in beats.
///
/// Allocates. Complexity: O(n) in the samples for the edits, O(n log n) for a stretch.
pub fn derive(base: &AudioData, markers: &SampleMarkers, params: &SamplerParams, bpm: f64) -> (AudioData, Vec<Zone>) {
    let mut data = base.clone();
    let mut markers = markers.clone();
    let effects = &params.effects;
    if effects.trim {
        let removed = edit::trim_silence(&mut data, TRIM_THRESHOLD) as u64;
        let len = data.frames() as u64;
        markers.cues = markers.cues.iter().filter_map(|c| c.checked_sub(removed)).filter(|c| *c < len).collect();
        markers.regions = markers.regions.iter().filter_map(|(s, l)| s.checked_sub(removed).map(|s| (s, *l))).filter(|(s, _)| *s < len).collect();
    }
    if effects.remove_dc {
        edit::remove_dc(&mut data);
    }
    if effects.normalize {
        edit::normalize(&mut data);
    }
    if effects.reverse_polarity {
        edit::invert(&mut data);
    }
    if effects.swap_stereo {
        edit::swap_channels(&mut data);
    }
    if effects.fade_stereo {
        edit::fade_stereo(&mut data);
    }
    if params.reverse {
        edit::reverse(&mut data);
        let len = data.frames() as u64;
        markers.cues = markers.cues.iter().map(|c| len.saturating_sub(*c)).collect();
        markers.cues.sort_unstable();
        markers.regions = markers.regions.iter().map(|(s, l)| (len.saturating_sub(s + l), *l)).collect();
        markers.regions.sort_unstable();
    }
    let rate = f64::from(data.rate.hz());
    if effects.fade_in > 0.0 {
        edit::fade_in(&mut data, (f64::from(effects.fade_in) * rate).round() as usize);
    }
    if effects.fade_out > 0.0 {
        edit::fade_out(&mut data, (f64::from(effects.fade_out) * rate).round() as usize);
    }
    if effects.crossfade > 0.0 && params.loop_mode != LoopMode::Off {
        let len = data.frames() as f64;
        let (start, end) = ((f64::from(params.loop_start) * len) as usize, (f64::from(params.loop_end) * len) as usize);
        let frames = (f64::from(effects.crossfade.min(0.5)) * end.saturating_sub(start) as f64) as usize;
        edit::loop_crossfade(&mut data, start, end, frames);
    }
    let mut scale = 1.0;
    if params.stretch.mode == SamplerStretchMode::Stretch && !params.stretch.is_identity() {
        let multiplier = length_multiplier(data.frames() as u64, data.rate, params, bpm);
        if (multiplier - 1.0).abs() > 1e-6 || params.stretch.pitch_cents != 0.0 {
            data = stretch_and_shift(&data, 1.0 / multiplier, params.stretch.pitch_cents / 100.0);
            scale = multiplier;
        }
    }
    let zones = zones(&markers, params, data.frames() as f64, scale);
    (data, zones)
}

/// How many times longer the stretch makes the played part of a sample `frames` long at `rate`: the multiplier, or
/// the fit to a length in beats.
pub fn length_multiplier(frames: u64, rate: SampleRate, params: &SamplerParams, bpm: f64) -> f64 {
    let stretch = &params.stretch;
    match stretch.time_beats {
        Some(beats) if beats > 0.0 && bpm > 0.0 => {
            let part = f64::from((params.end - params.start).clamp(0.0, 1.0)) * frames as f64 / f64::from(rate.hz());
            let target = beats * 60.0 / bpm;
            if part > 0.0 { (target / part).clamp(0.05, 20.0) } else { 1.0 }
        }
        _ => f64::from(stretch.multiplier).clamp(0.05, 20.0),
    }
}

/// The key zones the markers make, in frames of a sample `len` long, after a stretch that scaled time by `scale`.
fn zones(markers: &SampleMarkers, params: &SamplerParams, len: f64, scale: f64) -> Vec<Zone> {
    let at = |frame: u64| (frame as f64 * scale).min(len);
    if params.load_regions && !markers.regions.is_empty() {
        return markers.regions.iter().map(|(s, l)| (at(*s), at(s + l))).filter(|(s, e)| e > s).collect();
    }
    if params.load_slices && !markers.cues.is_empty() {
        let mut bounds: Vec<f64> = markers.cues.iter().map(|c| at(*c)).collect();
        if bounds.first().is_some_and(|b| *b > 0.0) {
            bounds.insert(0, 0.0);
        }
        bounds.push(len);
        return bounds.windows(2).map(|w| (w[0], w[1])).filter(|(s, e)| e > s).collect();
    }
    Vec::new()
}

/// Rendered samples by their source and the settings that shaped them.
#[derive(Debug, Default)]
pub(crate) struct DerivedCache {
    /// Each entry keeps its source alive, so a pointer never names two samples.
    entries: HashMap<(usize, String), (Arc<AudioData>, SamplerData)>,
}

impl DerivedCache {
    /// What `params` plays from `base`, rendering it the first time.
    ///
    /// Allocates and renders on first use. Complexity: O(n log n) in the samples then, O(1) after.
    pub(crate) fn get(&mut self, base: &Arc<AudioData>, markers: &SampleMarkers, params: &SamplerParams, bpm: f64) -> SamplerData {
        let key = (Arc::as_ptr(base) as usize, shape_key(markers, params, bpm));
        if let Some((_, data)) = self.entries.get(&key) {
            return data.clone();
        }
        let untouched =
            params.effects.is_identity() && !params.reverse && (params.stretch.mode == SamplerStretchMode::Resample || params.stretch.is_identity());
        if untouched {
            // Nothing to render: the source itself, which the sample cache already keeps.
            return SamplerData { sample: Some(Arc::clone(base)), zones: zones(markers, params, base.frames() as f64, 1.0).into(), disk: None };
        }
        let (rendered, zones) = derive(base, markers, params, bpm);
        let data = SamplerData { sample: Some(Arc::new(rendered)), zones: zones.into(), disk: None };
        self.entries.insert(key, (Arc::clone(base), data.clone()));
        data
    }

    /// Forgets renders nothing plays any more.
    pub(crate) fn trim(&mut self) {
        self.entries.retain(|_, (_, data)| data.sample.as_ref().is_some_and(|s| Arc::strong_count(s) > 1));
    }
}

/// The settings that decide what the render looks like, flattened into a key.
fn shape_key(markers: &SampleMarkers, params: &SamplerParams, bpm: f64) -> String {
    let crossfade_loop = (params.effects.crossfade > 0.0 && params.loop_mode != LoopMode::Off).then_some((params.loop_start, params.loop_end));
    let stretched = params.stretch.mode == SamplerStretchMode::Stretch && !params.stretch.is_identity();
    let fit = (stretched && params.stretch.time_beats.is_some()).then_some((bpm.to_bits(), params.start, params.end));
    format!(
        "{:?}|{}|{:?}|{:?}|{:?}|{}|{}|{}",
        params.effects,
        params.reverse,
        crossfade_loop,
        stretched.then_some(params.stretch),
        fit,
        params.load_regions,
        params.load_slices,
        markers.cues.len() + markers.regions.len(),
    )
}

#[cfg(test)]
mod tests {
    use bodo_kit_core::SampleRate;
    use bodo_model::channel::SampleEffects;

    use super::*;

    fn ramp(frames: usize) -> AudioData {
        AudioData { rate: SampleRate::HZ_48000, channels: vec![(0..frames).map(|i| i as f32 / frames as f32).collect()] }
    }

    #[test]
    fn slices_cover_the_sample_from_marker_to_marker() {
        let markers = SampleMarkers { cues: vec![100, 400], regions: Vec::new(), loops: Vec::new() };
        let params = SamplerParams { load_slices: true, ..SamplerParams::default() };
        let (_, zones) = derive(&ramp(1000), &markers, &params, 120.0);
        assert_eq!(zones, [(0.0, 100.0), (100.0, 400.0), (400.0, 1000.0)]);
        let regions = SampleMarkers { cues: Vec::new(), regions: vec![(200, 300)], loops: Vec::new() };
        let params = SamplerParams { load_regions: true, ..SamplerParams::default() };
        assert_eq!(derive(&ramp(1000), &regions, &params, 120.0).1, [(200.0, 500.0)]);
    }

    #[test]
    fn reversing_mirrors_the_audio_and_the_slices() {
        let markers = SampleMarkers { cues: vec![100], regions: Vec::new(), loops: Vec::new() };
        let params = SamplerParams { reverse: true, load_slices: true, ..SamplerParams::default() };
        let (data, zones) = derive(&ramp(1000), &markers, &params, 120.0);
        assert_eq!(data.channels[0][0], 0.999);
        assert_eq!(zones, [(0.0, 900.0), (900.0, 1000.0)]);
    }

    #[test]
    fn trimming_moves_the_markers_with_the_audio() {
        let mut audio = ramp(1000);
        audio.channels[0][..50].fill(0.0);
        let markers = SampleMarkers { cues: vec![10, 300], regions: Vec::new(), loops: Vec::new() };
        let params =
            SamplerParams { load_slices: true, effects: SampleEffects { trim: true, ..SampleEffects::default() }, ..SamplerParams::default() };
        let (data, zones) = derive(&audio, &markers, &params, 120.0);
        assert_eq!(data.frames(), 950);
        assert_eq!(zones, [(0.0, 250.0), (250.0, 950.0)]);
    }

    #[test]
    fn a_stretch_fitted_to_beats_sets_the_length() {
        // One second at 48 kHz fitted to 4 beats at 120 BPM: two seconds.
        let tone = AudioData { rate: SampleRate::HZ_48000, channels: vec![(0..48_000).map(|i| (i as f32 * 0.05).sin() * 0.5).collect()] };
        let mut params = SamplerParams::default();
        params.stretch.mode = SamplerStretchMode::Stretch;
        params.stretch.time_beats = Some(4.0);
        let (data, _) = derive(&tone, &SampleMarkers::default(), &params, 120.0);
        assert_eq!(data.frames(), 96_000);
    }

    #[test]
    fn the_cache_shares_renders_and_skips_untouched_samples() {
        let base = Arc::new(ramp(100));
        let mut cache = DerivedCache::default();
        let plain = cache.get(&base, &SampleMarkers::default(), &SamplerParams::default(), 120.0);
        assert!(Arc::ptr_eq(plain.sample.as_ref().unwrap(), &base), "no effects: the source itself");
        let reversed = SamplerParams { reverse: true, ..SamplerParams::default() };
        let a = cache.get(&base, &SampleMarkers::default(), &reversed, 120.0);
        let b = cache.get(&base, &SampleMarkers::default(), &reversed, 120.0);
        assert!(Arc::ptr_eq(a.sample.as_ref().unwrap(), b.sample.as_ref().unwrap()));
        let filtered = SamplerParams { reverse: true, root_key: 40, ..SamplerParams::default() };
        let c = cache.get(&base, &SampleMarkers::default(), &filtered, 120.0);
        assert!(Arc::ptr_eq(a.sample.as_ref().unwrap(), c.sample.as_ref().unwrap()), "the root key doesn't change the render");
    }
}
