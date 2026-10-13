//! Atomic writes: a temporary file next to the target, flushed to disk, then renamed over it. A crash leaves either
//! the old file or the new one, never a mix.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::IoError;

/// Counts temporary files, so two writes in one process never pick the same name.
static NEXT: AtomicU64 = AtomicU64::new(0);

/// A temporary path next to `target`: hidden, and ending in `.tmp`.
pub(crate) fn temp_path(target: &Path) -> PathBuf {
    let name = target.file_name().map_or_else(|| String::from("project"), |n| n.to_string_lossy().into_owned());
    let unique = NEXT.fetch_add(1, Ordering::Relaxed);
    target.with_file_name(format!(".{name}.{}.{unique}.tmp", std::process::id()))
}

/// Whether `name` is one of the temporary names [`temp_path`] makes.
pub(crate) fn is_temp_name(name: &str) -> bool {
    name.starts_with('.') && name.ends_with(".tmp")
}

/// Removes a temporary file or folder when dropped, unless it was kept.
pub(crate) struct TempGuard {
    path: Option<PathBuf>,
}

impl TempGuard {
    pub(crate) fn new(path: &Path) -> Self {
        Self { path: Some(path.to_path_buf()) }
    }

    /// Keeps the file: it has been renamed into place.
    pub(crate) fn keep(mut self) {
        self.path = None;
    }
}

impl Drop for TempGuard {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            // Best effort: a leftover hidden temporary file is harmless and is skipped by every reader.
            let _ = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
        }
    }
}

/// Writes `bytes` to `target` atomically.
///
/// Complexity: O(bytes).
pub(crate) fn write(target: &Path, bytes: &[u8]) -> Result<(), IoError> {
    let temp = temp_path(target);
    let guard = TempGuard::new(&temp);
    let mut file = File::create(&temp).map_err(|err| IoError::io("create", &temp, err))?;
    file.write_all(bytes).map_err(|err| IoError::io("write", &temp, err))?;
    file.sync_all().map_err(|err| IoError::io("flush", &temp, err))?;
    drop(file);
    replace(&temp, target)?;
    guard.keep();
    Ok(())
}

/// Renames `temp` over `target` and flushes the folder, so the rename itself survives a crash.
pub(crate) fn replace(temp: &Path, target: &Path) -> Result<(), IoError> {
    std::fs::rename(temp, target).map_err(|err| IoError::io("replace", target, err))?;
    if let Some(folder) = target.parent() {
        sync_folder(folder)?;
    }
    Ok(())
}

/// Flushes a folder's entries to disk. Only Unix can open a folder for this; elsewhere the rename is enough.
pub(crate) fn sync_folder(folder: &Path) -> Result<(), IoError> {
    #[cfg(unix)]
    {
        let folder = if folder.as_os_str().is_empty() { Path::new(".") } else { folder };
        File::open(folder).and_then(|dir| dir.sync_all()).map_err(|err| IoError::io("flush", folder, err))?;
    }
    #[cfg(not(unix))]
    let _ = folder;
    Ok(())
}
