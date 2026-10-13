//! `render.*`: the song, or one pattern, rendered offline to the mix or to one file per insert (project.md sections
//! 6.8 and 12.2).
//!
//! Each stem is what its insert sends to the master, delay compensated, so the stems summed with the master at its
//! defaults give the mix. Channels that play straight into the master get stems of their own.

use std::path::{Path, PathBuf};

use bodo_kit_audio::{AudioData, AudioError, FlacWriter, W64Format, W64Writer, WavFormat, WavWriter};
use bodo_kit_core::{Seconds, Tick};
use bodo_mix::{OfflineOptions, PlayMode, RenderProgress, render_offline_with, render_stems_with};
use bodo_model::{Project, SampleRoots};
use serde::Deserialize;
use serde_json::{Value, json};

use super::pattern_length;
use crate::error::CommandError;
use crate::params::{PatternRef, Position};
use crate::registry::{Effects, Registry};
use crate::session::Ctx;

/// The file format of a render.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
enum Format {
    /// 32-bit float WAV.
    #[default]
    Wav,
    /// 24-bit FLAC.
    Flac,
    /// 32-bit float Wave64, for files past 4 GB.
    W64,
}

impl Format {
    fn extension(self) -> &'static str {
        match self {
            Self::Wav => "wav",
            Self::Flac => "flac",
            Self::W64 => "w64",
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExportMix {
    /// The file to write: an absolute path, or one relative to the project's folder. The format's extension is added
    /// when it has none.
    path: String,
    /// Loop this pattern instead of playing the playlist.
    pattern: Option<PatternRef>,
    /// Where to start; the start of the song or pattern by default.
    start: Option<Position>,
    /// Where to stop; the end of the song or of one pass of the pattern by default. One second of tail follows.
    end: Option<Position>,
    /// The file format; WAV by default.
    #[serde(default)]
    format: Format,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExportStems {
    /// The folder the stems go in: an absolute path, or one relative to the project's folder. It is made when needed.
    folder: String,
    /// Loop this pattern instead of playing the playlist.
    pattern: Option<PatternRef>,
    /// Where to start; the start of the song or pattern by default.
    start: Option<Position>,
    /// Where to stop; the end of the song or of one pass of the pattern by default. One second of tail follows.
    end: Option<Position>,
    /// The file format; WAV by default.
    #[serde(default)]
    format: Format,
    /// Also write the whole mix, through the master, as `Mix`.
    #[serde(default)]
    mix: bool,
}

/// `name` made safe as a file name: path separators and other troublesome characters become dashes.
fn file_name(name: &str) -> String {
    let safe: String = name.trim().chars().map(|c| if c.is_control() || "/\\:*?\"<>|".contains(c) { '-' } else { c }).collect();
    if safe.is_empty() || safe.starts_with('.') { format!("Stem{safe}") } else { safe }
}

/// Writes `audio` to `path` in `format`.
///
/// Does I/O. Complexity: O(channels * frames).
fn write(audio: &AudioData, path: &Path, format: Format) -> Result<(), AudioError> {
    let channels: Vec<&[f32]> = audio.channels.iter().map(Vec::as_slice).collect();
    let (rate, count, frames) = (audio.rate, channels.len(), audio.frames());
    match format {
        Format::Wav => {
            let mut writer = WavWriter::create(path, rate, count, WavFormat::Float32)?;
            writer.write(&channels, frames)?;
            writer.finish()
        }
        Format::Flac => {
            let mut writer = FlacWriter::create(path, rate, count)?;
            writer.write(&channels, frames)?;
            writer.finish()
        }
        Format::W64 => {
            let mut writer = W64Writer::create(path, rate, count, W64Format::Float32)?;
            writer.write(&channels, frames)?;
            writer.finish()
        }
    }
}

/// The loudest sample of `audio`, in dBFS to a tenth of a dB, or -144 for silence.
fn peak_db(audio: &AudioData) -> f64 {
    let peak = audio.channels.iter().flatten().fold(0.0f32, |peak, s| peak.max(s.abs()));
    if peak > 0.0 { (200.0 * f64::from(peak).log10()).round() / 10.0 } else { -144.0 }
}

/// The length of `audio` in seconds, to a hundredth.
fn seconds(audio: &AudioData) -> f64 {
    (audio.frames() as f64 / f64::from(audio.rate.hz()) * 100.0).round() / 100.0
}

/// `text` as a path: absolute, or relative to the project's folder.
fn locate(ctx: &Ctx<'_>, text: &str) -> PathBuf {
    let path = PathBuf::from(text.trim());
    if path.is_absolute() { path } else { ctx.sample_roots().project.join(path) }
}

/// The offline render of `pattern`, or else of the song, from `start` up to `end`.
fn options(ctx: &Ctx<'_>, pattern: Option<&PatternRef>, start: Option<&Position>, end: Option<&Position>) -> Result<OfflineOptions, CommandError> {
    let start = start.map(|s| s.resolve(&ctx.project)).transpose()?.unwrap_or(Tick::ZERO).max(Tick::ZERO);
    let end = end.map(|e| e.resolve(&ctx.project)).transpose()?;
    let (mode, end) = match pattern {
        Some(r) => {
            let id = ctx.pattern(Some(r))?;
            (PlayMode::Pattern(id), end.unwrap_or_else(|| pattern_length(&ctx.project, id)))
        }
        None => {
            let song_end = ctx.project.tracks.values().flat_map(|t| t.clips.iter().map(|c| c.end())).max().unwrap_or(Tick::ZERO);
            (PlayMode::Song, end.unwrap_or(song_end))
        }
    };
    if end <= start {
        return Err(
            CommandError::rejected("there is nothing to render").with_hint("place clips on the playlist, or render one pattern with `pattern`")
        );
    }
    Ok(OfflineOptions { start, end, tail: Seconds(1.0), mode, block: 512, workers: 0 })
}

fn failed(error: bodo_mix::MixError) -> CommandError {
    CommandError::rejected(format!("the render failed: {error}"))
}

/// Makes `folder` and the folders above it.
fn make_folder(folder: &Path) -> Result<(), CommandError> {
    if folder.as_os_str().is_empty() {
        return Ok(());
    }
    std::fs::create_dir_all(folder).map_err(|e| CommandError::io(folder, e))
}

/// Where a render's files go.
#[derive(Clone, Debug)]
enum Target {
    /// One file of the mix.
    Mix(PathBuf),
    /// A folder of stems, with the mix beside them when set.
    Stems { folder: PathBuf, mix: bool },
}

/// A render, prepared from the session and ready to run: on the session's own thread, as the `render.*` commands
/// do, or on a background thread so the interface keeps working while it renders.
#[derive(Clone, Debug)]
pub struct ExportJob {
    project: Project,
    roots: SampleRoots,
    options: OfflineOptions,
    target: Target,
    format: Format,
}

impl ExportJob {
    /// What the job writes, for a progress line: the mix file, or the stems folder.
    pub fn place(&self) -> &Path {
        match &self.target {
            Target::Mix(path) => path,
            Target::Stems { folder, .. } => folder,
        }
    }

    /// Renders and writes the files, counting progress in `progress` and stopping when it is cancelled. Returns what
    /// the `render.*` command returns.
    ///
    /// Does I/O. Complexity: proportional to the rendered length times the engine's cost per block.
    pub fn run(&self, progress: &RenderProgress) -> Result<Value, CommandError> {
        match &self.target {
            Target::Mix(path) => {
                make_folder(path.parent().unwrap_or(Path::new("")))?;
                let audio = render_offline_with(&self.project, &self.roots, &self.options, progress).map_err(failed)?;
                write(&audio, path, self.format).map_err(|e| CommandError::io(path, e))?;
                Ok(json!({ "path": path.display().to_string(), "seconds": seconds(&audio), "peakDb": peak_db(&audio) }))
            }
            Target::Stems { folder, mix } => {
                make_folder(folder)?;
                let stems = render_stems_with(&self.project, &self.roots, &self.options, progress).map_err(failed)?;
                let extension = self.format.extension();
                let digits = stems.len().to_string().len().max(2);
                let mut files = Vec::with_capacity(stems.len());
                for (n, stem) in stems.iter().enumerate() {
                    let path = folder.join(format!("{:0digits$} {}.{extension}", n + 1, file_name(&stem.name)));
                    write(&stem.audio, &path, self.format).map_err(|e| CommandError::io(&path, e))?;
                    files.push(
                        json!({ "source": stem.source, "name": stem.name, "path": path.display().to_string(), "peakDb": peak_db(&stem.audio) }),
                    );
                }
                let length = stems.first().map_or(0.0, |s| seconds(&s.audio));
                let mix = if *mix {
                    let audio = render_offline_with(&self.project, &self.roots, &self.options, progress).map_err(failed)?;
                    let path = folder.join(format!("Mix.{extension}"));
                    write(&audio, &path, self.format).map_err(|e| CommandError::io(&path, e))?;
                    Some(json!({ "path": path.display().to_string(), "peakDb": peak_db(&audio) }))
                } else {
                    None
                };
                Ok(json!({ "folder": folder.display().to_string(), "stems": files, "mix": mix, "seconds": length }))
            }
        }
    }
}

fn prepare_mix(ctx: &Ctx<'_>, p: ExportMix) -> Result<ExportJob, CommandError> {
    let options = options(ctx, p.pattern.as_ref(), p.start.as_ref(), p.end.as_ref())?;
    let path = locate(ctx, &p.path);
    let path = if path.extension().is_none() { path.with_extension(p.format.extension()) } else { path };
    Ok(ExportJob { project: ctx.project.clone(), roots: ctx.sample_roots(), options, target: Target::Mix(path), format: p.format })
}

fn prepare_stems(ctx: &Ctx<'_>, p: ExportStems) -> Result<ExportJob, CommandError> {
    let options = options(ctx, p.pattern.as_ref(), p.start.as_ref(), p.end.as_ref())?;
    let target = Target::Stems { folder: locate(ctx, &p.folder), mix: p.mix };
    Ok(ExportJob { project: ctx.project.clone(), roots: ctx.sample_roots(), options, target, format: p.format })
}

/// Prepares the render command `id` (`render.export_mix` or `render.export_stems`) with `params` against the
/// session, without rendering: the job runs later, on any thread.
pub(crate) fn prepare(ctx: &Ctx<'_>, id: &str, params: Value) -> Result<ExportJob, CommandError> {
    let invalid = |e: serde_json::Error| CommandError::invalid(e.to_string());
    match id {
        "render.export_mix" => prepare_mix(ctx, serde_json::from_value(params).map_err(invalid)?),
        "render.export_stems" => prepare_stems(ctx, serde_json::from_value(params).map_err(invalid)?),
        other => Err(CommandError::invalid(format!("{other} is not a render command"))),
    }
}

fn export_mix(ctx: &mut Ctx<'_>, p: ExportMix) -> Result<Value, CommandError> {
    prepare_mix(ctx, p)?.run(&RenderProgress::default())
}

fn export_stems(ctx: &mut Ctx<'_>, p: ExportStems) -> Result<Value, CommandError> {
    prepare_stems(ctx, p)?.run(&RenderProgress::default())
}

pub(super) fn register(r: &mut Registry) {
    r.edit("render.export_mix", "Export mix", "Renders the song, or one pattern, through the master to an audio file.", Effects::DISK, export_mix)
        .example(json!({ "path": "Song.wav" }))
        .menu("File/Export mix");
    r.edit(
        "render.export_stems",
        "Export stems",
        "Renders the song, or one pattern, to one audio file per insert in a folder, delay compensated so they line up.",
        Effects::DISK,
        export_stems,
    )
    .example(json!({ "folder": "Stems", "format": "flac" }))
    .menu("File/Export stems");
}

#[cfg(test)]
mod tests {
    use super::file_name;

    #[test]
    fn file_names_lose_separators() {
        assert_eq!(file_name("Drums/Bus"), "Drums-Bus");
        assert_eq!(file_name("  Kick  "), "Kick");
        assert_eq!(file_name(""), "Stem");
        assert_eq!(file_name(".hidden"), "Stem.hidden");
    }
}
