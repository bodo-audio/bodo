//! Rendering a project faster than real time, for export and for the assistant's analysis (project.md section 6.8).
//!
//! It runs exactly the engine that plays live, so a render matches what the user hears. It is deterministic: the
//! same project gives bit-identical output on every run, whatever the block size or number of workers.

use std::path::Path;

use bodo_kit_audio::AudioData;
use bodo_kit_core::{Seconds, Tick};
use bodo_kit_dsp::AudioBuffer;
use bodo_model::Project;

use crate::MixError;
use crate::engine::{EngineConfig, engine};
use crate::sequence::{PlayMode, SongIndex, frame_at};

/// What to render.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OfflineOptions {
    /// Where to start.
    pub start: Tick,
    /// Where to stop, before the tail.
    pub end: Tick,
    /// Extra time after `end`, so releases and echoes can ring out.
    pub tail: Seconds,
    /// What to play.
    pub mode: PlayMode,
    /// Frames per block.
    pub block: usize,
    /// RT worker threads besides the rendering thread.
    pub workers: usize,
}

impl Default for OfflineOptions {
    fn default() -> Self {
        Self { start: Tick::ZERO, end: Tick::from_quarters(16), tail: Seconds(1.0), mode: PlayMode::Song, block: 512, workers: 0 }
    }
}

/// Renders `project` to stereo audio at its sample rate. Sample paths are relative to `base`.
///
/// Allocates the output. Complexity: proportional to the rendered length times the engine's cost per block.
pub fn render_offline(project: &Project, base: &Path, options: &OfflineOptions) -> Result<AudioData, MixError> {
    let rate = project.settings.sample_rate;
    let block = options.block.max(1);
    let (mut control, mut audio) = engine(project, base, EngineConfig { workers: options.workers, max_frames: block, realtime_buffer: None })?;
    control.set_mode(options.mode);
    control.seek(options.start);
    control.play();
    let song = SongIndex::build(project);
    let start = frame_at(&song, options.start, rate).0;
    let end = frame_at(&song, options.end, rate).0 + rate.seconds_to_frames(options.tail).0;
    let total = usize::try_from(end - start).unwrap_or(0);
    let mut channels = vec![Vec::with_capacity(total), Vec::with_capacity(total)];
    let mut buffer = AudioBuffer::new(2, block);
    let mut rendered = 0;
    while rendered < total {
        let frames = block.min(total - rendered);
        audio.render(frames, &mut buffer);
        for (out, channel) in channels.iter_mut().enumerate() {
            channel.extend_from_slice(&buffer.channel(out)[..frames]);
        }
        rendered += frames;
    }
    if audio.faults() > 0 {
        return Err(MixError::Faults(audio.faults()));
    }
    Ok(AudioData { rate, channels })
}
