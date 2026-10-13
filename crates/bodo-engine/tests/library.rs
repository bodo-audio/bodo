//! The user library (project.md section 14.1, ADR 0029): channel and mixer presets and scores saved and loaded through
//! commands, templates, demos, tags and default export folders. Results are pinned by snapshots; paths inside the
//! library are `library:` keys, so they don't depend on where the test's library is.

#![allow(clippy::unwrap_used, clippy::panic, reason = "tests")]

mod common;

use std::path::PathBuf;

use bodo_engine::library::UserLibrary;
use bodo_engine::library::user::PresetFolder;
use bodo_engine::{Env, Session};
use bodo_model::channel::ChannelKind;
use common::{factory, run, temp_dir};
use serde_json::json;

/// A deterministic session with the factory content and a fresh user library.
fn session(name: &str) -> (Session, UserLibrary) {
    let root = temp_dir(name);
    let library = UserLibrary::new(&root);
    library.ensure().unwrap();
    let session = Session::deterministic(Env { factory: Some(factory()), library: Vec::new(), user: Some(root) });
    (session, library)
}

fn sampler_of<'a>(session: &'a Session, name: &str) -> &'a bodo_model::Channel {
    session.project().channels.values().find(|c| c.name == name).unwrap_or_else(|| panic!("no channel {name}"))
}

fn sample_path(session: &Session, channel: &bodo_model::Channel) -> String {
    let ChannelKind::Sampler(params) = &channel.kind else { panic!("{} is not a sampler", channel.name) };
    session.project().samples[&params.sample.unwrap()].path.clone()
}

#[test]
fn a_channel_preset_carries_its_sample_and_settings_into_another_project() {
    let (mut a, library) = session("channel-preset");
    run(&mut a, "channel.set_pan", json!({ "channel": "Kick", "pan": -0.5 }));
    run(&mut a, "channel.set_volume", json!({ "channel": "Kick", "volume": 0.6 }));
    let saved = run(&mut a, "preset.save_channel", json!({ "channel": "Kick", "name": "Deep Kick" }));
    insta::assert_json_snapshot!("save_channel", saved);
    assert!(library.presets(PresetFolder::ChannelPresets).join("Deep Kick.bodochannel").is_file());

    let (mut b, _) = session("channel-preset-load");
    b.set_env(a.env().clone());
    let steps = b.undo_labels().count();
    let loaded = run(&mut b, "preset.load_channel", json!({ "path": "Deep Kick" }));
    insta::assert_json_snapshot!("load_channel", loaded);
    assert_eq!(b.undo_labels().count(), steps + 1, "one undo step");
    let kick = sampler_of(&b, "Deep Kick");
    assert_eq!((kick.pan, kick.volume), (-0.5, 0.6));
    assert_eq!(sample_path(&b, kick), "factory:Packs/Bodo Drums/Kick.wav");
    assert_eq!(b.editor().channel, Some(kick.id), "the new channel is selected");

    // Into an existing channel: it keeps its place and insert, and takes the preset's sound.
    let clap = sampler_of(&b, "Clap").clone();
    let replaced =
        run(&mut b, "preset.load_channel", json!({ "path": "library:User data/Channel presets/Deep Kick.bodochannel", "channel": "Clap" }));
    assert_eq!(replaced["replaced"], true);
    let now = &b.project().channels[&clap.id];
    assert_eq!((now.target, now.pan), (clap.target, -0.5));
    assert_eq!(sample_path(&b, now), "factory:Packs/Bodo Drums/Kick.wav");
    assert_eq!(b.project().rack_order.iter().position(|c| *c == clap.id), Some(1), "same place in the rack");
}

#[test]
fn layers_save_their_children_and_automation_clips_are_refused() {
    let (mut s, _) = session("layer");
    let layer = s.project().channels.values().find(|c| c.name == "Kick").unwrap().id;
    let snare = s.project().channels.values().find(|c| c.name == "Snare").unwrap().id;
    let hat = s.project().channels.values().find(|c| c.name == "HiHat").unwrap().id;
    // Turn Kick into a layer of Snare and HiHat by hand, as the layer commands will.
    let mut project = (**s.project()).clone();
    let mut channel = (*project.channels[&layer]).clone();
    channel.kind = ChannelKind::Layer { children: imbl::vector![snare, hat] };
    channel.name = String::from("Stack");
    project.channels.insert(layer, std::sync::Arc::new(channel));
    s.recover(project);
    run(&mut s, "preset.save_channel", json!({ "channel": "Stack" }));
    let before = s.project().channels.len();
    run(&mut s, "preset.load_channel", json!({ "path": "Stack" }));
    assert_eq!(s.project().channels.len(), before + 3, "the layer and its two children");
    let ChannelKind::Layer { children } = &sampler_of(&s, "Stack 2").kind else { panic!("not a layer") };
    let names: Vec<&str> = children.iter().map(|c| s.project().channels[c].name.as_str()).collect();
    assert_eq!(names, ["Snare 2", "HiHat 2"]);

    run(&mut s, "automation.create_for", json!({ "target": { "channel": "Snare", "param": "volume" } }));
    let automation = s.project().channels.values().find(|c| matches!(c.kind, ChannelKind::Automation(_))).unwrap().id;
    let error = s.execute("preset.save_channel", json!({ "channel": automation.to_string() })).unwrap_err();
    assert!(error.message.contains("automation clip"), "{error}");
}

#[test]
fn a_mixer_preset_gives_an_insert_its_sound_but_not_its_name_or_routing() {
    let (mut s, _) = session("mixer-preset");
    run(&mut s, "mixer.set_volume", json!({ "insert": 1, "db": -6.0 }));
    run(&mut s, "mixer.set_pan", json!({ "insert": 1, "pan": 0.25 }));
    let saved = run(&mut s, "preset.save_mixer", json!({ "insert": 1, "name": "Quiet Right" }));
    insta::assert_json_snapshot!("save_mixer", saved);
    run(&mut s, "mixer.mute", json!({ "insert": 2 }));
    let before = s.project().mixer.inserts[&s.project().mixer.order[1]].clone();
    let steps = s.undo_labels().count();
    let loaded = run(&mut s, "preset.load_mixer", json!({ "path": "Quiet Right", "insert": 2 }));
    insta::assert_json_snapshot!("load_mixer", loaded);
    assert_eq!(s.undo_labels().count(), steps + 1);
    let after = &s.project().mixer.inserts[&s.project().mixer.order[1]];
    assert_eq!((after.volume_db, after.pan), (-6.0, 0.25));
    assert_eq!((after.id, &after.name, after.muted), (before.id, &before.name, before.muted), "id, name and mute stay");
}

#[test]
fn a_score_lands_where_asked_and_can_replace_what_was_there() {
    let (mut s, _) = session("score");
    run(&mut s, "pattern.set_steps", json!({ "channel": "Kick", "steps": "x... x... x... x..." }));
    let saved = run(&mut s, "preset.save_score", json!({ "channel": "Kick" }));
    insta::assert_json_snapshot!("save_score", saved);
    run(&mut s, "pattern.set_steps", json!({ "channel": "Clap", "steps": "..x. .... .... ...." }));
    let loaded = run(&mut s, "preset.load_score", json!({ "path": "Pattern 1 Kick", "channel": "Clap", "at": 240 }));
    insta::assert_json_snapshot!("load_score", loaded);
    let pattern = s.project().patterns.values().next().unwrap();
    let clap = s.project().channels.values().find(|c| c.name == "Clap").unwrap().id;
    let starts: Vec<i64> = pattern.notes[&clap].notes().iter().map(|n| n.start.0).collect();
    assert_eq!(starts, [240, 480, 1200, 2160, 3120], "the score added a beat apart from 240, beside the clap's own");
    run(&mut s, "preset.load_score", json!({ "path": "Pattern 1 Kick", "channel": "Clap", "replace": true }));
    let pattern = s.project().patterns.values().next().unwrap();
    let starts: Vec<i64> = pattern.notes[&clap].notes().iter().map(|n| n.start.0).collect();
    assert_eq!(starts, [0, 960, 1920, 2880], "replace drops the clap's own notes");
    let error = s.execute("preset.save_score", json!({ "channel": "Snare" })).unwrap_err();
    assert!(error.message.contains("no notes"), "{error}");
}

#[test]
fn presets_need_a_library_or_a_path() {
    let mut s = common::session();
    let error = s.execute("preset.save_channel", json!({ "channel": "Kick" })).unwrap_err();
    assert!(error.message.contains("no user library"), "{error}");
    let dir = temp_dir("preset-path");
    let file = dir.join("Kick.bodochannel");
    let saved = run(&mut s, "preset.save_channel", json!({ "channel": "Kick", "path": file.display().to_string() }));
    assert_eq!(PathBuf::from(saved["path"].as_str().unwrap()), file);
    run(&mut s, "preset.load_channel", json!({ "path": file.display().to_string() }));
    assert!(s.project().channels.values().any(|c| c.name == "Kick 2"));
}

#[test]
fn templates_and_demos_start_new_projects_that_leave_the_original_alone() {
    let (mut s, library) = session("templates");
    run(&mut s, "project.set_info", json!({ "title": "Starter" }));
    run(&mut s, "channel.add", json!({ "sample": "factory:Packs/Bodo Drums/Rim.wav" }));
    let saved = run(&mut s, "project.save_template", json!({}));
    insta::assert_json_snapshot!("save_template", saved);
    assert!(library.templates().join("Starter.bodo").is_file());
    assert!(s.file().is_none(), "the project keeps having no file");

    run(&mut s, "project.new", json!({ "template": "empty", "discardChanges": true }));
    assert!(!s.project().channels.values().any(|c| c.name == "Rim"));
    let template_id = s.project().id;
    run(&mut s, "project.new", json!({ "from": "Starter" }));
    assert!(s.project().channels.values().any(|c| c.name == "Rim"), "the template's channels are there");
    assert_ne!(s.project().id, template_id);
    assert!(s.file().is_none() && !s.is_dirty(), "a new project, nothing to save yet");

    let written = bodo_engine::demo::install(&library.demos(), s.env()).unwrap();
    assert_eq!(written.len(), 1);
    assert!(bodo_engine::demo::install(&library.demos(), s.env()).unwrap().is_empty(), "installed once");
    let demo = written[0].display().to_string();
    run(&mut s, "project.new", json!({ "from": demo }));
    assert_eq!(s.project().meta.title, "Bodo Beat");
    assert_eq!(s.project().patterns.len(), 4);
    assert!(s.project().samples.values().all(|sample| sample.path.starts_with("factory:")), "demos use factory sounds only");
}

#[test]
fn the_library_lists_tags_and_searches_files() {
    let (mut s, library) = session("tags");
    run(&mut s, "preset.save_channel", json!({ "channel": "Snare" }));
    let listed = run(&mut s, "library.list", json!({ "folder": "Channel presets" }));
    insta::assert_json_snapshot!("list", listed);
    let folders = run(&mut s, "library.list", json!({}));
    assert_eq!(folders["folders"].as_array().unwrap().len(), 18);
    let tagged = run(&mut s, "library.tag", json!({ "path": "factory:Packs/Bodo Drums/Kick.wav", "add": ["Punchy", "low end"] }));
    insta::assert_json_snapshot!("tag", tagged);
    assert!(library.tags_file().is_file());
    let found = run(&mut s, "library.search", json!({ "tags": ["punchy"] }));
    assert_eq!(found["count"], 1);
    assert_eq!(found["files"][0]["path"], "factory:Packs/Bodo Drums/Kick.wav");
    let presets = run(&mut s, "library.search", json!({ "query": "snare", "tags": ["channel presets"] }));
    assert_eq!(presets["files"][0]["kind"], "channelPreset");
    run(&mut s, "library.tag", json!({ "path": "factory:Packs/Bodo Drums/Kick.wav", "remove": ["punchy"] }));
    assert_eq!(run(&mut s, "library.search", json!({ "tags": ["punchy"] }))["count"], 0);
}

#[test]
fn exports_of_a_project_with_no_folder_go_to_rendered() {
    let (mut s, library) = session("rendered");
    run(&mut s, "pattern.set_steps", json!({ "channel": "Kick", "steps": "x... x... x... x..." }));
    let written = run(&mut s, "pattern.export_midi", json!({}));
    let path = PathBuf::from(written["path"].as_str().unwrap());
    assert_eq!(path, library.rendered().join("Pattern 1.mid"));
    let again = run(&mut s, "pattern.export_midi", json!({}));
    assert!(again["path"].as_str().unwrap().ends_with("Pattern 1 2.mid"), "a new file each time");
    let named = run(&mut s, "pattern.export_midi", json!({ "path": "Loop" }));
    assert_eq!(PathBuf::from(named["path"].as_str().unwrap()), library.rendered().join("Loop.mid"));
    let saved_at = temp_dir("rendered-project").join("Song.bodo");
    run(&mut s, "project.save_as", json!({ "path": saved_at.display().to_string() }));
    let beside = run(&mut s, "pattern.export_midi", json!({}));
    assert_eq!(PathBuf::from(beside["path"].as_str().unwrap()), bodo_io::project_folder(&saved_at).join("Rendered").join("Pattern 1.mid"));
}
