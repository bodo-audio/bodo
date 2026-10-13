//! Helpers shared by the command line's tests.

#![allow(dead_code, clippy::unwrap_used, clippy::panic, reason = "shared test helpers; not every test uses every helper")]

use std::path::{Path, PathBuf};

use bodo_engine::{Call, Env, Session};
use serde_json::{Value, json};

/// A deterministic session with the factory content.
pub fn session() -> Session {
    Session::deterministic(Env { factory: bodo_engine::locate_factory(), library: Vec::new() })
}

/// The 4-pattern beat of the M2 exit criterion, built with the engine.
pub fn beat() -> Session {
    let script: Value = serde_json::from_str(include_str!("../../../../crates/bodo-engine/tests/fixtures/beat.json")).unwrap();
    let calls: Vec<Call> = serde_json::from_value(script["calls"].clone()).unwrap();
    let mut session = session();
    for call in calls {
        session.invoke(&call).unwrap_or_else(|e| panic!("{} failed: {e}", call.id));
    }
    session
}

/// The beat saved as `name` in `dir`.
pub fn saved_beat(dir: &Path, name: &str) -> (Session, PathBuf) {
    let mut session = beat();
    let path = dir.join(name);
    session.execute("project.save", json!({ "path": path.display().to_string() })).unwrap();
    (session, path)
}

/// A fresh folder for one test's files.
pub fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bodo-cli-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
