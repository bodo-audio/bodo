//! Bringing sample files into a project (project.md section 18.1).
//!
//! A sample is stored with a path in one of three forms: inside the factory content as `factory:<path>`, inside the
//! project's folder as a relative path with forward slashes, or anywhere else as an absolute path. A BLAKE3 hash of
//! the file goes with it, so a moved file can be found again.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bodo_model::{FACTORY_PREFIX, SampleId, SampleRef};

use crate::error::CommandError;
use crate::session::Ctx;

/// File extensions the browser lists and channels accept.
pub const AUDIO_EXTENSIONS: [&str; 9] = ["wav", "wave", "flac", "aif", "aiff", "mp3", "ogg", "m4a", "aac"];

/// Whether `path` has an audio file extension.
pub fn is_audio_file(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| AUDIO_EXTENSIONS.iter().any(|a| a.eq_ignore_ascii_case(e)))
}

/// The BLAKE3 hash of `file`, or `None` when it can't be read.
pub fn hash(file: &Path) -> Option<String> {
    bodo_io::hash_file(file).ok()
}

/// How to store the path of `file` in a project whose folder is `project_dir`, with factory content in `factory`.
pub fn stored_path(file: &Path, project_dir: Option<&Path>, factory: Option<&Path>) -> String {
    if let Some(rest) = factory.and_then(|f| file.strip_prefix(f).ok()) {
        return format!("{FACTORY_PREFIX}{}", slashes(rest));
    }
    bodo_io::sample_path(project_dir, file)
}

fn slashes(path: &Path) -> String {
    path.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
}

/// The file `text` names: a `factory:` path, an absolute path, or a path relative to the project's folder (or the
/// current folder for an unsaved project).
pub(crate) fn locate(ctx: &Ctx<'_>, text: &str) -> PathBuf {
    let text = text.trim();
    if text.starts_with(FACTORY_PREFIX) {
        return ctx.sample_roots().resolve(text);
    }
    let path = Path::new(text);
    if path.is_absolute() { path.to_path_buf() } else { ctx.sample_roots().project.join(path) }
}

/// Adds the audio file `text` names to the project's samples, or finds it there already, and returns its id. The file
/// must exist and decode.
///
/// Does I/O: reads the file's header and hashes it. Complexity: O(s) in samples, plus the file's size.
pub(crate) fn import(ctx: &mut Ctx<'_>, text: &str) -> Result<SampleId, CommandError> {
    let file = locate(ctx, text);
    if !file.is_file() {
        return Err(CommandError::not_found("sample file", text)
            .with_hint("give an absolute path, a path inside the project folder, or a factory: path from browser.search"));
    }
    bodo_kit_audio::Decoder::open(&file).map_err(|e| CommandError::io(&file, format!("not a readable audio file: {e}")))?;
    let stored = stored_path(&file, ctx.project_dir().as_deref(), ctx.env().factory.as_deref());
    if let Some(existing) = ctx.project.samples.values().find(|s| s.path == stored) {
        return Ok(existing.id);
    }
    let id = ctx.ids().sample();
    ctx.project.samples.insert(id, Arc::new(SampleRef { id, path: stored, hash: hash(&file), market: None }));
    Ok(id)
}

/// The file name of `path` without its extension, as a channel name.
pub fn display_name(path: &str) -> String {
    let name = path.rsplit(['/', '\\', ':']).next().unwrap_or(path);
    name.rsplit_once('.').map_or(name, |(stem, _)| stem).to_owned()
}
