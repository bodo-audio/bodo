//! The sounds the browser offers: Bodo's factory content and the user's sample folders (project.md section 14.1).

use std::path::{Path, PathBuf};

use bodo_model::FACTORY_PREFIX;

use crate::samples::is_audio_file;
use crate::session::Env;

/// How deep a search goes below each folder.
const MAX_DEPTH: usize = 8;
/// The most files a search looks at, so a huge folder can't stall it.
const MAX_FILES: usize = 20_000;

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

/// One entry of a folder listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The file or folder name.
    pub name: String,
    /// Its full path.
    pub path: PathBuf,
    /// Whether it is a folder.
    pub is_dir: bool,
}

/// The folders and audio files directly inside `dir`, folders first, each group sorted by name ignoring case. Hidden
/// entries are skipped. An unreadable folder lists as empty.
///
/// Does I/O. Complexity: O(n log n) in the entries of the folder.
pub fn list(dir: &Path) -> Vec<Entry> {
    let Ok(read) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut entries: Vec<Entry> = read
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let path = e.path();
            let is_dir = path.is_dir();
            (!name.starts_with('.') && (is_dir || is_audio_file(&path))).then_some(Entry { name, path, is_dir })
        })
        .collect();
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    entries
}

/// Audio files in the factory content and the library folders of `env` whose folder and name contain every word of
/// `query` (ignoring case), up to `limit` of them.
///
/// Does I/O. Complexity: O(f) in files visited, at most a fixed number.
pub fn search(env: &Env, query: &str, limit: usize) -> Vec<Sound> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let mut found = Vec::new();
    let mut visited = 0;
    let roots = env.factory.iter().map(|f| (f, true)).chain(env.library.iter().map(|l| (l, false)));
    'roots: for (root, factory) in roots {
        let mut stack = vec![(root.clone(), 0)];
        while let Some((dir, depth)) = stack.pop() {
            for entry in list(&dir).into_iter().rev() {
                visited += 1;
                if visited > MAX_FILES || found.len() >= limit {
                    break 'roots;
                }
                if entry.is_dir {
                    if depth < MAX_DEPTH {
                        stack.push((entry.path, depth + 1));
                    }
                    continue;
                }
                let relative = entry.path.strip_prefix(root).unwrap_or(&entry.path);
                let text = relative.to_string_lossy().replace('\\', "/");
                let lower = text.to_lowercase();
                if words.iter().all(|w| lower.contains(w)) {
                    let folder = text.rsplit_once('/').map_or("", |(folder, _)| folder).to_owned();
                    let name = entry.path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
                    let path = if factory { format!("{FACTORY_PREFIX}{text}") } else { entry.path.to_string_lossy().into_owned() };
                    found.push(Sound { name, path, folder });
                }
            }
        }
    }
    found.sort_by(|a, b| a.path.cmp(&b.path));
    found
}
