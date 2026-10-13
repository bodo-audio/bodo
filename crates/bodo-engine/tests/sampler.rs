//! The sampler and channel window commands (project.md section 10.2): partial updates, validation, knob drags as
//! one undo step, and the layer and MIDI Out channel kinds.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use bodo_engine::{Call, ErrorCode, Session};
use bodo_model::channel::{ChannelKind, FilterKind, LoopMode, SamplerParams};
use common::{run, session};
use serde_json::json;

fn kick(session: &Session) -> SamplerParams {
    let channel = session.project().channels.values().find(|c| c.name == "Kick").unwrap();
    match &channel.kind {
        ChannelKind::Sampler(params) => (**params).clone(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn nested_settings_change_only_the_fields_given() {
    let mut s = session();
    run(&mut s, "sampler.set", json!({ "channel": "Kick", "filter": { "kind": "lowPass", "cutoff": 900 }, "loopMode": "forward" }));
    run(&mut s, "sampler.set", json!({ "channel": "Kick", "filter": { "resonance": 0.5 } }));
    let params = kick(&s);
    assert_eq!((params.filter.kind, params.filter.cutoff, params.filter.resonance), (FilterKind::LowPass, 900.0, 0.5));
    assert_eq!(params.loop_mode, LoopMode::Forward, "untouched fields keep their value");
    run(&mut s, "sampler.set", json!({ "channel": "Kick", "stretch": { "timeBeats": 4 } }));
    assert_eq!(kick(&s).stretch.time_beats, Some(4.0));
    run(&mut s, "sampler.set", json!({ "channel": "Kick", "stretch": { "timeBeats": 0 } }));
    assert_eq!(kick(&s).stretch.time_beats, None, "0 goes back to the multiplier");
}

#[test]
fn out_of_range_or_crossed_settings_are_refused_and_change_nothing() {
    let mut s = session();
    let before = kick(&s);
    for params in [
        json!({ "channel": "Kick", "start": 0.8, "end": 0.2 }),
        json!({ "channel": "Kick", "filter": { "cutoff": 5 } }),
        json!({ "channel": "Kick", "lowKey": "C5", "highKey": "C3" }),
        json!({ "channel": "Kick", "pitchEnvelope": { "amount": 99 } }),
        json!({ "channel": "Kick", "arp": { "octaves": 9 } }),
        json!({ "channel": "Kick", "effects": { "crossfade": 0.9 } }),
    ] {
        let error = s.execute("sampler.set", params.clone()).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidParams, "{params}: {error}");
    }
    assert_eq!(kick(&s), before);
    run(&mut s, "channel.add_layer", json!({ "name": "Stack", "children": ["Kick"] }));
    let error = s.execute("sampler.set", json!({ "channel": "Stack", "end": 0.5 })).unwrap_err();
    assert_eq!(error.code, ErrorCode::Rejected, "a layer has no sampler");
}

#[test]
fn a_knob_drag_on_the_sampler_is_one_undo_step() {
    let mut s = session();
    let before = s.undo_labels().count();
    for cutoff in [1000, 900, 800, 700] {
        s.invoke(&Call::new("sampler.set", json!({ "channel": "Kick", "filter": { "cutoff": cutoff } })).in_gesture(3)).unwrap();
    }
    assert_eq!(s.undo_labels().count(), before + 1);
    assert_eq!(kick(&s).filter.cutoff, 700.0);
    s.undo();
    assert_eq!(kick(&s).filter.cutoff, SamplerParams::default().filter.cutoff);
}

#[test]
fn layers_play_their_children_and_never_themselves() {
    let mut s = session();
    let result = run(&mut s, "channel.add_layer", json!({ "name": "Big", "children": ["Kick", "Clap", "Kick"] }));
    assert_eq!(result["children"].as_array().unwrap().len(), 2, "each child once");
    run(&mut s, "channel.add_layer", json!({ "name": "Bigger", "children": ["Big", "Snare"] }));
    let error = s.execute("channel.set_layer", json!({ "channel": "Big", "children": ["Bigger"] })).unwrap_err();
    assert_eq!(error.code, ErrorCode::Rejected, "Big would play itself through Bigger");
    let error = s.execute("channel.set_layer", json!({ "channel": "Big", "children": ["Big"] })).unwrap_err();
    assert_eq!(error.code, ErrorCode::Rejected);
    // Removing a child takes it out of the layer.
    run(&mut s, "channel.remove", json!({ "channels": ["Clap"] }));
    let big = s.project().channels.values().find(|c| c.name == "Big").unwrap();
    let ChannelKind::Layer { children } = &big.kind else { panic!() };
    assert_eq!(children.len(), 1);
}

#[test]
fn midi_out_channels_keep_a_port_and_a_channel_from_1_to_16() {
    let mut s = session();
    let result = run(&mut s, "channel.add_midi_out", json!({ "port": "Synth", "midiChannel": 16 }));
    assert_eq!(result["midiChannel"], 16);
    let synth = s.project().channels.values().find(|c| c.name == "Synth").unwrap();
    assert_eq!(synth.kind, ChannelKind::MidiOut { port: String::from("Synth"), channel: 15 });
    run(&mut s, "channel.set_midi_out", json!({ "channel": "Synth", "port": "Other" }));
    let synth = s.project().channels.values().find(|c| c.name == "Synth").unwrap();
    assert_eq!(synth.kind, ChannelKind::MidiOut { port: String::from("Other"), channel: 15 });
    assert_eq!(s.execute("channel.add_midi_out", json!({ "port": "Synth", "midiChannel": 17 })).unwrap_err().code, ErrorCode::InvalidParams);
    assert_eq!(s.execute("channel.set_midi_out", json!({ "channel": "Kick", "midiChannel": 2 })).unwrap_err().code, ErrorCode::Rejected);
}
