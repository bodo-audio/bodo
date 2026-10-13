//! Channels and mixer tracks: new channels play into the master, and assigning gives a channel the track selected in
//! the mixer or a free one, naming a track that still has its default name after the channel.

#![allow(clippy::unwrap_used, clippy::panic, reason = "tests")]

mod common;

use bodo_engine::Session;
use common::{run, session};
use serde_json::json;

fn target(s: &Session, channel: &str) -> usize {
    let project = s.project();
    let c = project.channels.values().find(|c| c.name == channel).unwrap();
    c.target.and_then(|t| bodo_engine::insert_number(project, t)).unwrap_or(0)
}

fn track_name(s: &Session, number: usize) -> String {
    let project = s.project();
    project.mixer.inserts[&project.mixer.order[number - 1]].name.clone()
}

#[test]
fn new_channels_play_into_the_master() {
    let mut s = session();
    for name in ["Kick", "Clap", "HiHat", "Snare"] {
        assert_eq!(target(&s, name), 0, "the starter kit isn't assigned");
    }
    run(&mut s, "channel.add", json!({ "name": "Bass" }));
    assert_eq!(target(&s, "Bass"), 0);
    assert_eq!(s.editor().insert, None, "no mixer track is selected at first");
}

#[test]
fn assigning_takes_a_free_track_and_names_it() {
    let mut s = session();
    let result = run(&mut s, "channel.assign_insert", json!({ "channel": "Kick" }));
    assert_eq!((result["insert"].as_u64(), result["renamed"].as_bool()), (Some(1), Some(true)));
    assert_eq!(target(&s, "Kick"), 1);
    assert_eq!(track_name(&s, 1), "Kick");
    // The next free track skips one a channel plays into and one the user named.
    run(&mut s, "mixer.rename", json!({ "insert": 2, "name": "Drum bus" }));
    run(&mut s, "channel.assign_insert", json!({ "channel": "Clap" }));
    assert_eq!(target(&s, "Clap"), 3);
    assert_eq!(track_name(&s, 3), "Clap");
    assert_eq!(track_name(&s, 2), "Drum bus");
}

#[test]
fn assigning_uses_the_selected_track_and_keeps_a_name_the_user_gave_it() {
    let mut s = session();
    run(&mut s, "mixer.rename", json!({ "insert": 5, "name": "Room" }));
    run(&mut s, "edit.select", json!({ "insert": 5 }));
    let result = run(&mut s, "channel.assign_insert", json!({ "channel": "Snare" }));
    assert_eq!(target(&s, "Snare"), 5);
    assert_eq!(result["renamed"], json!(false));
    assert_eq!(track_name(&s, 5), "Room", "a track the user named keeps its name");
    // With a track selected, `free` still asks for a free one.
    run(&mut s, "channel.assign_insert", json!({ "channel": "HiHat", "free": true }));
    assert_eq!(target(&s, "HiHat"), 1);
}

#[test]
fn every_track_taken_makes_a_new_one_and_unassigning_goes_back_to_the_master() {
    let mut s = session();
    let tracks = s.project().mixer.order.len();
    for n in 1..=tracks {
        run(&mut s, "channel.add", json!({ "name": format!("Synth {n}") }));
        run(&mut s, "channel.assign_insert", json!({ "channel": format!("Synth {n}") }));
    }
    run(&mut s, "channel.assign_insert", json!({ "channel": "Kick" }));
    assert_eq!(s.project().mixer.order.len(), tracks + 1, "a new track after the last");
    assert_eq!(target(&s, "Kick"), tracks + 1);
    assert_eq!(track_name(&s, tracks + 1), "Kick");
    run(&mut s, "channel.unassign_insert", json!({ "channel": "Kick" }));
    assert_eq!(target(&s, "Kick"), 0);
    assert!(s.undo().is_some());
    assert_eq!(target(&s, "Kick"), tracks + 1, "undo puts the assignment back");
}
