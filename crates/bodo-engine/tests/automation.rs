//! What the automation commands do to the project, beyond their examples.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use bodo_engine::{Call, ErrorCode, Session};
use bodo_kit_core::Tick;
use bodo_model::Project;
use bodo_model::automation::{AutomationLane, ChannelParam, CurveShape, InsertParam, ParamTarget};
use bodo_model::channel::ChannelKind;
use bodo_model::playlist::ClipSource;
use common::{run, session};
use proptest::prelude::*;
use serde_json::{Value, json};

/// A session with a beat in two patterns, a route and the HiHat panned.
fn fixture() -> Session {
    let mut session = session();
    run(&mut session, "pattern.set_steps", json!({ "channel": "Kick", "steps": "x... x... x... x..." }));
    run(&mut session, "pattern.new", json!({ "name": "Verse" }));
    run(&mut session, "pattern.set_steps", json!({ "channel": "HiHat", "steps": "x.x. x.x. x.x. x.x." }));
    run(&mut session, "clip.place", json!({ "track": 1, "pattern": "Verse", "start": "1:1:0", "length": "4 bars" }));
    run(&mut session, "mixer.set_volume", json!({ "insert": 2, "db": -6.0 }));
    run(&mut session, "mixer.add_route", json!({ "from": "insert:1", "to": "insert:5", "level": 0.5 }));
    session
}

fn lane_of(project: &Project, name: &str) -> AutomationLane {
    let channel = project.channels.values().find(|c| c.name == name).unwrap_or_else(|| panic!("no channel {name}"));
    match &channel.kind {
        ChannelKind::Automation(lane) => (**lane).clone(),
        kind => panic!("{name} is {kind:?}"),
    }
}

/// The points of the automation channel `name`, as (ticks, value).
fn points(session: &Session, name: &str) -> Vec<(i64, f32)> {
    lane_of(session.project(), name).points.iter().map(|p| (p.at.0, p.value)).collect()
}

fn error(session: &mut Session, id: &str, params: Value) -> ErrorCode {
    match session.execute(id, params.clone()) {
        Ok(_) => panic!("{id} {params} should fail"),
        Err(error) => error.code,
    }
}

#[test]
fn a_new_clip_starts_at_the_current_value_on_a_free_track() {
    let mut session = fixture();
    let result = run(&mut session, "automation.create_for", json!({ "target": { "insert": 2 } }));
    assert_eq!(result["name"], "Insert 2 volume");
    let lane = lane_of(session.project(), "Insert 2 volume");
    let insert = session.project().mixer.order[1];
    assert_eq!(lane.target, ParamTarget::Insert(insert, InsertParam::Volume));
    let at_minus_6 = lane.target.normalized_of(-6.0);
    assert_eq!(points(&session, "Insert 2 volume"), [(0, at_minus_6), (4 * 3840, at_minus_6)]);
    assert!((lane.target.value_of(at_minus_6) + 6.0).abs() < 1e-3);

    // Track 1 holds the Verse clip over those bars, so the clip goes on track 2.
    let track = session.project().track_order[1];
    let clips: Vec<_> = session.project().tracks[&track].clips.iter().map(|c| (c.start.0, c.len.0, c.source)).collect();
    let channel = session.project().channels.values().find(|c| c.name == "Insert 2 volume").unwrap().id;
    assert_eq!(clips, [(0, 4 * 3840, ClipSource::Automation(channel))]);

    // A second clip for the same target gets its own name and goes on the next free track.
    let result = run(&mut session, "automation.create_for", json!({ "target": { "insert": 2 }, "start": "2:1:0", "length": "1 bar" }));
    assert_eq!(result["name"], "Insert 2 volume 2");
    assert_eq!(result["track"], json!(session.project().track_order[2]));
}

#[test]
fn targets_name_every_kind_of_parameter() {
    let mut session = fixture();
    let cases = [
        (json!("tempo"), "Tempo"),
        (json!({ "insert": 1, "param": "pan" }), "Insert 1 pan"),
        (json!({ "insert": "master", "param": "stereoSep" }), "Master stereo separation"),
        (json!({ "channel": "HiHat", "param": "pitch" }), "HiHat pitch"),
        (json!({ "channel": "HiHat", "param": "cc:74" }), "HiHat CC 74"),
        (json!({ "channel": "HiHat", "param": "pitchBend" }), "HiHat pitch bend"),
        (json!({ "route": { "from": "insert:1", "to": "insert:5" } }), "Insert 1 send to Insert 5"),
    ];
    for (target, name) in cases {
        let result = run(&mut session, "automation.create_for", json!({ "target": target }));
        assert_eq!(result["name"], name);
    }
    // A tempo lane starts at the project tempo; a route lane at its send level.
    let bpm = session.project().tempo.bpm_at(Tick::ZERO) as f32;
    assert!((ParamTarget::Tempo.value_of(points(&session, "Tempo")[0].1) - bpm).abs() < 0.01);
    assert_eq!(points(&session, "Insert 1 send to Insert 5")[0].1, 0.25);
    let hihat = session.project().channels.values().find(|c| c.name == "HiHat").unwrap().id;
    assert_eq!(lane_of(session.project(), "HiHat CC 74").target, ParamTarget::Channel(hihat, ChannelParam::Cc(74)));

    for target in [
        json!("swing"),
        json!({ "insert": 1, "param": "pitch" }),
        json!({ "channel": "HiHat", "param": "cc:128" }),
        json!({ "insert": 1, "channel": "HiHat" }),
        json!({}),
    ] {
        assert_eq!(error(&mut session, "automation.create_for", json!({ "target": target })), ErrorCode::InvalidParams, "{target}");
    }
    assert_eq!(
        error(&mut session, "automation.create_for", json!({ "target": { "route": { "from": "insert:2", "to": "insert:5" } } })),
        ErrorCode::NotFound
    );
}

#[test]
fn points_stay_sorted_through_every_edit() {
    let mut session = fixture();
    run(&mut session, "automation.create_for", json!({ "target": { "insert": 1 }, "name": "Ride" }));
    let unsorted = json!([{ "at": "3:1:0", "value": 0.2 }, { "at": 0, "value": 0.8 }, { "at": 960, "value": 0.5, "curve": "hold" }]);
    run(&mut session, "automation.set_points", json!({ "channel": "Ride", "points": unsorted }));
    assert_eq!(points(&session, "Ride"), [(0, 0.8), (960, 0.5), (7680, 0.2)]);
    assert_eq!(lane_of(session.project(), "Ride").points[1].curve, CurveShape::Hold);

    let added = run(&mut session, "automation.add_point", json!({ "channel": "Ride", "at": "1:2:0", "value": 1.0, "curve": { "stairs": 4 } }));
    // A point at the time of another one goes after it: the value jumps there.
    assert_eq!(added["index"], 2);
    assert_eq!(points(&session, "Ride"), [(0, 0.8), (960, 0.5), (960, 1.0), (7680, 0.2)]);
    assert_eq!(lane_of(session.project(), "Ride").points[2].curve, CurveShape::Stairs(4));

    run(&mut session, "automation.set_curve", json!({ "channel": "Ride", "index": 0, "curve": { "sCurve": 3.0 } }));
    assert_eq!(lane_of(session.project(), "Ride").points[0].curve, CurveShape::SCurve(1.0), "tension is clamped");

    run(&mut session, "automation.remove_point", json!({ "channel": "Ride", "at": "3:1:0" }));
    run(&mut session, "automation.remove_point", json!({ "channel": "Ride", "index": 0 }));
    assert_eq!(points(&session, "Ride"), [(960, 0.5), (960, 1.0)]);

    assert_eq!(error(&mut session, "automation.remove_point", json!({ "channel": "Ride", "index": 5 })), ErrorCode::NotFound);
    assert_eq!(error(&mut session, "automation.remove_point", json!({ "channel": "Ride" })), ErrorCode::InvalidParams);
    assert_eq!(error(&mut session, "automation.set_curve", json!({ "channel": "Ride", "index": 2, "curve": "sine" })), ErrorCode::NotFound);
    assert_eq!(error(&mut session, "automation.add_point", json!({ "channel": "Ride", "at": 0, "value": 1.5 })), ErrorCode::InvalidParams);
    assert_eq!(error(&mut session, "automation.clear", json!({ "channel": "Kick" })), ErrorCode::Rejected);

    run(&mut session, "automation.clear", json!({ "channel": "Ride" }));
    assert!(points(&session, "Ride").is_empty());
    session.project().validate().unwrap();
}

#[test]
fn a_dragged_point_is_one_undo_step() {
    let mut session = fixture();
    run(&mut session, "automation.create_for", json!({ "target": { "insert": 1 }, "name": "Ride" }));
    let steps = session.undo_labels().count();
    for value in [0.7, 0.6, 0.5] {
        let call = Call::new("automation.set_points", json!({ "channel": "Ride", "points": [{ "at": 0, "value": value }] })).in_gesture(3);
        session.invoke(&call).unwrap();
    }
    assert_eq!(session.undo_labels().count(), steps + 1);
    session.undo();
    assert_eq!(points(&session, "Ride").len(), 2, "one undo returns to the clip as created");
}

#[test]
fn pattern_lanes_replace_and_remove_by_target() {
    let mut session = fixture();
    let set = |value: f32| json!({ "pattern": "Verse", "target": { "channel": "HiHat", "param": "pan" }, "points": [{ "at": 0, "value": value }] });
    run(&mut session, "automation.set_lane", set(0.2));
    run(&mut session, "automation.set_lane", set(0.9));
    run(&mut session, "automation.set_lane", json!({ "target": { "channel": "Kick" }, "points": [] }));
    let verse = session.project().patterns.values().find(|p| p.name == "Verse").unwrap();
    assert_eq!(verse.automation.len(), 2, "the second set replaced the HiHat lane");
    assert_eq!(verse.automation[0].points[0].value, 0.9);
    session.project().validate().unwrap();

    run(&mut session, "automation.remove_lane", json!({ "pattern": "Verse", "target": { "channel": "HiHat", "param": "pan" } }));
    let verse = session.project().patterns.values().find(|p| p.name == "Verse").unwrap();
    assert_eq!(verse.automation.len(), 1);
    assert_eq!(
        error(&mut session, "automation.remove_lane", json!({ "pattern": "Verse", "target": { "channel": "HiHat", "param": "pan" } })),
        ErrorCode::NotFound
    );
}

/// One random automation edit.
#[derive(Clone, Debug)]
enum Op {
    Create { bar: u32, bars: u32 },
    Add { at: i64, value: f32 },
    Remove(usize),
    Curve(usize),
    Lane { channel: usize, value: f32 },
    RemoveLane(usize),
    RemoveChannel(usize),
    Undo,
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (1..=8u32, 1..=4u32).prop_map(|(bar, bars)| Op::Create { bar, bars }),
        (0..=8000i64, 0.0..=1.0f32).prop_map(|(at, value)| Op::Add { at, value }),
        (0..4usize).prop_map(Op::Remove),
        (0..4usize).prop_map(Op::Curve),
        (0..6usize, 0.0..=1.0f32).prop_map(|(channel, value)| Op::Lane { channel, value }),
        (0..6usize).prop_map(Op::RemoveLane),
        (0..8usize).prop_map(Op::RemoveChannel),
        Just(Op::Undo),
    ]
}

fn apply(session: &mut Session, op: &Op) {
    let ride = "Insert 1 volume";
    let result = match op {
        Op::Create { bar, bars } => {
            session.execute("automation.create_for", json!({ "target": { "insert": 1 }, "start": bar.to_string(), "length": format!("{bars} bars") }))
        }
        Op::Add { at, value } => session.execute("automation.add_point", json!({ "channel": ride, "at": at, "value": value })),
        Op::Remove(index) => session.execute("automation.remove_point", json!({ "channel": ride, "index": index })),
        Op::Curve(index) => session.execute("automation.set_curve", json!({ "channel": ride, "index": index, "curve": { "exp": 0.5 } })),
        Op::Lane { channel, value } => session
            .execute("automation.set_lane", json!({ "target": { "channel": channel + 1, "param": "pan" }, "points": [{ "at": 0, "value": value }] })),
        Op::RemoveLane(channel) => session.execute("automation.remove_lane", json!({ "target": { "channel": channel + 1, "param": "pan" } })),
        Op::RemoveChannel(channel) => session.execute("channel.remove", json!({ "channels": [channel + 1] })),
        Op::Undo => {
            session.undo();
            return;
        }
    };
    if let Err(error) = result {
        assert!(!error.message.contains("would break the project"), "{op:?} broke an invariant: {error}");
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, failure_persistence: None, ..ProptestConfig::default() })]

    #[test]
    fn random_automation_edits_keep_the_project_valid(ops in prop::collection::vec(op(), 1..30)) {
        let mut session = fixture();
        for op in &ops {
            apply(&mut session, op);
            prop_assert!(session.project().validate().is_ok(), "{op:?}: {:?}", session.project().validate());
        }
        while session.undo().is_some() {}
        prop_assert!(session.project().validate().is_ok());
    }
}
