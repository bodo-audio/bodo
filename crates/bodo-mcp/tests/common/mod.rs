//! Helpers shared by the MCP tests.

#![allow(dead_code, clippy::unwrap_used, clippy::panic, reason = "shared test helpers; not every test uses every helper")]

use std::path::PathBuf;

use bodo_engine::{Call, Env, Session};
use serde_json::Value;

/// A deterministic session with the factory content, as the engine's own tests use.
pub fn session() -> Session {
    Session::deterministic(Env { factory: bodo_engine::locate_factory(), library: Vec::new() })
}

/// The calls of the beat from the M2 exit criterion.
pub fn beat_calls() -> Vec<Call> {
    let script: Value = serde_json::from_str(include_str!("../../../bodo-engine/tests/fixtures/beat.json")).unwrap();
    serde_json::from_value(script["calls"].clone()).unwrap()
}

/// The beat built directly with the engine.
pub fn engine_beat() -> Session {
    let mut session = session();
    for call in beat_calls() {
        session.invoke(&call).unwrap_or_else(|e| panic!("{} failed: {e}", call.id));
    }
    session
}

/// A fresh folder for one test's files.
pub fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bodo-mcp-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
