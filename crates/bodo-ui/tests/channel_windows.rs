//! Channel windows (project.md sections 10.2 and 14.1), driven with real pointer events: the name button opens a
//! channel's window; the Sampler window's markers, checkboxes, lists, knobs and envelope points change the sampler
//! through commands, a drag being one undo step; and the audio clip, automation, layer and MIDI Out windows edit their
//! channels. Snapshots show both tabs of the Sampler window in the dark and light themes.

#![allow(clippy::unwrap_used, clippy::panic, reason = "tests")]

mod common;

use bodo_model::channel::{ChannelKind, FilterKind, LoopMode, SamplerParams};
use bodo_model::{Channel, ChannelId};
use bodo_ui::BodoApp;
use common::{click, drag, element, line};
use egui::{Modifiers, PointerButton, Theme, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

fn channel<'a>(h: &'a Harness<'_, BodoApp>, name: &str) -> &'a Channel {
    h.state().session.project().channels.values().find(|c| c.name == name).unwrap_or_else(|| panic!("no channel {name}"))
}

fn id(h: &Harness<'_, BodoApp>, name: &str) -> ChannelId {
    channel(h, name).id
}

fn sampler(h: &Harness<'_, BodoApp>, name: &str) -> SamplerParams {
    match &channel(h, name).kind {
        ChannelKind::Sampler(params) => (**params).clone(),
        other => panic!("{other:?}"),
    }
}

fn steps(h: &Harness<'_, BodoApp>) -> usize {
    h.state().session.undo_labels().count()
}

/// Steps until every sample preview has rendered.
fn settle(h: &mut Harness<'_, BodoApp>) {
    for _ in 0..2000 {
        if h.state_mut().view.channel_windows.previews_loading() == 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
        h.step();
    }
    h.run_steps(3);
}

/// The app with the Kick's Sampler window open, its preview rendered.
fn kick_window(setup: impl FnOnce(&mut BodoApp)) -> Harness<'static, BodoApp> {
    let mut app = common::app(Theme::Dark);
    setup(&mut app);
    let mut h = common::harness(app, Theme::Dark, false);
    h.run_steps(3);
    click(&mut h, "Kick");
    settle(&mut h);
    h
}

/// A vertical knob drag of `points` upwards.
fn turn(h: &mut Harness<'_, BodoApp>, label: &str, points: f32) {
    let at = element(h, label).center();
    drag(h, PointerButton::Primary, at, &line(at, at - vec2(0.0, points), 6), Modifiers::NONE);
}

#[test]
fn the_name_button_opens_the_channel_window_and_several_stay_open() {
    let mut h = kick_window(|_| {});
    let kick = id(&h, "Kick");
    assert!(h.state().view.channel_windows.is_open(kick));
    element(&h, "Kick waveform");
    click(&mut h, "Clap");
    assert!(h.state().view.channel_windows.is_open(id(&h, "Clap")) && h.state().view.channel_windows.is_open(kick), "both stay open");
    click(&mut h, "Close Sampler - Clap");
    assert!(!h.state().view.channel_windows.is_open(id(&h, "Clap")));
}

#[test]
fn dragging_a_marker_sets_the_played_part_in_one_undo_step() {
    let mut h = kick_window(|_| {});
    let wave = element(&h, "Kick waveform");
    let before = steps(&h);
    let end = element(&h, "Kick end marker").center();
    let to = pos2(wave.left() + wave.width() * 0.6, end.y);
    drag(&mut h, PointerButton::Primary, end, &line(end, to, 8), Modifiers::NONE);
    let params = sampler(&h, "Kick");
    assert!((params.end - 0.6).abs() < 0.02, "{}", params.end);
    assert_eq!(steps(&h), before + 1, "a drag is one undo step");
    // The start offset sits inside the played part.
    let offset = element(&h, "Kick start offset marker").center();
    drag(&mut h, PointerButton::Primary, offset, &line(offset, pos2(wave.left() + wave.width() * 0.3, offset.y), 6), Modifiers::NONE);
    let params = sampler(&h, "Kick");
    assert!((params.start_offset - 0.5).abs() < 0.03, "half way into 0 to 0.6: {}", params.start_offset);
}

#[test]
fn a_sound_dropped_on_the_waveform_loads_into_the_sampler() {
    let mut h = kick_window(|_| {});
    let before = sampler(&h, "Kick").sample;
    click(&mut h, "Folder Bodo Drums");
    let crash = element(&h, "Sound Crash").center();
    let wave = element(&h, "Kick waveform").center();
    drag(&mut h, PointerButton::Primary, crash, &line(crash, wave, 16), Modifiers::NONE);
    let after = sampler(&h, "Kick").sample.unwrap();
    assert_ne!(Some(after), before);
    let path = &h.state().session.project().samples[&after].path;
    assert!(path.ends_with("Crash.wav"), "{path}");
}

#[test]
fn checkboxes_and_lists_set_the_sampler() {
    let mut h = kick_window(|_| {});
    click(&mut h, "Kick Normalize");
    click(&mut h, "Kick Reverse");
    let params = sampler(&h, "Kick");
    assert!(params.effects.normalize && params.reverse);
    click(&mut h, "Kick Loop");
    h.get_by_label("Ping-pong loop").click();
    h.run_steps(3);
    assert_eq!(sampler(&h, "Kick").loop_mode, LoopMode::PingPong);
    // With a loop on, its markers show.
    element(&h, "Kick loop end marker");
}

#[test]
fn the_envelopes_tab_edits_envelopes_the_filter_and_the_arpeggiator() {
    let mut h = kick_window(|_| {});
    click(&mut h, "Kick tab Envelopes");
    h.run_steps(2);
    let before = steps(&h);
    let point = element(&h, "Kick volume envelope attack point").center();
    drag(&mut h, PointerButton::Primary, point, &line(point, point + vec2(60.0, 0.0), 6), Modifiers::NONE);
    assert!(sampler(&h, "Kick").envelope.attack.0 > 0.05, "{:?}", sampler(&h, "Kick").envelope);
    assert_eq!(steps(&h), before + 1);
    click(&mut h, "Kick Filter type");
    h.get_by_label("Low pass").click();
    h.run_steps(3);
    turn(&mut h, "Kick Cutoff", -60.0);
    let filter = sampler(&h, "Kick").filter;
    assert_eq!(filter.kind, FilterKind::LowPass);
    assert!(filter.cutoff < 15_000.0, "turning the cutoff down: {}", filter.cutoff);
    // The arpeggiator sits below the fold: scroll down to it over the window's margin, where no knob takes the wheel.
    let filter = element(&h, "Kick Filter type");
    let body = pos2(filter.left() - 4.0, filter.center().y);
    common::scroll_to(&mut h, "Kick Arpeggiate", body);
    click(&mut h, "Kick Arpeggiate");
    assert!(sampler(&h, "Kick").arp.enabled);
    common::scroll_to(&mut h, "Kick pitch envelope on", body);
    click(&mut h, "Kick pitch envelope on");
    turn(&mut h, "Kick pitch amount", 30.0);
    let pitch = sampler(&h, "Kick").pitch_envelope;
    assert!(pitch.enabled && pitch.amount > 0.0, "{pitch:?}");
}

#[test]
fn the_header_sets_pitch_and_the_insert() {
    let mut h = kick_window(|_| {});
    turn(&mut h, "Kick window pitch", 30.0);
    assert!(channel(&h, "Kick").pitch_cents > 0.0);
    click(&mut h, "Kick window insert");
    h.get_by_label_contains("5 Insert 5").click();
    h.run_steps(3);
    assert_eq!(bodo_engine::insert_number(h.state().session.project(), channel(&h, "Kick").target.unwrap()), Some(5));
}

#[test]
fn a_layer_window_adds_and_removes_children() {
    let mut app = common::app(Theme::Dark);
    app.run("channel.add_layer", json!({ "name": "Stack", "children": ["Kick"] })).unwrap();
    let mut h = common::harness(app, Theme::Dark, false);
    h.run_steps(3);
    click(&mut h, "Stack");
    click(&mut h, "Stack add channel");
    // The rack's name button is called Clap too; the list's item comes after it.
    h.query_all_by_label("Clap").last().unwrap().click();
    h.run_steps(3);
    let children = |h: &Harness<'_, BodoApp>| match &channel(h, "Stack").kind {
        ChannelKind::Layer { children } => children.len(),
        _ => 0,
    };
    assert_eq!(children(&h), 2);
    click(&mut h, "Stack remove Kick");
    assert_eq!(children(&h), 1);
}

#[test]
fn midi_out_audio_clip_and_automation_windows_edit_their_channels() {
    let mut app = common::app(Theme::Dark);
    app.run("channel.add_midi_out", json!({ "port": "Synth", "midiChannel": 1 })).unwrap();
    app.run("channel.add_audio", json!({ "sample": "factory:Packs/Bodo Drums/Crash.wav", "name": "Crash" })).unwrap();
    app.run("automation.create_for", json!({ "target": { "channel": "Kick", "param": "pan" }, "name": "Kick pan" })).unwrap();
    let mut h = common::harness(app, Theme::Dark, false);
    h.run_steps(3);
    // MIDI Out: the MIDI channel, a number dragged sideways.
    click(&mut h, "Synth");
    let number = element(&h, "Synth MIDI channel").center();
    drag(&mut h, PointerButton::Primary, number, &line(number, number + vec2(40.0, 0.0), 6), Modifiers::NONE);
    assert!(matches!(channel(&h, "Synth").kind, ChannelKind::MidiOut { channel, .. } if channel > 0));
    click(&mut h, "Close MIDI Out - Synth");
    // Audio clip: the stretch mode.
    click(&mut h, "Crash");
    click(&mut h, "Crash stretch mode");
    h.get_by_label("Stretch to the tempo").click();
    h.run_steps(3);
    assert!(matches!(channel(&h, "Crash").kind, ChannelKind::AudioClip { stretch: bodo_model::channel::StretchMode::Stretch, .. }));
    click(&mut h, "Close Audio clip - Crash");
    // Automation: clearing the points.
    click(&mut h, "Kick pan");
    element(&h, "Kick pan curve");
    click(&mut h, "Kick pan clear points");
    assert!(matches!(&channel(&h, "Kick pan").kind, ChannelKind::Automation(lane) if lane.points.is_empty()));
}

/// The Kick's Sampler window on `tab` in `theme`, with a loop and a filter, compared with snapshot `name`.
fn snapshot(name: &str, theme: Theme, envelopes: bool) {
    let mut app = common::app(theme);
    app.run(
        "sampler.set",
        json!({
            "channel": "Kick",
            "loopMode": "forward",
            "loopStart": 0.2,
            "loopEnd": 0.7,
            "oneShot": false,
            "end": 0.9,
            "startOffset": 0.1,
            "filter": { "kind": "lowPass", "cutoff": 1200, "resonance": 0.4 },
            "filterEnvelope": { "enabled": true, "decay": 0.5, "amount": 3 },
            "arp": { "enabled": true, "direction": "upDown" }
        }),
    )
    .unwrap();
    let mut h = common::harness(app, theme, true);
    h.run_steps(3);
    click(&mut h, "Kick");
    settle(&mut h);
    if envelopes {
        click(&mut h, "Kick tab Envelopes");
    }
    // New windows fade in; the snapshot waits until they have.
    h.run_steps(30);
    let os_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots").join(std::env::consts::OS);
    let options = egui_kittest::SnapshotOptions::new()
        .threshold(egui_kittest::OsThreshold::new(1.5).windows(2.0))
        .max_failed_pixels(egui_kittest::OsThreshold::new(400).windows(2000));
    let options = if os_dir.join(format!("{name}.png")).exists() { options.output_path(os_dir) } else { options };
    h.snapshot_options(name, &options);
}

#[test]
fn the_sampler_window_dark() {
    snapshot("sampler_dark", Theme::Dark, false);
}

#[test]
fn the_sampler_window_light() {
    snapshot("sampler_light", Theme::Light, false);
}

#[test]
fn the_sampler_envelopes_dark() {
    snapshot("sampler_envelopes_dark", Theme::Dark, true);
}

#[test]
fn the_sampler_envelopes_light() {
    snapshot("sampler_envelopes_light", Theme::Light, true);
}
