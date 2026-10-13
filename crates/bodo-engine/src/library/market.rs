//! What the user owns from the Marketplace (project.md sections 14.1 and 15): plugins and stems, which the Marketplace
//! puts in the user library's Marketplace folder (M8). The Browser lists them, marked new when they arrived since the
//! user last looked; looking is recorded in the folder's `seen.json`.

use std::collections::BTreeSet;
use std::path::PathBuf;

use super::user::MarketFolder;
use super::{Entry, UserLibrary, list};

/// An owned item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Owned {
    /// The file or folder.
    pub entry: Entry,
    /// Whether it arrived since the user last looked.
    pub new: bool,
}

fn seen_file(library: &UserLibrary) -> PathBuf {
    library.marketplace().join("seen.json")
}

fn seen(library: &UserLibrary) -> BTreeSet<String> {
    std::fs::read_to_string(seen_file(library)).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

/// What the user owns of kind `kind`, with whether each item is new.
///
/// Does I/O. Complexity: O(n log n) in items.
pub fn owned(library: &UserLibrary, kind: MarketFolder) -> Vec<Owned> {
    let seen = seen(library);
    list(&library.market(kind))
        .into_iter()
        .map(|entry| {
            let new = library.key(&entry.path).is_none_or(|key| !seen.contains(&key));
            Owned { entry, new }
        })
        .collect()
}

/// Whether anything of either kind is new.
///
/// Does I/O. Complexity: O(n log n) in items.
pub fn has_new(library: &UserLibrary) -> bool {
    MarketFolder::ALL.into_iter().any(|kind| owned(library, kind).iter().any(|o| o.new))
}

/// Records that the user looked at kind `kind`: what is there now is no longer new.
///
/// Does I/O. Complexity: O(n log n) in items.
pub fn mark_seen(library: &UserLibrary, kind: MarketFolder) -> std::io::Result<()> {
    let mut seen = seen(library);
    let before = seen.len();
    seen.extend(list(&library.market(kind)).iter().filter_map(|e| library.key(&e.path)));
    if seen.len() == before {
        return Ok(());
    }
    let path = seen_file(library);
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)?;
    }
    let text = serde_json::to_string_pretty(&seen).map_err(std::io::Error::other)?;
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, text)?;
    std::fs::rename(&temp, &path)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    #[test]
    fn items_are_new_until_their_folder_is_looked_at() {
        let root = std::env::temp_dir().join(format!("bodo-market-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let library = UserLibrary::new(&root);
        library.ensure().unwrap();
        assert!(!has_new(&library));
        std::fs::write(library.market(MarketFolder::Stems).join("Rhodes Dust.wav"), b"").unwrap();
        std::fs::create_dir_all(library.market(MarketFolder::Plugins).join("Glow.clap")).unwrap();
        assert!(has_new(&library));
        let plugins = owned(&library, MarketFolder::Plugins);
        assert_eq!((plugins.len(), plugins[0].new, plugins[0].entry.kind), (1, true, super::super::EntryKind::Plugin));
        mark_seen(&library, MarketFolder::Stems).unwrap();
        assert!(!owned(&library, MarketFolder::Stems)[0].new);
        assert!(has_new(&library), "the plugin is still new");
        mark_seen(&library, MarketFolder::Plugins).unwrap();
        assert!(!has_new(&library));
        std::fs::write(library.market(MarketFolder::Stems).join("Keys Haze.wav"), b"").unwrap();
        assert!(has_new(&library), "a new arrival is new again");
        let _ = std::fs::remove_dir_all(root);
    }
}
