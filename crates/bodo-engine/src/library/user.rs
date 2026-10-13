//! The user library: a Bodo folder in the user's documents, made on first run (project.md section 14.1, ADR 0029).
//!
//! It holds the folders the Browser lists, and Bodo puts things in them:
//!
//! - **My projects** is where a new project is saved first; **Templates** holds projects that `project.new` can start
//!   from; **Demo projects** holds demos built from the factory content.
//! - **Recorded** and **Rendered** take recordings and exports of a project that has no folder of its own yet.
//! - **Backup** holds the autosaves.
//! - **Packs** holds the user's own packs, listed beside the factory packs.
//! - **User data** holds the presets and scores (each in its own folder), the Marketplace's owned plugins and stems,
//!   and the tags the user gives files.
//!
//! The other folders (Audio, Channel envelopes, Clipboard files, Envelopes, Impulses, Misc, Project parts, Sliced
//! audio, Soundfonts and Speech) are places for the user's own files of those kinds; the Browser lists and opens
//! whatever Bodo can use in them.

use std::path::{Path, PathBuf};

/// The library's folders, in the Browser's order.
pub const FOLDERS: [&str; 18] = [
    "Audio",
    "Backup",
    "Channel envelopes",
    "Clipboard files",
    "Demo projects",
    "Envelopes",
    "Impulses",
    "Misc",
    "My projects",
    "Packs",
    "Project parts",
    "Recorded",
    "Rendered",
    "Sliced audio",
    "Soundfonts",
    "Speech",
    "Templates",
    "User data",
];

/// The folder of user data, inside the library.
pub const USER_DATA: &str = "User data";

/// Kinds of presets the library keeps, each in a folder of User data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum PresetFolder {
    /// Plugin presets.
    PluginPresets,
    /// Channel presets.
    ChannelPresets,
    /// Mixer presets.
    MixerPresets,
    /// Scores.
    Scores,
}

impl PresetFolder {
    /// Every kind, in the Browser's order.
    pub const ALL: [Self; 4] = [Self::PluginPresets, Self::ChannelPresets, Self::MixerPresets, Self::Scores];

    /// The folder's name, as the Browser shows it.
    pub fn name(self) -> &'static str {
        match self {
            Self::PluginPresets => "Plugin presets",
            Self::ChannelPresets => "Channel presets",
            Self::MixerPresets => "Mixer presets",
            Self::Scores => "Scores",
        }
    }
}

/// What the user owns from the Marketplace, by kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum MarketFolder {
    /// Plugins the user owns.
    Plugins,
    /// Stems the user owns.
    Stems,
}

impl MarketFolder {
    /// Both kinds, in the Browser's order.
    pub const ALL: [Self; 2] = [Self::Plugins, Self::Stems];

    /// The folder's name inside the Marketplace folder.
    pub fn name(self) -> &'static str {
        match self {
            Self::Plugins => "Plugins",
            Self::Stems => "Stems",
        }
    }
}

/// The user library at a root folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserLibrary {
    root: PathBuf,
}

impl UserLibrary {
    /// The library at `root`. Nothing is read or made until [`UserLibrary::ensure`].
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The library's folder.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The folder named `name`, one of [`FOLDERS`].
    pub fn folder(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// Makes every folder of the library that is missing, and returns how many it made.
    ///
    /// Does I/O. Complexity: O(1): a fixed number of folders.
    pub fn ensure(&self) -> std::io::Result<usize> {
        let mut made = 0;
        let preset_folders = PresetFolder::ALL.map(|p| self.presets(p));
        let market_folders = MarketFolder::ALL.map(|m| self.market(m));
        for folder in FOLDERS.iter().map(|f| self.folder(f)).chain(preset_folders).chain(market_folders) {
            if !folder.is_dir() {
                std::fs::create_dir_all(&folder)?;
                made += 1;
            }
        }
        Ok(made)
    }

    /// The folder of presets of kind `kind`, inside User data.
    pub fn presets(&self, kind: PresetFolder) -> PathBuf {
        self.folder(USER_DATA).join(kind.name())
    }

    /// The folder of the Marketplace, inside User data.
    pub fn marketplace(&self) -> PathBuf {
        self.folder(USER_DATA).join("Marketplace")
    }

    /// The folder of owned Marketplace items of kind `kind`.
    pub fn market(&self, kind: MarketFolder) -> PathBuf {
        self.marketplace().join(kind.name())
    }

    /// The file of the tags the user gave files.
    pub fn tags_file(&self) -> PathBuf {
        self.folder(USER_DATA).join("tags.json")
    }

    /// Where a project saves first.
    pub fn my_projects(&self) -> PathBuf {
        self.folder("My projects")
    }

    /// Project templates.
    pub fn templates(&self) -> PathBuf {
        self.folder("Templates")
    }

    /// Demo projects.
    pub fn demos(&self) -> PathBuf {
        self.folder("Demo projects")
    }

    /// Autosaves.
    pub fn backup(&self) -> PathBuf {
        self.folder("Backup")
    }

    /// Recordings of a project that has no folder yet.
    pub fn recorded(&self) -> PathBuf {
        self.folder("Recorded")
    }

    /// Exports of a project that has no folder yet.
    pub fn rendered(&self) -> PathBuf {
        self.folder("Rendered")
    }

    /// The user's own packs.
    pub fn packs(&self) -> PathBuf {
        self.folder("Packs")
    }

    /// How a file inside the library is named in tags and the Marketplace's records: `library:` and its path from the
    /// root with forward slashes. `None` for a file outside the library.
    pub fn key(&self, file: &Path) -> Option<String> {
        let relative = file.strip_prefix(&self.root).ok()?;
        let parts: Vec<String> = relative.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
        Some(format!("{LIBRARY_PREFIX}{}", parts.join("/")))
    }

    /// The file a `library:` key names.
    pub fn resolve(&self, key: &str) -> Option<PathBuf> {
        key.strip_prefix(LIBRARY_PREFIX).map(|rest| rest.split('/').fold(self.root.clone(), |path, part| path.join(part)))
    }
}

/// How keys of files in the user library start.
pub const LIBRARY_PREFIX: &str = "library:";

/// A file name for `name` in `folder` with `extension` that no file has yet: `name`, then `name 2`, `name 3` and on.
/// Characters that file systems refuse become underscores.
///
/// Reads the folder. Complexity: O(k) in the names tried.
pub fn unused_file(folder: &Path, name: &str, extension: &str) -> PathBuf {
    let clean: String =
        name.trim().chars().map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c }).collect();
    let clean = if clean.is_empty() { String::from("Untitled") } else { clean };
    let mut path = folder.join(format!("{clean}.{extension}"));
    let mut n = 2;
    while path.exists() {
        path = folder.join(format!("{clean} {n}.{extension}"));
        n += 1;
    }
    path
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    #[test]
    fn ensure_makes_every_folder_once() {
        let root = std::env::temp_dir().join(format!("bodo-library-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let library = UserLibrary::new(&root);
        assert_eq!(library.ensure().unwrap(), FOLDERS.len() + PresetFolder::ALL.len() + MarketFolder::ALL.len());
        assert_eq!(library.ensure().unwrap(), 0, "nothing to make the second time");
        assert!(library.presets(PresetFolder::Scores).ends_with("User data/Scores"));
        assert!(library.market(MarketFolder::Stems).is_dir());
        let file = library.packs().join("Mine").join("Hat.wav");
        let key = library.key(&file).unwrap();
        assert_eq!(key, "library:Packs/Mine/Hat.wav");
        assert_eq!(library.resolve(&key).unwrap(), file);
        std::fs::write(library.templates().join("Song.bodo"), b"").unwrap();
        assert!(unused_file(&library.templates(), "Song", "bodo").ends_with("Song 2.bodo"));
        assert!(unused_file(&library.templates(), "a/b", "bodo").ends_with("a_b.bodo"));
        let _ = std::fs::remove_dir_all(root);
    }
}
