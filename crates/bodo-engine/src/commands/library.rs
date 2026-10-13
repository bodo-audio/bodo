//! `library.*`: the user library, as the Browser shows it (project.md section 14.1, ADR 0029).
//!
//! These read and tag files; they never change the project.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::{CommandError, ErrorCode};
use crate::library::index::LibraryIndex;
use crate::library::tags::{TagStore, file_key, normalize};
use crate::library::user::{FOLDERS, LIBRARY_PREFIX, MarketFolder, PresetFolder};
use crate::library::{self, EntryKind, UserLibrary};
use crate::registry::{Effects, Registry};
use crate::session::Ctx;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct List {
    /// The folder: one of the library's folders ("Packs", "My projects", "Templates"), a preset folder ("Channel
    /// presets", "Mixer presets", "Scores", "Plugin presets"), "Marketplace/Plugins" or "Marketplace/Stems", a
    /// `library:` key, or an absolute path. Empty lists the library's folders.
    #[serde(default)]
    folder: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Search {
    /// Words the file's path or tags must contain, ignoring case. Empty matches everything.
    #[serde(default)]
    query: String,
    /// Tags the file must have, every one.
    #[serde(default)]
    tags: Vec<String>,
    /// The most results, from 1 to 500; 50 by default.
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Tag {
    /// The file: an absolute path, a `library:` key, or a `factory:` path.
    path: String,
    /// Tags to give it.
    #[serde(default)]
    add: Vec<String>,
    /// Tags to take from it.
    #[serde(default)]
    remove: Vec<String>,
}

fn user(ctx: &Ctx<'_>) -> Result<UserLibrary, CommandError> {
    ctx.env().user.as_ref().map(UserLibrary::new).ok_or_else(|| CommandError::invalid("there is no user library"))
}

/// The folder `text` names in `library`.
fn folder(library: &UserLibrary, text: &str) -> Option<PathBuf> {
    let text = text.trim();
    if let Some(found) = FOLDERS.iter().find(|f| f.eq_ignore_ascii_case(text)) {
        return Some(library.folder(found));
    }
    if let Some(kind) = PresetFolder::ALL.into_iter().find(|k| k.name().eq_ignore_ascii_case(text)) {
        return Some(library.presets(kind));
    }
    if let Some(kind) = MarketFolder::ALL.into_iter().find(|k| text.eq_ignore_ascii_case(&format!("Marketplace/{}", k.name()))) {
        return Some(library.market(kind));
    }
    if text.starts_with(LIBRARY_PREFIX) {
        return library.resolve(text);
    }
    Path::new(text).is_absolute().then(|| PathBuf::from(text))
}

/// The file `text` names: absolute, a `library:` key, or a `factory:` path.
fn file(ctx: &Ctx<'_>, text: &str) -> Result<PathBuf, CommandError> {
    let text = text.trim();
    if text.starts_with(LIBRARY_PREFIX) {
        return user(ctx)?.resolve(text).ok_or_else(|| CommandError::not_found("file", text));
    }
    if text.starts_with(bodo_model::FACTORY_PREFIX) {
        return Ok(ctx.sample_roots().resolve(text));
    }
    Ok(PathBuf::from(text))
}

fn list(ctx: &mut Ctx<'_>, p: List) -> Result<Value, CommandError> {
    let library = user(ctx)?;
    if p.folder.trim().is_empty() {
        let folders: Vec<Value> = FOLDERS.iter().map(|f| json!({ "name": f, "path": library.key(&library.folder(f)) })).collect();
        return Ok(json!({ "root": library.root().display().to_string(), "folders": folders }));
    }
    let dir = folder(&library, &p.folder).ok_or_else(|| {
        CommandError::not_found("library folder", &p.folder)
            .with_hint(format!("one of: {}, Channel presets, Mixer presets, Scores", FOLDERS.join(", ")))
    })?;
    if !dir.is_dir() {
        return Err(CommandError::not_found("library folder", &dir.display().to_string()));
    }
    let entries: Vec<Value> =
        library::list(&dir).into_iter().map(|e| json!({ "name": e.stem(), "kind": e.kind, "path": file_key(ctx.env(), &e.path) })).collect();
    Ok(json!({ "folder": file_key(ctx.env(), &dir), "entries": entries }))
}

fn search(ctx: &mut Ctx<'_>, p: Search) -> Result<Value, CommandError> {
    let limit = p.limit.unwrap_or(50);
    if !(1..=500).contains(&limit) {
        return Err(CommandError::invalid("limit must be from 1 to 500"));
    }
    let tags: Vec<String> = p.tags.iter().filter_map(|t| normalize(t)).collect();
    let index = LibraryIndex::build(ctx.env());
    let found: Vec<Value> = index
        .find(&p.query, &tags, limit)
        .into_iter()
        .map(|f| json!({ "name": f.name, "kind": f.kind, "path": f.key, "tags": f.tags().collect::<Vec<_>>() }))
        .collect();
    Ok(json!({ "count": found.len(), "files": found }))
}

fn tag(ctx: &mut Ctx<'_>, p: Tag) -> Result<Value, CommandError> {
    let library = user(ctx)?;
    let path = file(ctx, &p.path)?;
    if EntryKind::of(&path).is_none() || !path.exists() {
        return Err(CommandError::not_found("file", &p.path));
    }
    let key = file_key(ctx.env(), &path);
    let mut store = TagStore::load(&library);
    let added = p.add.iter().filter(|t| store.add(&key, t)).count();
    let removed = p.remove.iter().filter(|t| store.remove(&key, t)).count();
    if added + removed > 0 {
        store.save(&library).map_err(|e| CommandError::new(ErrorCode::Io, format!("could not save the tags: {e}")))?;
    }
    Ok(json!({ "path": key, "tags": store.of(&key).collect::<Vec<_>>() }))
}

pub(super) fn register(r: &mut Registry) {
    r.edit("library.list", "List library folder", "Lists a folder of the user library: sounds, projects, presets and scores.", Effects::NONE, list)
        .example(json!({ "folder": "Channel presets" }));
    r.edit(
        "library.search",
        "Search library",
        "Finds files of any kind in the factory content, the sample folders and the user library, by words and tags.",
        Effects::NONE,
        search,
    )
    .example(json!({ "query": "kick", "tags": ["bodo drums"] }));
    r.edit("library.tag", "Tag file", "Adds tags to a library file, or removes them; the Browser filters by them.", Effects::DISK, tag)
        .example(json!({ "path": "factory:Packs/Bodo Drums/Kick.wav", "add": ["punchy"] }));
}
