//! `project.*`: new, open, save, tempo, meter and information (project.md sections 12.2 and 18.1).
//!
//! New and open refuse to drop unsaved changes unless told to, so an agent can't lose the user's work by accident.

use std::path::PathBuf;
use std::sync::Arc;

use bodo_io::{DEFAULT_BACKUPS, SaveOptions};
use bodo_kit_core::TempoMap;
use bodo_model::MeterMap;
use bodo_model::meter::MeterChange;
use serde::Deserialize;
use serde_json::{Value, json};

use super::whole;
use crate::error::{CommandError, ErrorCode};
use crate::registry::{Effects, Registry};
use crate::session::{Ctx, Session};
use crate::template::Template;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct New {
    /// "basic" (a starter drum kit) or "empty"; basic by default.
    #[serde(default)]
    template: Template,
    /// Drop unsaved changes to the current project; without it, new refuses when there are any.
    #[serde(default)]
    discard_changes: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Open {
    /// The `.bodo` file or `.bodoproj` folder.
    path: String,
    /// Drop unsaved changes to the current project; without it, open refuses when there are any.
    #[serde(default)]
    discard_changes: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Save {
    /// Where to save: a `.bodo` file, or a `.bodoproj` folder for git. The project's file by default. A path without
    /// an extension gets `.bodo`; a relative one starts at the project's folder.
    path: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SaveAs {
    /// Where to save: a `.bodo` file, or a `.bodoproj` folder for git. A path without an extension gets `.bodo`; a
    /// relative one starts at the project's folder.
    path: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetTempo {
    /// The tempo at the start of the song, in beats per minute.
    bpm: f64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetMeter {
    /// Beats per bar, from 1 to 64.
    numerator: u8,
    /// The beat unit: 1, 2, 4, 8, 16 or 32.
    denominator: u8,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetInfo {
    /// The song's title.
    title: Option<String>,
    /// Its author.
    author: Option<String>,
}

/// Where `text` points: absolute, or relative to the project's folder (the current folder for an unsaved project),
/// with `.bodo` added when it has no extension.
fn target_path(session: &Session, text: &str) -> PathBuf {
    let path = PathBuf::from(text.trim());
    let path = if path.is_absolute() { path } else { session.sample_roots().project.join(path) };
    if path.extension().is_none() { path.with_extension("bodo") } else { path }
}

fn io_error(error: &bodo_io::IoError) -> CommandError {
    CommandError::new(ErrorCode::Io, error.to_string())
}

fn refuse_unsaved(session: &Session, discard: bool, action: &str) -> Result<(), CommandError> {
    if session.is_dirty() && !discard {
        return Err(CommandError::rejected(format!("the project has unsaved changes, so {action} would lose them"))
            .with_hint("save first with project.save, or pass discardChanges: true"));
    }
    Ok(())
}

fn summary(session: &Session) -> Value {
    let project = session.project();
    json!({
        "path": session.file().map(|p| p.display().to_string()),
        "title": project.meta.title,
        "channels": project.channels.len(),
        "patterns": project.patterns.len(),
        "tracks": project.tracks.len(),
    })
}

fn new(session: &mut Session, p: New) -> Result<Value, CommandError> {
    refuse_unsaved(session, p.discard_changes, "starting a new project")?;
    session.new_project(p.template);
    Ok(summary(session))
}

fn open(session: &mut Session, p: Open) -> Result<Value, CommandError> {
    refuse_unsaved(session, p.discard_changes, "opening another project")?;
    let path = target_path(session, &p.path);
    let project = bodo_io::load(&path).map_err(|e| io_error(&e))?;
    session.replace(project, Some(path), true);
    Ok(summary(session))
}

/// Saves to `path`, or to the project's file.
pub(crate) fn save_to(session: &mut Session, path: Option<PathBuf>) -> Result<Value, CommandError> {
    let path = path
        .or_else(|| session.file().map(PathBuf::from))
        .ok_or_else(|| CommandError::invalid("the project has no file yet").with_hint("pass path, such as \"My Song.bodo\" or an absolute path"))?;
    let options = SaveOptions { from: session.file().map(PathBuf::from), backups: DEFAULT_BACKUPS };
    let saved = bodo_io::save(session.project(), &path, &options).map_err(|e| io_error(&e))?;
    session.mark_saved(saved, path.clone());
    Ok(json!({ "path": path.display().to_string() }))
}

fn set_tempo(ctx: &mut Ctx<'_>, p: SetTempo) -> Result<Value, CommandError> {
    let mut points = ctx.project.tempo.points().to_vec();
    if let Some(first) = points.first_mut() {
        first.bpm = p.bpm;
    }
    let tempo = TempoMap::new(points).map_err(|e| CommandError::invalid(e.to_string()))?;
    if tempo != *ctx.project.tempo {
        ctx.project.tempo = Arc::new(tempo);
    }
    ctx.set_label(format!("Set tempo to {} BPM", p.bpm));
    Ok(json!({ "bpm": p.bpm }))
}

fn set_meter(ctx: &mut Ctx<'_>, p: SetMeter) -> Result<Value, CommandError> {
    let mut changes = ctx.project.meter.changes().to_vec();
    let first = MeterChange { bar: 0, numerator: p.numerator, denominator: p.denominator };
    match changes.first_mut() {
        Some(change) => *change = first,
        None => changes.push(first),
    }
    let meter = MeterMap::new(changes).map_err(|e| CommandError::invalid(e.to_string()))?;
    if meter != *ctx.project.meter {
        ctx.project.meter = Arc::new(meter);
    }
    ctx.set_label(format!("Set meter to {}/{}", p.numerator, p.denominator));
    Ok(json!({ "meter": format!("{}/{}", p.numerator, p.denominator) }))
}

fn set_info(ctx: &mut Ctx<'_>, p: SetInfo) -> Result<Value, CommandError> {
    let mut meta = (*ctx.project.meta).clone();
    if let Some(title) = p.title {
        meta.title = title.trim().to_owned();
    }
    if let Some(author) = p.author {
        meta.author = author.trim().to_owned();
    }
    if meta != *ctx.project.meta {
        ctx.project.meta = Arc::new(meta.clone());
    }
    ctx.set_label("Edit project information");
    Ok(json!({ "title": meta.title, "author": meta.author }))
}

pub(super) fn register(r: &mut Registry) {
    r.session("project.new", "New project", "Starts a new project from a template, replacing the current one.", Effects::DESTRUCTIVE, new)
        .example(json!({ "template": "basic" }))
        .menu("File/New")
        .shortcut("Mod+N");
    r.session("project.open", "Open project", "Opens a .bodo file or .bodoproj folder, replacing the current project.", Effects::DESTRUCTIVE, open)
        .example(json!({ "path": "My Song.bodo" }))
        .menu("File/Open")
        .shortcut("Mod+O");
    r.session("project.save", "Save", "Saves the project to its file, or to path.", Effects::DISK, |s, p: Save| {
        let path = p.path.map(|text| target_path(s, &text));
        save_to(s, path)
    })
    .example(json!({ "path": "My Song.bodo" }))
    .menu("File/Save")
    .shortcut("Mod+S");
    r.session("project.save_as", "Save as", "Saves the project to a new file, which becomes its file.", Effects::DISK, |s, p: SaveAs| {
        let path = target_path(s, &p.path);
        save_to(s, Some(path))
    })
    .example(json!({ "path": "My Song.bodo" }))
    .menu("File/Save as")
    .shortcut("Mod+Shift+S");
    r.edit("project.set_tempo", "Set tempo", "Sets the tempo at the start of the song.", Effects::UNDOABLE, set_tempo)
        .example(json!({ "bpm": 128.0 }))
        .merge(whole);
    r.edit("project.set_meter", "Set meter", "Sets the time signature at the start of the song.", Effects::UNDOABLE, set_meter)
        .example(json!({ "numerator": 3, "denominator": 4 }));
    r.edit("project.set_info", "Set project information", "Sets the song's title and author.", Effects::UNDOABLE, set_info)
        .example(json!({ "title": "First Beat", "author": "hasan-muzak" }));
}
