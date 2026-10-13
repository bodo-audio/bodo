//! Helpers shared by the engine's integration tests.

#![allow(dead_code, clippy::unwrap_used, clippy::panic, reason = "shared test helpers; not every test uses every helper")]

use std::path::PathBuf;

use bodo_engine::{Env, Session};
use serde_json::Value;

/// The repository's factory content.
pub fn factory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/factory").canonicalize().unwrap()
}

/// A deterministic session with the factory content.
pub fn session() -> Session {
    Session::deterministic(Env { factory: Some(factory()), library: Vec::new() })
}

/// Runs a command that must succeed and returns its result.
pub fn run(session: &mut Session, id: &str, params: Value) -> Value {
    match session.execute(id, params.clone()) {
        Ok(output) => output.result,
        Err(error) => panic!("{id} {params} failed:\n{error}"),
    }
}

/// A fresh folder for one test's files.
pub fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bodo-engine-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
