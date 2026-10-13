//! Audio and automation clips on the playlist (project.md sections 6.7, 10.3 and 10.5), driven with real pointer
//! events: a sound dropped from the browser, an audio clip sliced, a fade handle dragged, automation points dragged,
//! added and removed, and an automation clip made from a channel rack knob.
//!
//! Two snapshots show a waveform with its fades and an automation curve, in the dark and light themes, compared as
//! `tests/layout.rs` compares its own. After an intended change, `UPDATE_SNAPSHOTS=1 cargo test -p bodo-ui --test
//! audio_clips` rewrites them.

#![allow(clippy::unwrap_used, clippy::panic, reason = "tests")]

mod common;

use bodo_kit_core::Tick;
use bodo_model::automation::{ChannelParam, ParamTarget};
use bodo_model::playlist::{ClipOffset, ClipSource, Fades};
use bodo_model::{ChannelId, ChannelKind, Clip};
use bodo_ui::BodoApp;
use common::{click, double_click_at, drag, element, line};
use egui::{Event, Modifiers, PointerButton, Pos2, Rect, Theme, pos2};
use egui_kittest::Harness;
use serde_json::json;

/// Points per bar in these tests.
const BAR: f32 = 48.0;
/// The height of a track.
const TRACK: f32 = 48.0;
/// The crash cymbal of the factory drums.
const CRASH: &str = "factory:Packs/Bodo Drums/Crash.wav";

/// The app zoomed to `BAR` points a bar, after `setup` ran on it.
fn harness(setup: impl FnOnce(&mut BodoApp)) -> (Harness<'static, BodoApp>, Rect) {
    let mut app = common::app(Theme::Dark);
    app.view.playlist.zoom = BAR / 3840.0;
    setup(&mut app);
    let mut harness = common::harness(app, Theme::Dark, false);
    harness.run_steps(4);
    settle(&mut harness);
    let canvas = element(&harness, "Playlist canvas");
    (harness, canvas)
}

/// Steps until every waveform has been read.
fn settle(harness: &mut Harness<'_, BodoApp>) {
    for _ in 0..1000 {
        if harness.state_mut().view.playlist.waveforms_loading() == 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
        harness.step();
    }
    harness.run_steps(2);
}

/// The point at `bar` (counted from 1, fractions allowed) on track `track` (from 1), in the middle of the track.
fn at(canvas: Rect, bar: f32, track: usize) -> Pos2 {
    pos2(canvas.left() + (bar - 1.0) * BAR, canvas.top() + (track as f32 - 0.5) * TRACK)
}

fn clips(harness: &Harness<'_, BodoApp>, track: usize) -> Vec<Clip> {
    let project = harness.state().session.project().clone();
    let id = project.track_order[track - 1];
    project.tracks[&id].clips.iter().cloned().collect()
}

fn steps(harness: &Harness<'_, BodoApp>) -> usize {
    harness.state().session.undo_labels().count()
}

fn channel_named(harness: &Harness<'_, BodoApp>, name: &str) -> ChannelId {
    harness.state().session.project().channels.values().find(|c| c.name == name).unwrap_or_else(|| panic!("no channel {name}")).id
}

/// An audio clip of the crash on track 1, four bars from the start.
fn crash_clip(app: &mut BodoApp) {
    app.run("channel.add_audio", json!({ "sample": CRASH, "name": "Crash" })).unwrap();
    app.run("clip.place_audio", json!({ "track": 1, "channel": "Crash", "start": 0, "length": "4 bars" })).unwrap();
}

/// A press and release of the right button at `at`.
fn right_click_at(harness: &mut Harness<'_, BodoApp>, at: Pos2) {
    harness.event(Event::PointerMoved(at));
    harness.step();
    for pressed in [true, false] {
        harness.event(Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        harness.step();
    }
    harness.step();
}

#[test]
fn a_sound_dropped_on_a_track_becomes_an_audio_clip_there() {
    let (mut h, canvas) = harness(|_| {});
    click(&mut h, "Folder Bodo Drums");
    let crash = element(&h, "Sound Crash").center();
    drag(&mut h, PointerButton::Primary, crash, &line(crash, at(canvas, 3.3, 2), 16), Modifiers::NONE);
    let placed = clips(&h, 2);
    assert_eq!(placed.len(), 1, "one clip on track 2");
    let channel = channel_named(&h, "Crash");
    assert_eq!(placed[0].source, ClipSource::Audio(channel));
    assert_eq!(placed[0].start, Tick(2 * 3840 + 960), "the beat the sound was dropped on");
    assert_eq!(h.state().view.playlist.source, Some(channel), "the dropped sound is what the draw tool places next");
    // The draw tool now places the crash, as long as its sample.
    common::click_at(&mut h, at(canvas, 8.2, 3));
    let drawn = clips(&h, 3);
    assert_eq!(drawn.iter().map(|c| (c.source, c.start, c.len)).collect::<Vec<_>>(), [(ClipSource::Audio(channel), Tick(7 * 3840), placed[0].len)]);
    settle(&mut h);
    assert_eq!(h.state_mut().view.playlist.waveforms_loading(), 0);
}

#[test]
fn slicing_an_audio_clip_offsets_its_right_half() {
    let (mut h, canvas) = harness(crash_clip);
    h.state_mut().run("ui.set_tool", json!({ "tool": "slice" })).unwrap();
    drag(
        &mut h,
        PointerButton::Primary,
        at(canvas, 3.0, 1) - egui::vec2(0.0, 20.0),
        &line(at(canvas, 3.0, 1), at(canvas, 3.0, 2), 4),
        Modifiers::NONE,
    );
    let halves = clips(&h, 1);
    assert_eq!(halves.iter().map(|c| (c.start.0, c.len.0)).collect::<Vec<_>>(), [(0, 7680), (7680, 7680)]);
    assert_eq!(halves[1].offset, ClipOffset::Ticks(Tick(7680)), "the right half plays on from where the cut was");
    assert_eq!(halves[0].source, halves[1].source);
}

#[test]
fn dragging_fade_handles_sets_the_fades_one_undo_step_each() {
    let (mut h, canvas) = harness(crash_clip);
    let before = steps(&h);
    let fade_in = element(&h, "Fade in Crash").center();
    drag(&mut h, PointerButton::Primary, fade_in, &line(fade_in, pos2(at(canvas, 2.0, 1).x, fade_in.y), 6), Modifiers::NONE);
    assert_eq!(clips(&h, 1)[0].fades, Some(Fades { fade_in: Tick(3840), fade_out: Tick(0) }));
    assert_eq!(steps(&h), before + 1, "a drag is one undo step");
    let fade_out = element(&h, "Fade out Crash").center();
    drag(&mut h, PointerButton::Primary, fade_out, &line(fade_out, pos2(at(canvas, 4.5, 1).x, fade_out.y), 6), Modifiers::NONE);
    assert_eq!(clips(&h, 1)[0].fades, Some(Fades { fade_in: Tick(3840), fade_out: Tick(1920) }));
    assert_eq!(steps(&h), before + 2);
    assert_eq!(clips(&h, 1)[0].start, Tick(0), "the clip itself stays put");
}

/// An automation clip of the kick's pan on track 1, four bars long, with a third point at bar 3.
fn pan_clip(app: &mut BodoApp) {
    app.run("automation.create_for", json!({ "target": { "channel": "Kick", "param": "pan" }, "start": 0, "length": "4 bars", "track": 1 })).unwrap();
    app.run("automation.add_point", json!({ "channel": "Kick pan", "at": "3:1:0", "value": 0.5 })).unwrap();
}

fn points(harness: &Harness<'_, BodoApp>) -> Vec<(i64, f32)> {
    let id = channel_named(harness, "Kick pan");
    match &harness.state().session.project().channels[&id].kind {
        ChannelKind::Automation(lane) => lane.points.iter().map(|p| (p.at.0, p.value)).collect(),
        kind => panic!("not automation: {kind:?}"),
    }
}

#[test]
fn automation_points_drag_add_and_remove_on_the_playlist() {
    let (mut h, canvas) = harness(pan_clip);
    let before = steps(&h);
    let point = element(&h, "Point 2 Kick pan").center();
    let to = pos2(at(canvas, 2.5, 1).x, canvas.top() + 18.0);
    drag(&mut h, PointerButton::Primary, point, &line(point, to, 8), Modifiers::NONE);
    let moved = points(&h);
    assert_eq!(moved.len(), 3);
    assert_eq!(moved[1].0, 3840 + 1920, "to bar 2.5, on the beat");
    assert!(moved[1].1 > 0.85, "near the top: {}", moved[1].1);
    assert_eq!(steps(&h), before + 1, "a drag is one undo step");
    // A double-click on the clip adds a point there.
    double_click_at(&mut h, pos2(at(canvas, 4.0, 1).x, at(canvas, 4.0, 1).y + 10.0));
    let added = points(&h);
    assert_eq!(added.len(), 4);
    assert_eq!(added[2].0, 3 * 3840);
    assert!(added[2].1 < 0.5, "below the middle: {}", added[2].1);
    // A right-click on a point removes it.
    let third = element(&h, "Point 3 Kick pan").center();
    right_click_at(&mut h, third);
    assert_eq!(points(&h).len(), 3);
    assert!(!clips(&h, 1).is_empty(), "the clip stays");
    // A double-click on an empty track places a clip of the lane, and adds no point to it.
    h.state_mut().view.playlist.source = Some(channel_named(&h, "Kick pan"));
    double_click_at(&mut h, at(canvas, 2.2, 4));
    assert_eq!(clips(&h, 4).iter().map(|c| (c.start.0, c.len.0)).collect::<Vec<_>>(), [(3840, 4 * 3840)]);
    assert_eq!(points(&h).len(), 3);
}

#[test]
fn painting_an_audio_clip_lays_copies_end_to_end() {
    let (mut h, canvas) = harness(crash_clip);
    h.state_mut().view.playlist.source = Some(channel_named(&h, "Crash"));
    h.state_mut().run("ui.set_tool", json!({ "tool": "paint" })).unwrap();
    let before = steps(&h);
    drag(&mut h, PointerButton::Primary, at(canvas, 1.1, 3), &line(at(canvas, 1.1, 3), at(canvas, 7.5, 3), 24), Modifiers::NONE);
    let painted = clips(&h, 3);
    assert!(painted.len() >= 3, "{painted:?}");
    let length = painted[0].len.0;
    for (index, clip) in painted.iter().enumerate() {
        assert_eq!((clip.start.0, clip.len.0), (index as i64 * length, length), "copy {index}");
    }
    assert_eq!(steps(&h), before + 1, "a stroke is one undo step");
}

#[test]
fn a_rack_knob_makes_an_automation_clip_of_itself() {
    let (mut h, _) = harness(|_| {});
    let knob = element(&h, "Kick pan").center();
    right_click_at(&mut h, knob);
    click(&mut h, "Create automation clip");
    let id = channel_named(&h, "Kick pan");
    let kick = channel_named(&h, "Kick");
    let project = h.state().session.project().clone();
    let ChannelKind::Automation(lane) = &project.channels[&id].kind else { panic!("not automation") };
    assert_eq!(lane.target, ParamTarget::Channel(kick, ChannelParam::Pan));
    assert!(project.tracks.values().flat_map(|t| t.clips.iter()).any(|c| c.source == ClipSource::Automation(id)), "a clip of it on the playlist");
    assert_eq!(h.state().view.playlist.source, Some(id), "and the draw tool places more of it");
    h.run_steps(2);
    element(&h, "Kick pan kind");
    element(&h, "Clip source Kick pan");
}

/// The main window with a faded audio clip of the crash and an automation clip with several curve shapes, in
/// `theme`, compared with snapshot `name`.
fn snapshot(name: &str, theme: Theme) {
    let mut app = common::app(theme);
    app.run("channel.add_audio", json!({ "sample": CRASH, "name": "Crash" })).unwrap();
    app.run("clip.place_audio", json!({ "track": 1, "channel": "Crash", "start": 0, "length": "2 bars" })).unwrap();
    app.run("clip.set_fades", json!({ "fadeIn": 60, "fadeOut": "2 beats" })).unwrap();
    app.run("automation.create_for", json!({ "target": { "channel": "Kick", "param": "pan" }, "start": "3:1:0", "length": "4 bars", "track": 2 }))
        .unwrap();
    let points = json!([
        { "at": 0, "value": 0.5 },
        { "at": "1:3:0", "value": 0.9, "curve": { "sCurve": 0.5 } },
        { "at": "3:1:0", "value": 0.2, "curve": { "exp": 0.6 } },
        { "at": "4:1:0", "value": 0.7, "curve": "hold" },
    ]);
    app.run("automation.set_points", json!({ "channel": "Kick pan", "points": points })).unwrap();
    app.view.playlist.source = app.session.project().channels.values().find(|c| c.name == "Crash").map(|c| c.id);
    let mut harness = common::harness(app, theme, true);
    harness.run_steps(4);
    settle(&mut harness);
    harness.run_steps(4);
    harness.snapshot_options(name, &common::snapshot_options(name));
}

#[test]
fn a_waveform_and_a_curve_dark() {
    snapshot("clips_dark", Theme::Dark);
}

#[test]
fn a_waveform_and_a_curve_light() {
    snapshot("clips_light", Theme::Light);
}

#[test]
fn the_clip_menu_sets_an_audio_clips_stretch_and_removes_its_fades() {
    use egui_kittest::kittest::Queryable;
    let (mut h, _) = harness(crash_clip);
    let clip = clips(&h, 1)[0].id;
    h.state_mut().run("clip.set_fades", json!({ "clips": [clip], "fadeIn": 960, "fadeOut": 960 })).unwrap();
    h.run_steps(2);
    // The name strip along the clip's top opens its menu.
    let strip = element(&h, "Clip Crash at 1:1:0");
    let header = pos2(strip.center().x, strip.top() + 4.0);
    right_click_at(&mut h, header);
    h.get_by_label_contains("Stretch").click();
    h.run_steps(3);
    h.get_by_label("Stretch to the tempo").click();
    h.run_steps(3);
    let stretch = match &h.state().session.project().channels[&channel_named(&h, "Crash")].kind {
        ChannelKind::AudioClip { stretch, .. } => *stretch,
        other => panic!("{other:?}"),
    };
    assert_eq!(stretch, bodo_model::channel::StretchMode::Stretch);
    right_click_at(&mut h, header);
    h.get_by_label("Remove fades").click();
    h.run_steps(3);
    assert!(clips(&h, 1)[0].fades.is_none_or(|f| f.fade_in == Tick(0) && f.fade_out == Tick(0)), "{:?}", clips(&h, 1)[0].fades);
}
