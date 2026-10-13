//! Demo projects, built from the factory content (project.md section 14.1, ADR 0029).
//!
//! Each demo is a script of commands, so it is made of factory sounds only, released as CC0 like them, and always
//! matches the current project format. On first run the app writes the demos into the user library's Demo projects
//! folder; opening one starts a new project from it, so the demo itself stays as it was.

use std::path::{Path, PathBuf};

use bodo_model::Project;

use crate::error::CommandError;
use crate::session::{Call, Env, Session};

/// Every demo: its name and its script, `{ "title": ..., "calls": [...] }`.
pub const DEMOS: [(&str, &str); 1] = [("Bodo Beat", include_str!("bodo_beat.json"))];

/// Builds the project `script` describes, with the factory content of `env`.
///
/// Complexity: O(c) in the script's commands.
pub fn build(env: &Env, script: &str) -> Result<Project, CommandError> {
    let value: serde_json::Value = serde_json::from_str(script).map_err(|e| CommandError::invalid(format!("the demo script is not JSON: {e}")))?;
    let calls: Vec<Call> = serde_json::from_value(value.get("calls").cloned().unwrap_or_default())
        .map_err(|e| CommandError::invalid(format!("the demo script has bad calls: {e}")))?;
    let mut session = Session::new(Env { user: None, ..env.clone() });
    for call in &calls {
        session.invoke(call)?;
    }
    Ok((**session.project()).clone())
}

/// Writes every demo that `folder` doesn't have yet, as `<name>.bodo`, and returns the files it wrote. Demos need the
/// factory content, so without it nothing is written.
///
/// Does I/O. Complexity: O(c) in the demos' commands, plus saving them.
pub fn install(folder: &Path, env: &Env) -> Result<Vec<PathBuf>, CommandError> {
    if env.factory.is_none() {
        return Ok(Vec::new());
    }
    let mut written = Vec::new();
    for (name, script) in DEMOS {
        let path = folder.join(format!("{name}.bodo"));
        if path.exists() {
            continue;
        }
        let project = build(env, script)?;
        bodo_io::save(&project, &path, &bodo_io::SaveOptions { from: None, backups: 0 })
            .map_err(|e| CommandError::new(crate::error::ErrorCode::Io, e.to_string()))?;
        written.push(path);
    }
    Ok(written)
}
