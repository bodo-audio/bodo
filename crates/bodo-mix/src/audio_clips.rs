//! Audio clips on the playlist (project.md section 6.7): which piece of which clip each block plays.
//!
//! The control thread flattens every unmuted audio clip on an unmuted track into a list per track, sorted by start.
//! For each block, the audio thread finds the clips that overlap it by binary search, as the sequencer does for
//! pattern clips, and pushes one [`AudioSegment`] per overlap for the clip's channel to render.
//!
//! How a clip follows the tempo depends on its channel's [`StretchMode`]:
//! - `None` plays at the file's own speed from the clip's start;
//! - `Resample` plays faster or slower, pitch and all, so the sample's tempo matches the project's at the clip's start;
//! - `Stretch` plays audio time-stretched to that tempo ahead of time, keeping the pitch.

use bodo_kit_core::{PPQ, Seconds, TempoMap, Tick, rt_safe};
use bodo_model::channel::{ChannelKind, StretchMode};
use bodo_model::playlist::{ClipOffset, ClipSource};
use bodo_model::{ChannelId, Clip, Project};

use crate::context::BlockContext;
use crate::keys;
use crate::segments::AudioSegment;

/// A stretch ratio as a key: ratios that agree to four decimal places share stretched audio.
pub fn ratio_key(ratio: f64) -> u32 {
    (ratio.clamp(0.01, 100.0) * 10_000.0).round() as u32
}

/// The ratio a key stands for.
pub fn key_ratio(key: u32) -> f64 {
    f64::from(key) / 10_000.0
}

/// How a channel's clips follow the tempo: the mode, and the sample's own tempo in BPM, if it has one.
fn channel_timing(project: &Project, channel: ChannelId) -> Option<(StretchMode, Option<f64>)> {
    match &project.channels.get(&channel)?.kind {
        ChannelKind::AudioClip { stretch, tempo, .. } => Some((*stretch, *tempo)),
        _ => None,
    }
}

/// How fast clip `clip` of a channel plays its sample: the project's tempo at the clip's start over the sample's. 1
/// for clips that play at the file's own speed, or whose sample has no tempo.
pub fn clip_ratio(project: &Project, mode: StretchMode, sample_tempo: Option<f64>, clip: &Clip) -> f64 {
    match (mode, sample_tempo) {
        (StretchMode::None, _) | (_, None) => 1.0,
        (_, Some(tempo)) if tempo > 0.0 => key_ratio(ratio_key(project.tempo.bpm_at(clip.start) / tempo)),
        _ => 1.0,
    }
}

/// The stretch ratios channel `channel`'s clips need, as keys, sorted and without repeats.
///
/// Complexity: O(n log n) in the clips on the playlist.
pub fn stretch_keys(project: &Project, channel: ChannelId) -> Vec<u32> {
    let Some((StretchMode::Stretch, tempo)) = channel_timing(project, channel) else { return Vec::new() };
    let mut keys: Vec<u32> = project
        .tracks
        .values()
        .flat_map(|t| t.clips.iter())
        .filter(|c| c.source == ClipSource::Audio(channel))
        .map(|c| ratio_key(clip_ratio(project, StretchMode::Stretch, tempo, c)))
        .collect();
    keys.sort_unstable();
    keys.dedup();
    keys
}

/// An audio clip, flattened.
#[derive(Clone, Copy, Debug, PartialEq)]
struct IndexedAudioClip {
    start: i64,
    end: i64,
    channel: u64,
    mode: StretchMode,
    /// The sample's tempo, for converting tick offsets in tempo-following modes.
    sample_tempo: f64,
    ratio: f64,
    offset: ClipOffset,
    fade_in: i64,
    fade_out: i64,
}

/// The audio clips of one track, sorted by start.
#[derive(Clone, Debug, Default, PartialEq)]
struct AudioTrack {
    clips: Vec<IndexedAudioClip>,
    max_len: i64,
}

/// Every audio clip of the playlist, flat and sorted, built off the audio thread.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioIndex {
    tracks: Vec<AudioTrack>,
}

impl AudioIndex {
    /// Flattens `project`'s audio clips. Muted clips and tracks are left out.
    ///
    /// Allocates. Complexity: O(n log n) in clips.
    pub fn build(project: &Project) -> Self {
        let solo = bodo_model::playlist::any_solo(project.tracks.values());
        let tracks = project
            .track_order
            .iter()
            .filter_map(|id| project.tracks.get(id))
            .filter(|track| track.plays(solo))
            .map(|track| {
                let clips: Vec<IndexedAudioClip> = track
                    .clips
                    .iter()
                    .filter(|clip| !clip.muted)
                    .filter_map(|clip| {
                        let ClipSource::Audio(channel) = clip.source else { return None };
                        let (mode, tempo) = channel_timing(project, channel)?;
                        let sample_tempo = tempo.filter(|t| *t > 0.0).unwrap_or_else(|| project.tempo.bpm_at(clip.start));
                        let fades = clip.fades.unwrap_or_default();
                        Some(IndexedAudioClip {
                            start: clip.start.0,
                            end: clip.end().0,
                            channel: keys::channel(channel),
                            mode,
                            sample_tempo,
                            ratio: clip_ratio(project, mode, tempo, clip),
                            offset: clip.offset,
                            fade_in: fades.fade_in.0.clamp(0, clip.len.0),
                            fade_out: fades.fade_out.0.clamp(0, clip.len.0),
                        })
                    })
                    .collect();
                let max_len = clips.iter().map(|c| c.end - c.start).max().unwrap_or(0);
                AudioTrack { clips, max_len }
            })
            .filter(|track| !track.clips.is_empty())
            .collect();
        Self { tracks }
    }

    /// Pushes the segments of the block from frame `first` to `end` (exclusive) at `hz` frames per second.
    ///
    /// RT-safe. Complexity: O(T log c + k) in tracks, clips per track and clips overlapping the block.
    #[rt_safe]
    pub fn emit(&self, tempo: &TempoMap, first: i64, end: i64, hz: f64, context: &mut BlockContext) {
        if self.tracks.is_empty() || end <= first {
            return;
        }
        let frame = |tick: i64| (tempo.seconds_at(Tick(tick)).0 * hz).round() as i64;
        let t0 = tempo.ticks_at(Seconds((first - 1) as f64 / hz));
        let t1 = tempo.ticks_at(Seconds((end + 1) as f64 / hz));
        for track in &self.tracks {
            let earliest = t0 - track.max_len as f64;
            let from = track.clips.partition_point(|c| (c.start as f64) <= earliest);
            for clip in track.clips[from..].iter().take_while(|c| (c.start as f64) < t1) {
                if (clip.end as f64) <= t0 {
                    continue;
                }
                let (start_frame, end_frame) = (frame(clip.start), frame(clip.end));
                let (from_frame, to_frame) = (start_frame.max(first), end_frame.min(end));
                if from_frame >= to_frame {
                    continue;
                }
                let elapsed = (from_frame - start_frame) as f64;
                // The offset in frames of the sample, at its own speed.
                let offset = match (clip.offset, clip.mode) {
                    (ClipOffset::Frames(f), _) => f.0 as f64,
                    (ClipOffset::Ticks(t), StretchMode::None) => {
                        (tempo.seconds_at(Tick(clip.start + t.0)).0 - tempo.seconds_at(Tick(clip.start)).0) * hz
                    }
                    (ClipOffset::Ticks(t), _) => t.0 as f64 / PPQ as f64 * 60.0 / clip.sample_tempo * hz,
                };
                let (position, step, stretched) = match clip.mode {
                    StretchMode::None => (offset + elapsed, 1.0, None),
                    StretchMode::Resample => (offset + elapsed * clip.ratio, clip.ratio, None),
                    StretchMode::Stretch => (offset / clip.ratio + elapsed, 1.0, Some(ratio_key(clip.ratio))),
                };
                let fade_in = if clip.fade_in > 0 { (frame(clip.start + clip.fade_in) - start_frame) as f64 } else { 0.0 };
                let fade_out = if clip.fade_out > 0 { (end_frame - frame(clip.end - clip.fade_out)) as f64 } else { 0.0 };
                context.audio.push(
                    clip.channel,
                    AudioSegment {
                        frame: (from_frame - first) as u32,
                        frames: (to_frame - from_frame) as u32,
                        position,
                        step,
                        stretched,
                        ratio: clip.ratio,
                        clip_position: elapsed,
                        clip_frames: (end_frame - start_frame) as f64,
                        fade_in,
                        fade_out,
                    },
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratio_keys_round_to_four_decimals() {
        assert_eq!(ratio_key(1.0), 10_000);
        assert_eq!(ratio_key(1.000_04), 10_000);
        assert_eq!(key_ratio(ratio_key(1.25)), 1.25);
        assert_eq!(ratio_key(1e9), 1_000_000, "absurd ratios are clamped");
    }
}
