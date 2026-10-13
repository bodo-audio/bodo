//! The migration corpus (project.md section 18.1): every fixture in `tests/corpus` must load. A file of the current
//! version must also come back byte for byte when written again, so the format never drifts by accident.
//!
//! `v1-full.json` is generated from [`common::full`]. After a deliberate model change, regenerate it with
//! `BODO_BLESS=1 cargo test -p bodo-io --test corpus`, and add a migration if older files need one.

#![allow(clippy::unwrap_used, clippy::panic, reason = "test helpers outside #[test] functions")]

mod common;

use std::path::PathBuf;

use bodo_io::{FORMAT_VERSION, from_json, to_json};

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("corpus")
}

#[test]
fn the_full_fixture_matches_its_corpus_file() {
    let (project, _) = common::full();
    let json = to_json(&project).unwrap();
    let path = corpus().join("v1-full.json");
    if std::env::var_os("BODO_BLESS").is_some() {
        std::fs::write(&path, &json).unwrap();
    }
    let stored = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(stored == json, "{} is out of date; regenerate it with BODO_BLESS=1 if the change is deliberate", path.display());
    assert_eq!(from_json(&stored).unwrap(), project);
}

#[test]
fn every_corpus_file_loads() {
    if std::env::var_os("BODO_BLESS").is_some() {
        // The other test is rewriting the corpus right now.
        return;
    }
    let mut count = 0;
    for entry in std::fs::read_dir(corpus()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let project = from_json(&text).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
        project.validate().unwrap();
        let version: serde_json::Value = serde_json::from_str(&text).unwrap();
        if version["format_version"] == FORMAT_VERSION {
            assert_eq!(to_json(&project).unwrap(), text, "{} does not round-trip", path.display());
        }
        count += 1;
    }
    assert!(count >= 1, "the corpus is empty");
}
