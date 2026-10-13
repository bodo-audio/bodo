//! MIDI input bindings, the metronome and count-in through the session's commands.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use bodo_engine::{ErrorCode, Event};
use bodo_model::channel::MidiBinding;
use common::{run, session};
use serde_json::json;

#[test]
fn channels_bind_to_a_port_and_one_midi_channel_and_unbind() {
    let mut session = session();
    run(&mut session, "channel.set_midi_in", json!({ "channel": "Snare", "port": "Pads", "midiChannel": 10 }));
    let snare = |s: &bodo_engine::Session| s.project().channels.values().find(|c| c.name == "Snare").and_then(|c| c.midi_in.clone());
    assert_eq!(snare(&session), Some(MidiBinding { port: String::from("Pads"), channel: Some(9) }), "MIDI channel 10 is stored as 9");
    run(&mut session, "channel.set_midi_in", json!({ "channel": "Snare", "port": "Keys" }));
    assert_eq!(snare(&session), Some(MidiBinding { port: String::from("Keys"), channel: None }));
    run(&mut session, "channel.set_midi_in", json!({ "channel": "Snare", "port": null }));
    assert_eq!(snare(&session), None);
    let out_of_range = session.execute("channel.set_midi_in", json!({ "channel": "Snare", "port": "Keys", "midiChannel": 17 })).unwrap_err();
    assert_eq!(out_of_range.code, ErrorCode::InvalidParams);
    let without_port = session.execute("channel.set_midi_in", json!({ "channel": "Snare", "midiChannel": 1 })).unwrap_err();
    assert_eq!(without_port.code, ErrorCode::InvalidParams);
    session.undo();
    assert!(snare(&session).is_some(), "bindings are undoable");
}

#[test]
fn the_metronome_and_count_in_toggle_and_tell_the_audio_side() {
    let mut session = session();
    session.take_events();
    run(&mut session, "transport.metronome", json!({}));
    run(&mut session, "transport.count_in", json!({ "on": true }));
    assert!(session.transport().metronome && session.transport().count_in);
    run(&mut session, "transport.metronome", json!({}));
    assert!(!session.transport().metronome);
    assert_eq!(session.take_events(), [Event::Metronome(true), Event::CountIn(true), Event::Metronome(false)]);
    assert_eq!(session.undo_labels().count(), 0, "neither is a project edit");
}
