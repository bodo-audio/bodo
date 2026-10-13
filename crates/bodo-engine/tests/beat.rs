//! The M2 exit criterion through commands only: build a 4-pattern beat with the step sequencer and the sampler,
//! arrange it, save it, and reopen it identically.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use bodo_engine::{Call, Session, inspect};
use common::{run, session, temp_dir};
use serde_json::{Value, json};

/// The calls of the beat script, shared with the MCP agent-task test.
pub fn beat_calls() -> Vec<Call> {
    let script: Value = serde_json::from_str(include_str!("fixtures/beat.json")).unwrap();
    serde_json::from_value(script["calls"].clone()).unwrap()
}

fn build(session: &mut Session) {
    for call in beat_calls() {
        if let Err(error) = session.invoke(&call) {
            panic!("{} {} failed:\n{error}", call.id, call.params);
        }
    }
}

#[test]
fn a_four_pattern_beat_saves_and_reopens_identically() {
    let mut first = session();
    build(&mut first);
    let summary = inspect::project(&first).text;
    for expected in [
        "Project \"First Beat\": 96 BPM",
        "Verse: 1 bar, 17 notes",
        "Kick         x...x...x...x...",
        "Clap         ....x.......x...",
        "808          x.....x...x.....",
        "Snare        ....x...x.x.xxxx",
        "Drums: Intro at 1:1:0 (2 bars), Verse at 3:1:0 (4 bars), Fill at 7:1:0 (1 bar), Outro at 8:1:0 (1 bar)",
        "song length 8 bars",
    ] {
        assert!(summary.contains(expected), "missing `{expected}` in:\n{summary}");
    }
    assert_eq!(first.project().patterns.len(), 4);
    assert!(first.is_dirty());

    let dir = temp_dir("beat");
    let path = dir.join("First Beat.bodo");
    run(&mut first, "project.save", json!({ "path": path.display().to_string() }));
    assert!(!first.is_dirty());
    assert_eq!(first.file(), Some(path.as_path()));

    let mut second = session();
    run(&mut second, "project.open", json!({ "path": path.display().to_string() }));
    assert_eq!(**second.project(), **first.project(), "the reopened project is the saved one");
    assert!(!second.is_dirty());
    let reopened = inspect::project(&second).text;
    // The current pattern is editor state, not part of the document, so it isn't saved.
    let body = |text: &str| text.lines().skip(2).collect::<Vec<_>>().join("\n").replace(" (current)", "");
    assert_eq!(body(&reopened), body(&summary));

    // Saving the reopened project again writes the same bytes.
    let again = dir.join("Again.bodo");
    run(&mut second, "project.save_as", json!({ "path": again.display().to_string() }));
    assert_eq!(std::fs::read(&path).unwrap(), std::fs::read(&again).unwrap());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_beat_undoes_back_to_the_template() {
    let mut session = session();
    let start = session.project().clone();
    build(&mut session);
    let steps = session.undo_labels().count();
    assert!(steps >= 20, "{steps} undo steps");
    while session.undo().is_some() {}
    assert_eq!(**session.project(), *start);
    while session.redo().is_some() {}
    let mut rebuilt = common::session();
    build(&mut rebuilt);
    assert_eq!(**session.project(), **rebuilt.project());
}

#[test]
fn unsaved_work_is_never_dropped_silently() {
    let mut session = session();
    run(&mut session, "pattern.toggle_step", json!({ "channel": "Kick", "step": 1 }));
    let refused = session.execute("project.new", json!({})).unwrap_err();
    assert_eq!(refused.code, bodo_engine::ErrorCode::Rejected);
    assert!(refused.to_string().contains("unsaved changes"), "{refused}");
    run(&mut session, "project.new", json!({ "discardChanges": true }));
    assert!(!session.is_dirty());
}
