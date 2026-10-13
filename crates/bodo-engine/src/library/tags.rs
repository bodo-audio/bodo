//! Tags on library files (project.md section 14.1, ADR 0029).
//!
//! Every file has tags from where it is and what it is called: the names of its folders below its library root, and
//! the words of its name ("Packs/Bodo Drums/Kick 01.wav" gets `packs`, `bodo drums`, `kick`). The user adds their
//! own from the Browser; those live in the user library's tags file, keyed by the file's key: `factory:` and its path
//! for factory content, `library:` and its path for the user library, and the absolute path for anything else.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use bodo_model::FACTORY_PREFIX;

use super::UserLibrary;
use crate::session::Env;

/// The tags the user gave files, by file key.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TagStore {
    files: BTreeMap<String, BTreeSet<String>>,
}

impl TagStore {
    /// The tags saved in `library`; none when the file is missing or unreadable.
    ///
    /// Does I/O. Complexity: O(size of the file).
    pub fn load(library: &UserLibrary) -> Self {
        let Ok(text) = std::fs::read_to_string(library.tags_file()) else { return Self::default() };
        serde_json::from_str(&text).unwrap_or_else(|error| {
            tracing::warn!(%error, "ignoring an unreadable tags file");
            Self::default()
        })
    }

    /// Saves the tags in `library`, replacing the file atomically.
    ///
    /// Does I/O. Complexity: O(tags).
    pub fn save(&self, library: &UserLibrary) -> std::io::Result<()> {
        let path = library.tags_file();
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder)?;
        }
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, text)?;
        std::fs::rename(&temp, &path)
    }

    /// The tags the user gave the file `key` names.
    pub fn of(&self, key: &str) -> impl Iterator<Item = &str> {
        self.files.get(key).into_iter().flatten().map(String::as_str)
    }

    /// Gives the file `key` names tag `tag`. Returns whether it was new.
    ///
    /// Complexity: O(log n) in tagged files.
    pub fn add(&mut self, key: &str, tag: &str) -> bool {
        normalize(tag).is_some_and(|tag| self.files.entry(key.to_owned()).or_default().insert(tag))
    }

    /// Takes tag `tag` from the file `key` names. Returns whether it had it.
    ///
    /// Complexity: O(log n) in tagged files.
    pub fn remove(&mut self, key: &str, tag: &str) -> bool {
        let Some(tag) = normalize(tag) else { return false };
        let Some(tags) = self.files.get_mut(key) else { return false };
        let removed = tags.remove(&tag);
        if tags.is_empty() {
            self.files.remove(key);
        }
        removed
    }
}

/// A tag as stored: trimmed and lowercase. `None` for an empty one.
pub fn normalize(tag: &str) -> Option<String> {
    let tag = tag.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    (!tag.is_empty()).then_some(tag)
}

/// The tags a file gets from `relative`, its path below its library root with forward slashes: the name of each
/// folder, and each word of its name that has a letter in it.
///
/// Complexity: O(length of the path).
pub fn auto_tags(relative: &str) -> Vec<String> {
    let mut tags = BTreeSet::new();
    let mut parts: Vec<&str> = relative.split('/').filter(|p| !p.is_empty()).collect();
    let file = parts.pop().unwrap_or_default();
    for folder in parts {
        tags.extend(normalize(folder));
    }
    let stem = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
    for word in words(stem) {
        tags.insert(word);
    }
    tags.into_iter().collect()
}

/// The words of a file name, lowercase: runs of letters, split where a lowercase letter meets an uppercase one.
/// Runs of digits are left out, and so are single letters.
fn words(name: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut last_lower = false;
    for c in name.chars() {
        if c.is_alphabetic() {
            if c.is_uppercase() && last_lower && !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            current.extend(c.to_lowercase());
            last_lower = c.is_lowercase();
        } else {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            last_lower = false;
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words.retain(|w| w.chars().count() > 1);
    words
}

/// The key of `file` in `env`: `factory:` and its path inside the factory content, `library:` and its path inside
/// the user library, or its absolute path.
pub fn file_key(env: &Env, file: &Path) -> String {
    if let Some(rest) = env.factory.as_deref().and_then(|f| file.strip_prefix(f).ok()) {
        let parts: Vec<String> = rest.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
        return format!("{FACTORY_PREFIX}{}", parts.join("/"));
    }
    if let Some(key) = env.user.as_ref().and_then(|u| UserLibrary::new(u).key(file)) {
        return key;
    }
    file.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    #[test]
    fn files_get_tags_from_their_folders_and_name_words() {
        assert_eq!(auto_tags("Packs/Bodo Drums/Kick 01.wav"), ["bodo drums", "kick", "packs"]);
        assert_eq!(auto_tags("DeepHouseSnare_v2.wav"), ["deep", "house", "snare"], "camel case splits; digits and single letters go");
        assert_eq!(auto_tags("x.wav"), Vec::<String>::new());
    }

    #[test]
    fn user_tags_are_kept_normalized_and_saved() {
        let root = std::env::temp_dir().join(format!("bodo-tags-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let library = UserLibrary::new(&root);
        let mut tags = TagStore::default();
        assert!(tags.add("library:Audio/Pad.wav", "  Warm   Pads "));
        assert!(!tags.add("library:Audio/Pad.wav", "warm pads"), "already there");
        assert!(!tags.add("library:Audio/Pad.wav", "   "));
        tags.save(&library).unwrap();
        let loaded = TagStore::load(&library);
        assert_eq!(loaded.of("library:Audio/Pad.wav").collect::<Vec<_>>(), ["warm pads"]);
        let mut loaded = loaded;
        assert!(loaded.remove("library:Audio/Pad.wav", "Warm Pads"));
        assert_eq!(loaded, TagStore::default(), "a file with no tags left goes");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn keys_name_where_a_file_is() {
        let env = Env { factory: Some("/f".into()), library: Vec::new(), user: Some("/u".into()) };
        assert_eq!(file_key(&env, Path::new("/f/Packs/Kick.wav")), "factory:Packs/Kick.wav");
        assert_eq!(file_key(&env, Path::new("/u/Audio/Pad.wav")), "library:Audio/Pad.wav");
        assert_eq!(file_key(&env, Path::new("/elsewhere/Pad.wav")), "/elsewhere/Pad.wav");
    }
}
