//! Quitting with unsaved changes: the first close request asks, and "Don't save" really closes, however many close
//! requests the window system sends after it.

#![allow(clippy::unwrap_used, clippy::panic, reason = "tests")]

mod common;

use egui::Theme;
use egui_kittest::kittest::Queryable;
use serde_json::json;

#[test]
fn dont_save_closes_instead_of_asking_again() {
    let mut app = common::app(Theme::Dark);
    app.run("project.set_tempo", json!({ "bpm": 128 })).unwrap();
    assert!(app.session.is_dirty());
    let mut h = common::harness(app, Theme::Dark, false);
    h.run_steps(2);

    // The window's close button: refused, and the dialog asks.
    assert!(!h.state_mut().allow_close(), "unsaved changes keep the window open");
    h.run_steps(2);
    h.get_by_label("Don't save").click();
    h.run_steps(2);
    assert!(h.state_mut().take_quit(), "the answer is a quit");
    // The app closes its window, and the window system sends the close request back: it goes through now.
    assert!(h.state_mut().allow_close(), "the close the app asked for isn't refused again");
    assert!(h.state_mut().allow_close());
}

#[test]
fn cancel_keeps_the_window_open_and_asks_next_time() {
    let mut app = common::app(Theme::Dark);
    app.run("project.set_tempo", json!({ "bpm": 128 })).unwrap();
    let mut h = common::harness(app, Theme::Dark, false);
    h.run_steps(2);
    assert!(!h.state_mut().allow_close());
    h.run_steps(2);
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    assert!(!h.state_mut().take_quit(), "cancel is no quit");
    assert!(!h.state_mut().allow_close(), "the next close asks again");
}
