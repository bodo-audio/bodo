//! Helpers shared by the interface tests: the app in a kittest harness, and input as a person gives it.

#![allow(dead_code, clippy::unwrap_used, clippy::panic, reason = "shared test helpers; not every test uses every helper")]

use bodo_engine::{Env, Session};
use bodo_ui::{BodoApp, Prefs, Services};
use egui::{Event, Modifiers, PointerButton, Pos2, Rect, Theme, vec2};
use egui_kittest::Harness;

/// The app on a deterministic session with the factory content, without audio.
pub fn app(theme: Theme) -> BodoApp {
    let env = Env { factory: bodo_engine::locate_factory(), library: Vec::new() };
    let prefs =
        Prefs { theme: if theme == Theme::Light { bodo_ui::theme::ThemePref::Light } else { bodo_ui::theme::ThemePref::Dark }, ..Prefs::default() };
    BodoApp::new(Session::deterministic(env), prefs, Services::default(), false)
}

/// A harness that draws `app` at 1440 by 900 points, with frames of a sixtieth of a second.
pub fn harness(app: BodoApp, theme: Theme, wgpu: bool) -> Harness<'static, BodoApp> {
    let builder = Harness::builder().with_size(vec2(1440.0, 900.0)).with_theme(theme).with_step_dt(1.0 / 60.0).with_max_steps(120);
    let builder = if wgpu { builder.wgpu() } else { builder };
    builder.build_ui_state(
        |ui, app: &mut BodoApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    )
}

/// Where the element labeled `label` was drawn last frame.
pub fn element(harness: &Harness<'_, BodoApp>, label: &str) -> Rect {
    let found = bodo_ui::control::elements(&harness.ctx);
    found.iter().rev().find(|e| e.label == label).map(|e| e.rect).unwrap_or_else(|| {
        let labels: Vec<&str> = found.iter().map(|e| e.label.as_str()).collect();
        panic!("no element `{label}`; there are {labels:?}")
    })
}

/// A click at `at`: the pointer moves there, presses and releases, a frame each.
pub fn click_at(harness: &mut Harness<'_, BodoApp>, at: Pos2) {
    harness.event(Event::PointerMoved(at));
    harness.step();
    for pressed in [true, false] {
        harness.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        harness.step();
    }
    harness.step();
}

/// A double click at `at`, after a pause long enough that it isn't taken as part of an earlier click.
pub fn double_click_at(harness: &mut Harness<'_, BodoApp>, at: Pos2) {
    harness.run_steps(30);
    harness.event(Event::PointerMoved(at));
    harness.step();
    for pressed in [true, false, true, false] {
        harness.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        harness.step();
    }
    harness.step();
}

/// Clicks the element labeled `label`.
pub fn click(harness: &mut Harness<'_, BodoApp>, label: &str) {
    let at = element(harness, label).center();
    click_at(harness, at);
}

/// Clicks steps `steps` (counted from 1) of channel `channel` in the channel rack.
pub fn steps(harness: &mut Harness<'_, BodoApp>, channel: &str, steps: &[u32]) {
    for step in steps {
        click(harness, &format!("{channel} step {step}"));
    }
}

/// A press of `button` at `from`, moves along `path` a frame each, and a release at the last point.
pub fn drag(harness: &mut Harness<'_, BodoApp>, button: PointerButton, from: Pos2, path: &[Pos2], modifiers: Modifiers) {
    harness.event(Event::PointerMoved(from));
    harness.step();
    harness.event_modifiers(Event::PointerButton { pos: from, button, pressed: true, modifiers }, modifiers);
    harness.step();
    let mut last = from;
    for &point in path {
        harness.event_modifiers(Event::PointerMoved(point), modifiers);
        harness.step();
        last = point;
    }
    harness.event_modifiers(Event::PointerButton { pos: last, button, pressed: false, modifiers }, modifiers);
    harness.step();
    harness.step();
}

/// `count` points from `from` to `to`, the last one `to`.
pub fn line(from: Pos2, to: Pos2, count: usize) -> Vec<Pos2> {
    (1..=count).map(|i| from + (to - from) * (i as f32 / count as f32)).collect()
}

/// The app with the M2 beat built through commands.
pub fn beat_app() -> BodoApp {
    let mut app = app(Theme::Dark);
    let script: serde_json::Value = serde_json::from_str(include_str!("../../../bodo-engine/tests/fixtures/beat.json")).unwrap();
    let calls: Vec<bodo_engine::Call> = serde_json::from_value(script["calls"].clone()).unwrap();
    for call in calls {
        app.invoke(call).unwrap();
    }
    app
}
