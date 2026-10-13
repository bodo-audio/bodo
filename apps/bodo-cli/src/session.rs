//! Sessions on project files, with Bodo's factory content.

use std::path::Path;

use anyhow::{Context, Result};
use bodo_engine::{Env, Session};
use serde_json::json;

/// The environment the command line works in: the factory content where [`bodo_engine::locate_factory`] finds it,
/// and no sample folders of the user's.
pub fn env() -> Env {
    Env { factory: bodo_engine::locate_factory(), library: Vec::new(), user: None }
}

/// A session on the project at `path`, or on a new project from the basic template when `path` is `None`.
///
/// Does I/O: reads the project file.
pub fn open(path: Option<&Path>) -> Result<Session> {
    let mut session = Session::new(env());
    if let Some(path) = path {
        session.execute("project.open", json!({ "path": path.display().to_string() })).with_context(|| format!("cannot open {}", path.display()))?;
    }
    Ok(session)
}
