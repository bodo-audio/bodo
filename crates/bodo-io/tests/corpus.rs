//! The migration corpus (project.md section 18.1): every fixture in `tests/corpus` must load, validate, and survive
//! being saved and loaded again.
//!
//! `v1-full.json` is generated from [`common::full`] and must come back byte for byte, so the format never drifts by
//! accident. After a deliberate model change, regenerate it with `BODO_BLESS=1 cargo test -p bodo-io --test corpus`,
//! keep the previous file in the corpus under a name that says what it predates, and add a migration if older files
//! need one. The other files are frozen history: files real users may have.

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
        if version["format_version"] == FORMAT_VERSION && path.ends_with("v1-full.json") {
            assert_eq!(to_json(&project).unwrap(), text, "{} does not round-trip", path.display());
        }
        assert_eq!(from_json(&to_json(&project).unwrap()).unwrap(), project, "{} changes when saved again", path.display());
        count += 1;
    }
    assert!(count >= 1, "the corpus is empty");
}

#[test]
fn samplers_saved_before_one_shot_play_one_shots() {
    let text = std::fs::read_to_string(corpus().join("v1-before-one-shot.json")).unwrap();
    assert!(!text.contains("oneShot"));
    let project = from_json(&text).unwrap();
    let one_shots = project.channels.values().filter_map(|c| match &c.kind {
        bodo_model::channel::ChannelKind::Sampler(params) => Some(params.one_shot),
        _ => None,
    });
    assert!(one_shots.into_iter().all(|on| on), "a missing field takes its default");
}
