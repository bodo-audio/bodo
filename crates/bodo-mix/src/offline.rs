//! Rendering a project faster than real time, for export and for the assistant's analysis (project.md section 6.8).
//!
//! It runs exactly the engine that plays live, so a render matches what the user hears. It is deterministic: the
//! same project gives bit-identical output on every run, whatever the block size or number of workers.
//!
//! [`render_stems`] renders every signal that enters the master, each insert and each channel that plays straight
//! into the master, to its own audio in the same pass, lined up as the master hears them. With the master strip at its
//! defaults, the stems add up to the mix.

use bodo_kit_audio::AudioData;
use bodo_kit_core::{Seconds, Tick};
use bodo_kit_dsp::AudioBuffer;
use bodo_model::mixer::RouteSource;
use bodo_model::{Project, SampleRoots};

use crate::MixError;
use crate::engine::{AudioEngine, Engine, EngineConfig, build, engine};
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

/// One stem: a signal entering the master.
#[derive(Clone, Debug, PartialEq)]
pub struct Stem {
    /// The insert, or the channel playing straight into the master.
    pub source: RouteSource,
    /// Its name.
    pub name: String,
    /// The audio, stereo at the project's sample rate, as long as the mix.
    pub audio: AudioData,
}

/// Starts an engine for `options` and returns its two sides, with the number of frames to render.
fn start(project: &Project, roots: &SampleRoots, options: &OfflineOptions, stems: bool) -> Result<(Engine, AudioEngine, usize), MixError> {
    let rate = project.settings.sample_rate;
    let block = options.block.max(1);
    let config = EngineConfig { workers: options.workers, max_frames: block, realtime_buffer: None };
    let (mut control, audio) = if stems { build(project, roots, config, true)? } else { engine(project, roots, config)? };
    control.set_mode(options.mode);
    control.seek(options.start);
    control.play();
    let song = SongIndex::build(project);
    let start = frame_at(&song, options.start, rate).0;
    let end = frame_at(&song, options.end, rate).0 + rate.seconds_to_frames(options.tail).0;
    Ok((control, audio, usize::try_from(end - start).unwrap_or(0)))
}

/// How far a render has come, shared with the thread that waits for it, and a way to stop it.
#[derive(Debug, Default)]
pub struct RenderProgress {
    done: std::sync::atomic::AtomicU64,
    total: std::sync::atomic::AtomicU64,
    cancel: std::sync::atomic::AtomicBool,
}

impl RenderProgress {
    /// The share rendered so far, from 0 to 1; 0 until the render knows its length.
    pub fn fraction(&self) -> f32 {
        use std::sync::atomic::Ordering;
        let total = self.total.load(Ordering::Relaxed);
        if total == 0 { 0.0 } else { (self.done.load(Ordering::Relaxed) as f64 / total as f64).clamp(0.0, 1.0) as f32 }
    }

    /// Asks the render to stop at its next block; it then fails with [`MixError::Cancelled`].
    pub fn cancel(&self) {
        self.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// Whether the render was asked to stop.
    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Starts counting a render of `total` frames, or a further pass of that many.
    fn begin(&self, total: usize) {
        use std::sync::atomic::Ordering;
        self.total.fetch_add(total as u64, Ordering::Relaxed);
    }

    /// Counts `frames` more rendered; fails when the render was cancelled.
    fn advance(&self, frames: usize) -> Result<(), MixError> {
        self.done.fetch_add(frames as u64, std::sync::atomic::Ordering::Relaxed);
        if self.is_cancelled() { Err(MixError::Cancelled) } else { Ok(()) }
    }
}

/// Renders `project` to stereo audio at its sample rate. Sample paths resolve through `roots`.
///
/// Allocates the output. Complexity: proportional to the rendered length times the engine's cost per block.
pub fn render_offline(project: &Project, roots: &SampleRoots, options: &OfflineOptions) -> Result<AudioData, MixError> {
    render_offline_with(project, roots, options, &RenderProgress::default())
}

/// [`render_offline`], counting its progress in `progress` and stopping when it is cancelled.
///
/// Allocates the output. Complexity: as [`render_offline`].
pub fn render_offline_with(
    project: &Project,
    roots: &SampleRoots,
    options: &OfflineOptions,
    progress: &RenderProgress,
) -> Result<AudioData, MixError> {
    let rate = project.settings.sample_rate;
    let block = options.block.max(1);
    let (_control, mut audio, total) = start(project, roots, options, false)?;
    progress.begin(total);
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
        progress.advance(frames)?;
    }
    if audio.faults() > 0 {
        return Err(MixError::Faults(audio.faults()));
    }
    Ok(AudioData { rate, channels })
}

/// Renders `project` as stems: every insert that feeds the master, and every channel that plays straight into it,
/// each to its own stereo audio, over the same range as [`render_offline`]. Each stem is delayed as delay compensation
/// delays it on its way into the master, so the stems line up with each other and with the mix.
///
/// Allocates the output. Complexity: proportional to the rendered length times the engine's cost per block, plus the
/// stems.
pub fn render_stems(project: &Project, roots: &SampleRoots, options: &OfflineOptions) -> Result<Vec<Stem>, MixError> {
    render_stems_with(project, roots, options, &RenderProgress::default())
}

/// [`render_stems`], counting its progress in `progress` and stopping when it is cancelled.
///
/// Allocates the output. Complexity: as [`render_stems`].
pub fn render_stems_with(project: &Project, roots: &SampleRoots, options: &OfflineOptions, progress: &RenderProgress) -> Result<Vec<Stem>, MixError> {
    let rate = project.settings.sample_rate;
    let block = options.block.max(1);
    let (control, mut audio, total) = start(project, roots, options, true)?;
    progress.begin(total);
    let sources = control.stems();
    let mut channels: Vec<[Vec<f32>; 2]> = sources.iter().map(|_| [Vec::with_capacity(total), Vec::with_capacity(total)]).collect();
    let mut buffer = AudioBuffer::new(2, block);
    let mut rendered = 0;
    while rendered < total {
        let frames = block.min(total - rendered);
        audio.render(frames, &mut buffer);
        audio.read_stems(|index, stem| {
            if let Some([left, right]) = channels.get_mut(index) {
                for (out, side) in [(left, 0), (right, 1)] {
                    let source = stem.channel(side.min(stem.channels().saturating_sub(1)));
                    out.extend_from_slice(source.get(..frames).unwrap_or(&[]));
                    out.resize(rendered + frames, 0.0);
                }
            }
        });
        rendered += frames;
        progress.advance(frames)?;
    }
    if audio.faults() > 0 {
        return Err(MixError::Faults(audio.faults()));
    }
    Ok(sources
        .into_iter()
        .zip(channels)
        .map(|((source, delay), [left, right])| {
            // The master hears each stem delayed by its edge's compensation; so do the stems.
            let shift = |mut samples: Vec<f32>| {
                samples.resize(total, 0.0);
                let delay = (delay as usize).min(total);
                samples.rotate_right(delay);
                samples[..delay].fill(0.0);
                samples
            };
            Stem { source: source.source, name: source.name, audio: AudioData { rate, channels: vec![shift(left), shift(right)] } }
        })
        .collect())
}
