//! Setting up the user library on first run (project.md section 14.1, ADR 0029).

use std::path::{Path, PathBuf};

use bodo_engine::library::UserLibrary;
use bodo_engine::{Env, demo};

/// The environment variable that puts the user library somewhere else, for example in tests.
pub const LIBRARY_VAR: &str = "BODO_LIBRARY";

/// Where the user library goes by default: `BODO_LIBRARY` when set, otherwise a Bodo folder in the user's documents
/// (or home folder). `None` when the system has neither.
pub fn default_root() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(LIBRARY_VAR).filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(path));
    }
    let dirs = directories::UserDirs::new()?;
    Some(dirs.document_dir().unwrap_or_else(|| dirs.home_dir()).join("Bodo"))
}

/// Makes the library's folders at `root` when they are missing, and writes the demo projects built from the factory
/// content at `factory`.
///
/// Does I/O. Complexity: O(1) folders, plus building the demos the first time.
pub fn prepare(root: &Path, factory: Option<&Path>) -> Result<(), String> {
    let library = UserLibrary::new(root);
    library.ensure().map_err(|e| format!("could not make the library at {}: {e}", root.display()))?;
    let env = Env { factory: factory.map(Path::to_path_buf), library: Vec::new(), user: None };
    demo::install(&library.demos(), &env).map_err(|e| format!("could not write the demo projects: {e}"))?;
    Ok(())
}
