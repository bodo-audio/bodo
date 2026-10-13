//! `bodo-cli render`: a project's song, or one pattern, to a WAV file through the offline engine
//! (project.md section 6.8). The render is deterministic: the same project gives the same samples.

use std::path::Path;

use anyhow::{Context, Result, bail};
use bodo_engine::params::{PatternRef, Position};
use bodo_engine::{Session, pattern_length};
use bodo_kit_audio::{AudioData, WavFormat, WavWriter};
use bodo_kit_core::{Seconds, Tick};
use bodo_mix::{OfflineOptions, PlayMode, render_offline};

/// What to render.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RenderOptions {
    /// Loop this pattern (by name, number or id) instead of playing the playlist.
    pub pattern: Option<String>,
    /// Where to stop, as `bar:beat:tick` or another position; the end of the song or of one pass of the pattern by
    /// default. One second of tail follows, so sounds ring out.
    pub end: Option<String>,
}

/// What a render produced.
#[derive(Clone, Debug, PartialEq)]
pub struct Rendered {
    /// The audio.
    pub audio: AudioData,
    /// The loudest sample, in dBFS.
    pub peak_db: f64,
}

impl Rendered {
    /// The length in seconds.
    pub fn seconds(&self) -> f64 {
        self.audio.frames() as f64 / f64::from(self.audio.rate.hz())
    }
}

/// The end of the last clip on the playlist.
///
/// Complexity: O(n) in clips.
fn song_end(session: &Session) -> Tick {
    session.project().tracks.values().flat_map(|t| t.clips.iter().map(|c| c.end())).max().unwrap_or(Tick::ZERO)
}

/// Renders the project of `session` as `options` says.
///
/// Does I/O to load samples. Complexity: proportional to the rendered length.
pub fn render_session(session: &Session, options: &RenderOptions) -> Result<Rendered> {
    let project = session.project();
    let end = options.end.as_ref().map(|text| Position::Text(text.clone()).resolve(project)).transpose()?;
    let (mode, end) = match &options.pattern {
        Some(text) => {
            let id = bodo_engine::resolve::pattern(project, &PatternRef::new(text.as_str()))?;
            (PlayMode::Pattern(id), end.unwrap_or_else(|| pattern_length(project, id)))
        }
        None => {
            let end = end.unwrap_or_else(|| song_end(session));
            if end <= Tick::ZERO {
                bail!("the playlist is empty; render a pattern with --pattern");
            }
            (PlayMode::Song, end)
        }
    };
    let options = OfflineOptions { start: Tick::ZERO, end, tail: Seconds(1.0), mode, block: 512, workers: 0 };
    let audio = render_offline(project, &session.sample_roots(), &options).context("the render failed")?;
    let peak = audio.channels.iter().flatten().fold(0.0f32, |peak, s| peak.max(s.abs()));
    let peak_db = if peak > 0.0 { 20.0 * f64::from(peak).log10() } else { f64::NEG_INFINITY };
    Ok(Rendered { audio, peak_db })
}

/// Writes `audio` to `path` as a 32-bit float WAV file.
///
/// Does I/O. Complexity: O(channels * frames).
pub fn write_wav(audio: &AudioData, path: &Path) -> Result<()> {
    let mut writer = WavWriter::create(path, audio.rate, audio.channels.len(), WavFormat::Float32)?;
    let channels: Vec<&[f32]> = audio.channels.iter().map(Vec::as_slice).collect();
    writer.write(&channels, audio.frames())?;
    writer.finish()?;
    Ok(())
}

/// Renders the project at `project` to the WAV file `out`.
///
/// Does I/O. Complexity: proportional to the rendered length.
pub fn render(project: &Path, out: &Path, options: &RenderOptions) -> Result<Rendered> {
    let session = crate::session::open(Some(project))?;
    let rendered = render_session(&session, options)?;
    write_wav(&rendered.audio, out).with_context(|| format!("cannot write {}", out.display()))?;
    Ok(rendered)
}
