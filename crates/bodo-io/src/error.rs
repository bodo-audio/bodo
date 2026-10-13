//! Everything that can go wrong reading or writing a project.

use std::path::{Path, PathBuf};

use bodo_model::ValidationError;

use crate::json::FORMAT_VERSION;

/// Why a project could not be read or written. Every message names the file, so people and agents can act on it.
#[derive(Debug, thiserror::Error)]
pub enum IoError {
    /// The file system refused an operation.
    #[error("could not {action} {}: {source}", .path.display())]
    Io {
        /// What was being done, such as "read" or "create".
        action: &'static str,
        /// The file or folder.
        path: PathBuf,
        /// The operating system's error.
        #[source]
        source: std::io::Error,
    },
    /// `project.json` is not valid JSON, or its data doesn't fit the project model.
    #[error("{} has bad project data at line {line}, column {column}: {message}", describe(.path.as_deref()))]
    Json {
        /// The project file, when there is one.
        path: Option<PathBuf>,
        /// The line of the problem, from 1, or 0 when it isn't known (after a migration).
        line: usize,
        /// The column of the problem, from 1, or 0 when it isn't known.
        column: usize,
        /// What is wrong.
        message: String,
    },
    /// The file is not a Bodo project: its `format` is missing or something else.
    #[error("{} is not a Bodo project (its format is {found}, not \"bodo\")", describe(.path.as_deref()))]
    NotBodo {
        /// The project file, when there is one.
        path: Option<PathBuf>,
        /// What the file says its format is.
        found: String,
    },
    /// The `format_version` is missing or not a positive integer.
    #[error("{} has no valid format_version (found {found})", describe(.path.as_deref()))]
    BadVersion {
        /// The project file, when there is one.
        path: Option<PathBuf>,
        /// What the file has instead.
        found: String,
    },
    /// The file was saved by a newer Bodo.
    #[error(
        "{} was saved by a newer version of Bodo (format version {found}); this version reads format versions up to {}. Update Bodo to open it",
        describe(.path.as_deref()),
        FORMAT_VERSION
    )]
    TooNew {
        /// The project file, when there is one.
        path: Option<PathBuf>,
        /// The file's format version.
        found: u64,
    },
    /// A migration from an older format version failed.
    #[error("{} could not be migrated from format version {from}: {message}", describe(.path.as_deref()))]
    Migration {
        /// The project file, when there is one.
        path: Option<PathBuf>,
        /// The version the failing step migrates from.
        from: u32,
        /// What went wrong.
        message: String,
    },
    /// The project breaks invariants of the model, so it is refused.
    #[error("{} breaks {} invariant(s): {}", describe(.path.as_deref()), .error.0.len(), list(.error))]
    Invalid {
        /// The project file, when there is one.
        path: Option<PathBuf>,
        /// Every broken invariant.
        error: ValidationError,
    },
    /// A container entry the format requires is missing.
    #[error("{} has no {entry}", .path.display())]
    Missing {
        /// The project file or folder.
        path: PathBuf,
        /// The entry, such as `project.json`.
        entry: &'static str,
    },
    /// The zip container is damaged or can't be written.
    #[error("{} is not a readable .bodo container: {source}", .path.display())]
    Zip {
        /// The project file.
        path: PathBuf,
        /// The zip library's error.
        #[source]
        source: zip::result::ZipError,
    },
    /// The target of a conversion already exists, and conversions never overwrite.
    #[error("{} already exists", .path.display())]
    Exists {
        /// The existing file or folder.
        path: PathBuf,
    },
    /// A MIDI file could not be read or written.
    #[error("{}: {message}", .path.as_deref().map_or_else(|| String::from("the MIDI file"), |p| p.display().to_string()))]
    Midi {
        /// The MIDI file, when there is one.
        path: Option<PathBuf>,
        /// What is wrong.
        message: String,
    },
    /// The project could not be turned into JSON.
    #[error("the project could not be written as JSON: {message}")]
    Encode {
        /// What went wrong.
        message: String,
    },
}

impl IoError {
    /// An I/O error while doing `action` to `path`.
    pub(crate) fn io(action: &'static str, path: &Path, source: std::io::Error) -> Self {
        Self::Io { action, path: path.to_path_buf(), source }
    }

    /// A zip error in `path`.
    pub(crate) fn zip(path: &Path, source: zip::result::ZipError) -> Self {
        Self::Zip { path: path.to_path_buf(), source }
    }

    /// The same error, naming `path` as the project file when it didn't name one yet.
    pub(crate) fn at(self, path: &Path) -> Self {
        let named = |current: Option<PathBuf>| current.or_else(|| Some(path.to_path_buf()));
        match self {
            Self::Json { path: p, line, column, message } => Self::Json { path: named(p), line, column, message },
            Self::NotBodo { path: p, found } => Self::NotBodo { path: named(p), found },
            Self::BadVersion { path: p, found } => Self::BadVersion { path: named(p), found },
            Self::TooNew { path: p, found } => Self::TooNew { path: named(p), found },
            Self::Migration { path: p, from, message } => Self::Migration { path: named(p), from, message },
            Self::Invalid { path: p, error } => Self::Invalid { path: named(p), error },
            Self::Midi { path: p, message } => Self::Midi { path: named(p), message },
            other => other,
        }
    }
}

/// How a message names the project: its path, or "the project" when it has none.
fn describe(path: Option<&Path>) -> String {
    path.map_or_else(|| String::from("the project"), |p| p.display().to_string())
}

/// Every violation, separated by semicolons.
fn list(error: &ValidationError) -> String {
    error.0.iter().map(ToString::to_string).collect::<Vec<_>>().join("; ")
}
