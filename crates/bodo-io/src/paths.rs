//! Where a project's samples are (project.md section 18.1).
//!
//! A [`SampleRef`] stores its file as one of three kinds of string:
//!
//! - **Relative**, with forward slashes, to the *project folder*: the folder that holds the `.bodo` file or the
//!   `.bodoproj` folder ([`project_folder`]). Packing and unpacking in place keeps these paths valid.
//! - **Absolute**, for files outside the project folder.
//! - **Factory content**, starting with [`FACTORY_PREFIX`]. These name Bodo's own content and are never rebased.
//!
//! Files inside the project folder are always stored relative to it, and files outside it absolute. Saving a project
//! somewhere else rebases the relative paths so they keep pointing at the same files.
//!
//! Paths are compared lexically: `.` and `..` are resolved without touching the disk, and symbolic links are not
//! followed, so a sample that doesn't exist yet still gets a sensible path.

use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use bodo_model::{Project, SampleRef};

use crate::error::IoError;

/// The prefix of sample paths that name Bodo's factory content, such as `factory:drums/kick.wav`.
pub const FACTORY_PREFIX: &str = "factory:";

/// Whether a stored sample path names factory content.
pub fn is_factory(stored: &str) -> bool {
    stored.starts_with(FACTORY_PREFIX)
}

/// The project folder of the project saved at `project`, a `.bodo` file or `.bodoproj` folder: the folder that
/// holds it, made absolute. Relative sample paths start there.
///
/// Complexity: O(length of the path).
pub fn project_folder(project: &Path) -> PathBuf {
    let absolute = absolute(project);
    absolute.parent().map_or(absolute.clone(), Path::to_path_buf)
}

/// The string to store for a sample at `file`, in a project whose folder is `folder` (`None` for a project that
/// was never saved): relative with forward slashes when the file is inside the folder, absolute otherwise.
///
/// Complexity: O(length of the paths).
pub fn sample_path(folder: Option<&Path>, file: &Path) -> String {
    let file = absolute(file);
    if let Some(folder) = folder
        && let Ok(rest) = file.strip_prefix(absolute(folder))
        && !rest.as_os_str().is_empty()
    {
        return rest.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
    }
    file.to_string_lossy().into_owned()
}

/// The file a stored sample path names, in a project whose folder is `folder` (`None` for a project that was never
/// saved, whose relative paths start at the current directory). Returns `None` for factory content, which the
/// caller resolves.
///
/// Complexity: O(length of the paths).
pub fn resolve_sample_path(folder: Option<&Path>, stored: &str) -> Option<PathBuf> {
    if is_factory(stored) {
        return None;
    }
    let path = Path::new(stored);
    if path.is_absolute() {
        return Some(normalize(path));
    }
    Some(match folder {
        Some(folder) => normalize(&absolute(folder).join(path)),
        None => absolute(path),
    })
}

/// `project` with its sample paths rebased from the folder `from` (`None`: the current directory) to the folder
/// `to`. Samples whose path doesn't change stay shared with `project`.
///
/// Complexity: O(s log s) in the number of samples, plus the length of their paths.
pub(crate) fn rebase(project: &Project, from: Option<&Path>, to: &Path) -> Project {
    let mut out = project.clone();
    for (id, sample) in &project.samples {
        let Some(file) = resolve_sample_path(from, &sample.path) else { continue };
        let stored = sample_path(Some(to), &file);
        if stored != sample.path {
            out.samples.insert(*id, Arc::new(SampleRef { path: stored, ..(**sample).clone() }));
        }
    }
    out
}

/// The BLAKE3 hash of the file at `path`, as 64 lowercase hex digits. Projects store it next to each sample so a
/// moved file can be found again.
///
/// Complexity: O(size of the file).
pub fn hash_file(path: &Path) -> Result<String, IoError> {
    let mut file = std::fs::File::open(path).map_err(|err| IoError::io("open", path, err))?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|err| IoError::io("read", path, err))?;
        if read == 0 {
            break;
        }
        hasher.update(buffer.get(..read).unwrap_or_default());
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// `path` made absolute against the current directory, with `.` and `..` resolved lexically.
pub(crate) fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        return normalize(path);
    }
    match std::env::current_dir() {
        Ok(dir) => normalize(&dir.join(path)),
        Err(_) => normalize(path),
    }
}

/// `path` with `.` removed and `..` folded into the component before it. A `..` at the root stays at the root.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                let last = out.components().next_back();
                match last {
                    Some(Component::Normal(_)) => {
                        out.pop();
                    }
                    Some(Component::RootDir | Component::Prefix(_)) => {}
                    _ => out.push(".."),
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizing_folds_dots() {
        assert_eq!(normalize(Path::new("/a/./b/../c")), PathBuf::from("/a/c"));
        assert_eq!(normalize(Path::new("/../a")), PathBuf::from("/a"));
        assert_eq!(normalize(Path::new("a/../../b")), PathBuf::from("../b"));
    }

    #[test]
    fn samples_inside_the_folder_are_relative() {
        let folder = absolute(Path::new("songs"));
        assert_eq!(sample_path(Some(&folder), &folder.join("Samples").join("kick.wav")), "Samples/kick.wav");
        let outside = absolute(Path::new("elsewhere/kick.wav"));
        assert_eq!(sample_path(Some(&folder), &outside), outside.to_string_lossy());
        assert_eq!(sample_path(None, &outside), outside.to_string_lossy());
        // A sibling folder whose name starts the same way is outside.
        let sibling = absolute(Path::new("songs2/kick.wav"));
        assert_eq!(sample_path(Some(&folder), &sibling), sibling.to_string_lossy());
    }

    #[test]
    fn stored_paths_resolve() {
        let folder = absolute(Path::new("songs"));
        assert_eq!(resolve_sample_path(Some(&folder), "Samples/kick.wav"), Some(folder.join("Samples").join("kick.wav")));
        assert_eq!(resolve_sample_path(Some(&folder), "../kick.wav"), Some(absolute(Path::new("kick.wav"))));
        assert_eq!(resolve_sample_path(Some(&folder), "factory:drums/kick.wav"), None);
        assert!(is_factory("factory:x"));
    }
}
