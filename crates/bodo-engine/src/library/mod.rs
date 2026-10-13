//! What the Browser offers (project.md section 14.1): Bodo's factory content, the user's sample folders, and the user
//! library, a Bodo folder in the user's documents (ADR 0029).
//!
//! - [`list`] reads one folder for the Browser, and [`search`] finds sounds for `browser.search`.
//! - [`user`] lays out the user library: its folders, and where presets, templates, recordings and exports go.
//! - [`tags`] keeps the tags the user gives files, and makes the ones every file gets from its folder and name.
//! - [`index`] reads the whole library once, for searching and filtering by tag.
//! - [`market`] lists what the user owns from the Marketplace, and what is new since they last looked.

pub mod index;
pub mod market;
pub mod tags;
pub mod user;

use std::path::{Path, PathBuf};

use bodo_io::preset::{CHANNEL_PRESET_EXTENSION, MIXER_PRESET_EXTENSION, SCORE_EXTENSION};
use bodo_model::FACTORY_PREFIX;

use crate::samples::is_audio_file;
use crate::session::Env;

pub use self::user::UserLibrary;

/// How deep a search goes below each folder.
const MAX_DEPTH: usize = 8;
/// The most files a search looks at, so a huge folder can't stall it.
const MAX_FILES: usize = 20_000;

/// The extension of plugin presets, which plugins read and write (M5 and M6).
pub const PLUGIN_PRESET_EXTENSION: &str = "bodopreset";

/// An audio file in the library.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sound {
    /// The file's name without its extension.
    pub name: String,
    /// The path to give `channel.add`: `factory:...` inside the factory content, absolute otherwise.
    pub path: String,
    /// The folder it is in, relative to its library root, with forward slashes.
    pub folder: String,
}

/// What a file in the Browser is, which says what opening it does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EntryKind {
    /// A folder.
    Folder,
    /// An audio file.
    Sound,
    /// A `.bodo` file or `.bodoproj` folder.
    Project,
    /// A channel preset.
    ChannelPreset,
    /// A mixer preset.
    MixerPreset,
    /// A score: notes of one channel.
    Score,
    /// A plugin preset.
    PluginPreset,
    /// A Standard MIDI File.
    Midi,
    /// A plugin file or bundle: CLAP, VST3, Audio Unit or LV2.
    Plugin,
}

/// Extensions of plugin files and bundles.
pub const PLUGIN_EXTENSIONS: [&str; 4] = ["clap", "vst3", "component", "lv2"];

impl EntryKind {
    /// What the file at `path` is, or `None` for a file the Browser doesn't show. Reads the disk only to tell folders
    /// from files.
    pub fn of(path: &Path) -> Option<Self> {
        let extension = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
        if PLUGIN_EXTENSIONS.contains(&extension.as_str()) {
            return Some(Self::Plugin);
        }
        if path.is_dir() {
            return Some(if extension == "bodoproj" { Self::Project } else { Self::Folder });
        }
        Some(match extension.as_str() {
            "bodo" => Self::Project,
            "mid" | "midi" => Self::Midi,
            e if e == CHANNEL_PRESET_EXTENSION => Self::ChannelPreset,
            e if e == MIXER_PRESET_EXTENSION => Self::MixerPreset,
            e if e == SCORE_EXTENSION => Self::Score,
            e if e == PLUGIN_PRESET_EXTENSION => Self::PluginPreset,
            _ if is_audio_file(path) => Self::Sound,
            _ => return None,
        })
    }

    /// A word for it, such as "sound" or "channel preset".
    pub fn noun(self) -> &'static str {
        match self {
            Self::Folder => "folder",
            Self::Sound => "sound",
            Self::Project => "project",
            Self::ChannelPreset => "channel preset",
            Self::MixerPreset => "mixer preset",
            Self::Score => "score",
            Self::PluginPreset => "plugin preset",
            Self::Midi => "MIDI file",
            Self::Plugin => "plugin",
        }
    }
}

/// One entry of a folder listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The file or folder name.
    pub name: String,
    /// Its full path.
    pub path: PathBuf,
    /// What it is.
    pub kind: EntryKind,
}

impl Entry {
    /// Whether it is a folder the Browser opens, rather than a file (a `.bodoproj` folder is a project).
    pub fn is_dir(&self) -> bool {
        self.kind == EntryKind::Folder
    }

    /// Its name without the extension.
    pub fn stem(&self) -> String {
        match self.kind {
            EntryKind::Folder => self.name.clone(),
            _ => self.path.file_stem().map_or_else(|| self.name.clone(), |s| s.to_string_lossy().into_owned()),
        }
    }
}

/// The folders and the files the Browser shows directly inside `dir`: audio, projects, presets, scores and MIDI
/// files. Folders come first, each group sorted by name ignoring case. Hidden entries are skipped. An unreadable
/// folder lists as empty.
///
/// Does I/O. Complexity: O(n log n) in the entries of the folder.
pub fn list(dir: &Path) -> Vec<Entry> {
    let Ok(read) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut entries: Vec<Entry> = read
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let path = e.path();
            if name.starts_with('.') {
                return None;
            }
            EntryKind::of(&path).map(|kind| Entry { name, path, kind })
        })
        .collect();
    entries.sort_by(|a, b| b.is_dir().cmp(&a.is_dir()).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    entries
}

/// Where recordings of a project go: `Recorded` beside its file, or the user library's Recorded folder for a project
/// with no file yet. `None` when there is neither.
pub fn recorded_folder(env: &Env, project_dir: Option<&Path>) -> Option<PathBuf> {
    project_dir.map(|d| d.join("Recorded")).or_else(|| env.user.as_ref().map(|u| UserLibrary::new(u).recorded()))
}

/// Where exports of a project go: `Rendered` beside its file, or the user library's Rendered folder for a project with
/// no file yet. `None` when there is neither.
pub fn rendered_folder(env: &Env, project_dir: Option<&Path>) -> Option<PathBuf> {
    project_dir.map(|d| d.join("Rendered")).or_else(|| env.user.as_ref().map(|u| UserLibrary::new(u).rendered()))
}

/// The folders a search reads, each with whether it is the factory content: the factory content, the user's sample
/// folders, then the user library.
pub(crate) fn roots(env: &Env) -> Vec<(PathBuf, bool)> {
    env.factory
        .iter()
        .map(|f| (f.clone(), true))
        .chain(env.library.iter().map(|l| (l.clone(), false)))
        .chain(env.user.iter().map(|u| (u.clone(), false)))
        .collect()
}

/// Audio files in the factory content, the sample folders and the user library of `env` whose folder and name
/// contain every word of `query` (ignoring case), up to `limit` of them.
///
/// Does I/O. Complexity: O(f) in files visited, at most a fixed number.
pub fn search(env: &Env, query: &str, limit: usize) -> Vec<Sound> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let mut found = Vec::new();
    let mut visited = 0;
    'roots: for (root, factory) in roots(env) {
        let mut stack = vec![(root.clone(), 0)];
        while let Some((dir, depth)) = stack.pop() {
            for entry in list(&dir).into_iter().rev() {
                visited += 1;
                if visited > MAX_FILES || found.len() >= limit {
                    break 'roots;
                }
                if entry.is_dir() {
                    if depth < MAX_DEPTH {
                        stack.push((entry.path, depth + 1));
                    }
                    continue;
                }
                if entry.kind != EntryKind::Sound {
                    continue;
                }
                let relative = entry.path.strip_prefix(&root).unwrap_or(&entry.path);
                let text = relative.to_string_lossy().replace('\\', "/");
                let lower = text.to_lowercase();
                if words.iter().all(|w| lower.contains(w)) {
                    let folder = text.rsplit_once('/').map_or("", |(folder, _)| folder).to_owned();
                    let name = entry.stem();
                    let path = if factory { format!("{FACTORY_PREFIX}{text}") } else { entry.path.to_string_lossy().into_owned() };
                    found.push(Sound { name, path, folder });
                }
            }
        }
    }
    found.sort_by(|a, b| a.path.cmp(&b.path));
    found.dedup_by(|a, b| a.path == b.path);
    found
}
