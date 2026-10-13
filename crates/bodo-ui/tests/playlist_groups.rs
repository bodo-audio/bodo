//! Track groups, track solo, the time selection, consolidating clips, curve tension and following the playhead on the
//! Playlist (project.md section 10.3), driven with real pointer and key events. Two snapshots show a track group and a
//! time selection in the dark and light themes.

#![allow(clippy::unwrap_used, clippy::panic, reason = "tests")]

mod common;

use bodo_kit_core::Tick;
use bodo_model::automation::CurveShape;
use bodo_model::playlist::ClipSource;
use bodo_ui::BodoApp;
use common::{click, drag, element, line};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, Theme, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

/// Points per bar in these tests.
const BAR: f32 = 48.0;

fn setup() -> (Harness<'static, BodoApp>, Rect) {
    let mut app = common::beat_app();
    app.view.playlist.zoom = BAR / 3840.0;
    let mut harness = common::harness(app, Theme::Dark, false);
    harness.run_steps(4);
    let canvas = element(&harness, "Playlist canvas");
    (harness, canvas)
}

fn x_of(canvas: Rect, bar: f32) -> f32 {
    canvas.left() + (bar - 1.0) * BAR
}

fn right_click(h: &mut Harness<'_, BodoApp>, at: Pos2) {
    h.event(Event::PointerMoved(at));
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        h.step();
    }
    h.step();
}

/// The tracks' names, groups and solo, in playlist order.
fn tracks(h: &Harness<'_, BodoApp>) -> Vec<(String, Option<String>, bool)> {
    let project = h.state().session.project();
    project.track_order.iter().map(|t| &project.tracks[t]).map(|t| (t.name.clone(), t.group.clone(), t.solo)).collect()
}

/// `(start, length)` of each clip on track `index` (from 0).
fn clips_on(h: &Harness<'_, BodoApp>, index: usize) -> Vec<(i64, i64)> {
    let project = h.state().session.project();
    project.tracks[&project.track_order[index]].clips.iter().map(|c| (c.start.0, c.len.0)).collect()
}

fn steps(h: &Harness<'_, BodoApp>) -> usize {
    h.state().session.undo_labels().count()
}

fn labels(h: &Harness<'_, BodoApp>) -> Vec<String> {
    bodo_ui::control::elements(&h.ctx).into_iter().map(|e| e.label).collect()
}

#[test]
fn a_track_joins_the_one_above_in_a_group_that_folds_mutes_and_solos() {
    let (mut h, _) = setup();
    let header = element(&h, "Track 2 on");
    right_click(&mut h, header.left_center() - vec2(50.0, 0.0));
    h.get_by_label("Group with track above").click();
    h.run_steps(3);
    let rows = tracks(&h);
    assert_eq!((rows[0].1.as_deref(), rows[1].1.as_deref()), (Some("Group 1"), Some("Group 1")));
    // Folding hides the group's tracks; the group header stays.
    click(&mut h, "Group Group 1");
    h.run_steps(2);
    let shown = labels(&h);
    assert!(!shown.iter().any(|l| l == "Drums on" || l == "Track 2 on"), "{shown:?}");
    assert!(shown.iter().any(|l| l == "Track 3 on"));
    click(&mut h, "Group Group 1");
    h.run_steps(2);
    assert!(labels(&h).iter().any(|l| l == "Track 2 on"), "unfolded again");
    // Solo and mute act on every track of the group.
    click(&mut h, "Group Group 1 solo");
    h.run_steps(2);
    let solo: Vec<bool> = tracks(&h).iter().map(|t| t.2).collect();
    assert_eq!(solo[..3], [true, true, false]);
    click(&mut h, "Group Group 1 on");
    h.run_steps(2);
    let project = h.state().session.project();
    assert!(project.track_order.iter().take(2).all(|t| project.tracks[t].muted));
}

#[test]
fn a_tracks_solo_button_solos_it() {
    let (mut h, _) = setup();
    click(&mut h, "Track 3 solo");
    h.run_steps(2);
    assert!(tracks(&h)[2].2);
    click(&mut h, "Track 3 solo");
    h.run_steps(2);
    assert!(!tracks(&h)[2].2);
}

#[test]
fn a_new_group_from_the_menu_takes_the_name_typed_and_renames() {
    let (mut h, _) = setup();
    let header = element(&h, "Track 3 on");
    right_click(&mut h, header.left_center() - vec2(50.0, 0.0));
    h.get_by_label_contains("Add to group").hover();
    h.run_steps(3);
    h.get_by_label("New group...").click();
    h.run_steps(3);
    for _ in 0..6 {
        h.key_press(Key::Backspace);
    }
    h.event(Event::Text(String::from("Keys")));
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(tracks(&h)[2].1.as_deref(), Some("Keys"));
    let band = element(&h, "Group Keys").center();
    right_click(&mut h, band);
    h.get_by_label("Rename group...").click();
    h.run_steps(3);
    for _ in 0..6 {
        h.key_press(Key::Backspace);
    }
    h.event(Event::Text(String::from("Pads")));
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(tracks(&h)[2].1.as_deref(), Some("Pads"));
}

#[test]
fn dragging_the_ruler_selects_time_and_its_menu_deletes_it() {
    let (mut h, canvas) = setup();
    let ruler = element(&h, "Playlist ruler");
    let (from, to) = (pos2(x_of(canvas, 3.0) + 1.0, ruler.center().y), pos2(x_of(canvas, 5.0) + 1.0, ruler.center().y));
    let before = steps(&h);
    drag(&mut h, PointerButton::Primary, from, &line(from, to, 6), Modifiers::NONE);
    assert_eq!(h.state().view.playlist.time_selection, Some((7680, 15_360)), "bars 3 to 5");
    assert_eq!(h.state().session.transport().loop_region, None, "a plain drag selects time, not the loop");
    assert_eq!(steps(&h), before, "selecting time is not an edit");
    right_click(&mut h, pos2(x_of(canvas, 4.0), ruler.center().y));
    h.get_by_label("Delete time").click();
    h.run_steps(3);
    // The Verse ran from bar 3 to bar 7; two bars of it are gone, and everything after moved two bars earlier.
    assert_eq!(clips_on(&h, 0), [(0, 7680), (7680, 7680), (15_360, 3840), (19_200, 3840)]);
    assert_eq!(h.state().view.playlist.time_selection, None);
    assert_eq!(steps(&h), before + 1);
}

#[test]
fn the_selection_menu_cuts_and_cmd_delete_removes_the_time() {
    let (mut h, canvas) = setup();
    let ruler = element(&h, "Playlist ruler");
    let (from, to) = (pos2(x_of(canvas, 2.0) + 1.0, ruler.center().y), pos2(x_of(canvas, 3.0) + 1.0, ruler.center().y));
    drag(&mut h, PointerButton::Primary, from, &line(from, to, 4), Modifiers::NONE);
    right_click(&mut h, pos2(x_of(canvas, 2.5), ruler.center().y));
    h.get_by_label("Cut").click();
    h.run_steps(3);
    assert_eq!(clips_on(&h, 0)[..2], [(0, 3840), (7680, 15_360)], "the Intro's second bar is cut, nothing moved");
    h.key_press_modifiers(Modifiers::COMMAND, Key::Delete);
    h.run_steps(2);
    assert_eq!(clips_on(&h, 0)[..2], [(0, 3840), (3840, 15_360)], "Cmd+Delete closed the gap");
    // Deleting the time also clears the selection.
    assert_eq!(h.state().view.playlist.time_selection, None);
}

#[test]
fn the_clip_menu_consolidates_a_pattern_clip_to_audio() {
    let dir = std::env::temp_dir().join(format!("bodo-ui-consolidate-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (mut h, canvas) = setup();
    h.state_mut().run("project.save_as", json!({ "path": dir.join("Song.bodo").display().to_string() })).unwrap();
    h.run_steps(2);
    right_click(&mut h, pos2(x_of(canvas, 1.5), canvas.top() + 4.0));
    h.get_by_label("Consolidate to audio").click();
    h.run_steps(3);
    let project = h.state().session.project();
    let first = &project.tracks[&project.track_order[0]].clips[0];
    assert_eq!((first.start, first.len), (Tick(0), Tick(7680)));
    assert!(matches!(first.source, ClipSource::Audio(_)), "{:?}", first.source);
    let rendered = bodo_io::project_folder(&dir.join("Song.bodo")).join("Rendered");
    assert_eq!(std::fs::read_dir(&rendered).unwrap().count(), 1, "one WAV in {}", rendered.display());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn dragging_a_tension_handle_bends_the_curve_in_one_step() {
    let mut app = common::beat_app();
    app.view.playlist.zoom = BAR / 3840.0;
    let created = app
        .run("automation.create_for", json!({ "target": { "channel": "Kick", "param": "volume" }, "track": 3, "start": 0, "length": "4 bars" }))
        .unwrap();
    let channel = created["channel"].clone();
    app.run("automation.set_points", json!({ "channel": channel, "points": [{ "at": 0, "value": 0.1 }, { "at": "3:1:0", "value": 0.9 }] })).unwrap();
    let mut h = common::harness(app, Theme::Dark, false);
    h.run_steps(4);
    let name = h.state().session.project().channels.values().find(|c| matches!(c.kind, bodo_model::ChannelKind::Automation(_))).unwrap().name.clone();
    let handle = element(&h, &format!("Bend 1 {name}")).center();
    let before = steps(&h);
    drag(&mut h, PointerButton::Primary, handle, &line(handle, handle + vec2(0.0, 12.0), 4), Modifiers::NONE);
    let lane = match &h.state().session.project().channels.values().find(|c| c.name == name).unwrap().kind {
        bodo_model::ChannelKind::Automation(lane) => lane.clone(),
        other => panic!("{other:?}"),
    };
    match lane.points[0].curve {
        CurveShape::Exp(t) => assert!(t > 0.2, "dragging a rising segment down makes it rise late: {t}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(steps(&h), before + 1, "a drag is one undo step");
}

#[test]
fn auto_scrolling_follows_the_playhead_while_the_song_plays() {
    let (mut h, canvas) = setup();
    click(&mut h, "Auto scroll");
    h.run_steps(2);
    assert!(h.state().view.auto_scroll);
    h.state_mut().run("transport.set_mode", json!({ "mode": "song" })).unwrap();
    h.state_mut().run("transport.set_position", json!({ "at": "40:1:0" })).unwrap();
    h.state_mut().run("transport.play", json!({})).unwrap();
    h.run_steps(3);
    let view = &h.state().view.playlist;
    let shown = view.scroll..view.scroll + f64::from(canvas.width() / view.zoom);
    assert!(shown.contains(&(39.0 * 3840.0)), "{shown:?}");
    h.state_mut().run("transport.stop", json!({})).unwrap();
}

/// The beat with its first two tracks in a soloed group and bars 3 to 5 selected, in `theme`.
fn snapshot(name: &str, theme: Theme) {
    let mut app = common::beat_app();
    app.prefs.theme = if theme == Theme::Light { bodo_ui::theme::ThemePref::Light } else { bodo_ui::theme::ThemePref::Dark };
    app.view.playlist.zoom = BAR / 3840.0;
    app.run("playlist.group_tracks", json!({ "tracks": [1, 2], "name": "Beat" })).unwrap();
    app.run("playlist.set_group_props", json!({ "group": "Beat", "solo": true })).unwrap();
    app.view.playlist.time_selection = Some((7680, 15_360));
    let mut harness = common::harness(app, theme, true);
    harness.run_steps(6);
    harness.snapshot_options(name, &common::snapshot_options(name));
}

#[test]
fn a_track_group_and_a_time_selection_dark() {
    snapshot("playlist_group_dark", Theme::Dark);
}

#[test]
fn a_track_group_and_a_time_selection_light() {
    snapshot("playlist_group_light", Theme::Light);
}
