//! The M2 exit criterion through the interface: build a 4-pattern beat by clicking steps in the channel rack and
//! loading the 808 from the browser, arrange it by clicking the playlist, then save it and reopen it identically.
//! Only saving goes around the file dialog, which a test can't click.

#![allow(clippy::unwrap_used, clippy::panic, reason = "tests")]

mod common;

use bodo_engine::{Env, Session, inspect};
use common::{click, click_at, double_click_at, element, steps};
use egui::{Theme, pos2};
use serde_json::json;

/// The same beat through the session's commands.
fn by_commands() -> Session {
    let mut session = Session::deterministic(Env { factory: bodo_engine::locate_factory(), library: Vec::new(), user: None });
    let calls = [
        ("pattern.set_steps", json!({ "channel": "Kick", "steps": "x.......x......." })),
        ("pattern.set_steps", json!({ "channel": "HiHat", "steps": "..x...x...x...x." })),
        ("pattern.new", json!({})),
        ("pattern.set_steps", json!({ "channel": "Kick", "steps": "x...x...x...x..." })),
        ("pattern.set_steps", json!({ "channel": "Clap", "steps": "....x.......x..." })),
        ("pattern.set_steps", json!({ "channel": "HiHat", "steps": "x.x.x.x.x.x.x.x." })),
        ("channel.add", json!({ "sample": "factory:Packs/Bodo Drums/808 Bass.wav" })),
        ("pattern.set_steps", json!({ "channel": "808 Bass", "steps": "x.....x...x....." })),
        ("pattern.new", json!({})),
        ("pattern.set_steps", json!({ "channel": "Kick", "steps": "x.......x.x....." })),
        ("pattern.set_steps", json!({ "channel": "Snare", "steps": "....x...x.x.xxxx" })),
        ("pattern.new", json!({})),
        ("pattern.set_steps", json!({ "channel": "Kick", "steps": "x..............." })),
        ("pattern.set_steps", json!({ "channel": "808 Bass", "steps": "x..............." })),
    ];
    for (id, params) in calls {
        session.execute(id, params).unwrap();
    }
    for bar in 1..=4 {
        session.execute("clip.place", json!({ "track": 1, "pattern": bar, "start": bar.to_string() })).unwrap();
    }
    session
}

/// Everything the summary says about the document, without the lines about the file and the transport.
fn body(session: &Session) -> String {
    inspect::project(session).text.lines().skip(2).collect::<Vec<_>>().join("\n").replace(" (current)", "")
}

#[test]
fn a_four_pattern_beat_built_by_clicking_saves_and_reopens_identically() {
    let mut harness = common::harness(common::app(Theme::Dark), Theme::Dark, false);
    harness.run_steps(4);

    // Pattern 1.
    steps(&mut harness, "Kick", &[1, 9]);
    steps(&mut harness, "HiHat", &[3, 7, 11, 15]);
    // Pattern 2, with the 808 loaded from the browser by double-clicking it.
    click(&mut harness, "New pattern");
    steps(&mut harness, "Kick", &[1, 5, 9, 13]);
    steps(&mut harness, "Clap", &[5, 13]);
    steps(&mut harness, "HiHat", &[1, 3, 5, 7, 9, 11, 13, 15]);
    click(&mut harness, "Folder Bodo Drums");
    let bass = element(&harness, "Sound 808 Bass").center();
    double_click_at(&mut harness, bass);
    steps(&mut harness, "808 Bass", &[1, 7, 11]);
    // Pattern 3.
    click(&mut harness, "New pattern");
    steps(&mut harness, "Kick", &[1, 9, 11]);
    steps(&mut harness, "Snare", &[5, 9, 11, 13, 14, 15, 16]);
    // Pattern 4.
    click(&mut harness, "New pattern");
    steps(&mut harness, "Kick", &[1]);
    steps(&mut harness, "808 Bass", &[1]);

    // Arrange: each pattern on track 1, a bar each, with the draw tool.
    for bar in 1..=4 {
        click(&mut harness, &format!("Pattern Pattern {bar}"));
        let canvas = element(&harness, "Playlist canvas");
        let bar_width = 72.0;
        click_at(&mut harness, pos2(canvas.left() + (bar as f32 - 1.0) * bar_width + 10.0, canvas.top() + 20.0));
    }

    let app = harness.state_mut();
    assert_eq!(app.session.project().patterns.len(), 4);
    assert_eq!(body(&app.session), body(&by_commands()), "clicking builds exactly the beat the commands build");

    let dir = std::env::temp_dir().join(format!("bodo-ui-{}-beat", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Clicked Beat.bodo");
    app.run("project.save", json!({ "path": path.display().to_string() })).unwrap();
    let saved = app.session.project().clone();
    let mut reopened = Session::deterministic(Env { factory: bodo_engine::locate_factory(), library: Vec::new(), user: None });
    reopened.execute("project.open", json!({ "path": path.display().to_string() })).unwrap();
    assert_eq!(**reopened.project(), *saved, "the reopened project is the saved one");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_stroke_of_painted_steps_is_one_undo_step() {
    let mut harness = common::harness(common::app(Theme::Dark), Theme::Dark, false);
    harness.run_steps(4);
    let (from, to) = (element(&harness, "HiHat step 1").center(), element(&harness, "HiHat step 8").center());
    harness.event(egui::Event::PointerMoved(from));
    harness.step();
    harness.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    harness.step();
    for i in 1..=8 {
        harness.event(egui::Event::PointerMoved(from + (to - from) * (i as f32 / 8.0)));
        harness.step();
    }
    harness.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    harness.run_steps(2);
    let app = harness.state();
    assert!(inspect::project(&app.session).text.contains("HiHat        xxxxxxxx........"), "{}", inspect::project(&app.session).text);
    assert_eq!(app.session.undo_labels().count(), 1);
}
