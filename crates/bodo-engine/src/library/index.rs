//! An index of every file the Browser can show, for searching and filtering by tag (project.md section 14.1).
//!
//! Building it reads the disk, so the Browser builds it on a background thread and keeps it until the user refreshes.

use std::collections::BTreeMap;
use std::path::PathBuf;

use super::tags::{TagStore, auto_tags, file_key};
use super::{EntryKind, MAX_DEPTH, MAX_FILES, UserLibrary, list, roots};
use crate::session::Env;

/// One file of the library.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexedFile {
    /// Its name without the extension.
    pub name: String,
    /// Where it is.
    pub path: PathBuf,
    /// Its key, as in [`file_key`].
    pub key: String,
    /// Its path below its library root, with forward slashes.
    pub relative: String,
    /// What it is.
    pub kind: EntryKind,
    /// The tags it has from its folders and name.
    pub auto_tags: Vec<String>,
    /// The tags the user gave it.
    pub user_tags: Vec<String>,
}

impl IndexedFile {
    /// Every tag it has, its own first.
    pub fn tags(&self) -> impl Iterator<Item = &str> {
        self.user_tags.iter().chain(&self.auto_tags).map(String::as_str)
    }

    fn has_tag(&self, tag: &str) -> bool {
        self.tags().any(|t| t == tag)
    }
}

/// Every file of the library, with its tags.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LibraryIndex {
    /// The files, sorted by key.
    pub files: Vec<IndexedFile>,
}

impl LibraryIndex {
    /// Reads every file the Browser shows in the factory content, the sample folders and the user library of `env`,
    /// up to a fixed number of files, with the user's tags.
    ///
    /// Does I/O. Complexity: O(f log f) in files visited, at most a fixed number.
    pub fn build(env: &Env) -> Self {
        let store = env.user.as_ref().map(|u| TagStore::load(&UserLibrary::new(u))).unwrap_or_default();
        let mut files = Vec::new();
        let mut visited = 0;
        'roots: for (root, _) in roots(env) {
            let mut stack = vec![(root.clone(), 0)];
            while let Some((dir, depth)) = stack.pop() {
                for entry in list(&dir) {
                    visited += 1;
                    if visited > MAX_FILES {
                        break 'roots;
                    }
                    if entry.is_dir() {
                        if depth < MAX_DEPTH {
                            stack.push((entry.path, depth + 1));
                        }
                        continue;
                    }
                    let relative = entry.path.strip_prefix(&root).unwrap_or(&entry.path);
                    let relative: Vec<String> = relative.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
                    let relative = relative.join("/");
                    files.push(IndexedFile {
                        name: entry.stem(),
                        key: file_key(env, &entry.path),
                        auto_tags: auto_tags(&relative),
                        user_tags: Vec::new(),
                        relative,
                        path: entry.path,
                        kind: entry.kind,
                    });
                }
            }
        }
        files.sort_by(|a, b| a.key.cmp(&b.key));
        files.dedup_by(|a, b| a.key == b.key);
        let mut index = Self { files };
        index.apply(&store);
        index
    }

    /// Takes the user's tags from `store`.
    ///
    /// Complexity: O(f log t) in files and tagged files.
    pub fn apply(&mut self, store: &TagStore) {
        for file in &mut self.files {
            file.user_tags = store.of(&file.key).map(str::to_owned).collect();
        }
    }

    /// Every tag and how many files have it, the most common first, then by name.
    ///
    /// Complexity: O(f · t + u log u) in files, tags per file and distinct tags.
    pub fn tag_counts(&self) -> Vec<(String, usize)> {
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for file in &self.files {
            for tag in file.tags() {
                *counts.entry(tag).or_default() += 1;
            }
        }
        let mut counts: Vec<(String, usize)> = counts.into_iter().map(|(tag, n)| (tag.to_owned(), n)).collect();
        counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        counts
    }

    /// The files whose path or tags contain every word of `query`, ignoring case, and that have every tag in `tags`,
    /// up to `limit` of them, in key order.
    ///
    /// Complexity: O(f · (w + t)) in files, query words and tags.
    pub fn find(&self, query: &str, tags: &[String], limit: usize) -> Vec<&IndexedFile> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        self.files
            .iter()
            .filter(|file| tags.iter().all(|tag| file.has_tag(tag)))
            .filter(|file| {
                let path = file.relative.to_lowercase();
                words.iter().all(|w| path.contains(w) || file.tags().any(|t| t.contains(w.as_str())))
            })
            .take(limit)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    #[test]
    fn the_index_holds_every_kind_with_its_tags() {
        let root = std::env::temp_dir().join(format!("bodo-index-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let library = UserLibrary::new(&root);
        library.ensure().unwrap();
        std::fs::write(library.folder("Audio").join("Warm Pad.wav"), b"").unwrap();
        std::fs::write(library.folder("Audio").join("notes.txt"), b"").unwrap();
        std::fs::write(library.presets(super::super::user::PresetFolder::ChannelPresets).join("Big Kick.bodochannel"), b"").unwrap();
        let mut store = TagStore::default();
        store.add("library:Audio/Warm Pad.wav", "favorite");
        store.save(&library).unwrap();

        let env = Env { factory: None, library: Vec::new(), user: Some(root.clone()) };
        let index = LibraryIndex::build(&env);
        let names: Vec<&str> = index.files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["Warm Pad", "Big Kick"], "text files are left out");
        let pad = &index.files[0];
        assert_eq!((pad.kind, pad.tags().collect::<Vec<_>>()), (EntryKind::Sound, vec!["favorite", "audio", "pad", "warm"]));
        assert_eq!(index.find("", &[String::from("favorite")], 10).len(), 1);
        assert_eq!(index.find("kick", &[], 10)[0].kind, EntryKind::ChannelPreset);
        assert_eq!(index.find("warm", &[String::from("user data")], 10).len(), 0, "every tag must match");
        let counts = index.tag_counts();
        assert!(counts.contains(&(String::from("audio"), 1)) && counts.contains(&(String::from("user data"), 1)));
        let _ = std::fs::remove_dir_all(root);
    }
}
