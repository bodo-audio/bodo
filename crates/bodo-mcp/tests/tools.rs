//! Every tool's text output on the beat, pinned by snapshots: this is what agents read.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use bodo_mcp::{Agent, ToolOutput};
use serde_json::{Value, json};

fn agent() -> Agent {
    Agent::new(common::engine_beat())
}

fn ok(agent: &mut Agent, tool: &str, args: Value) -> ToolOutput {
    let output = agent.call(tool, args.clone());
    assert!(!output.is_error, "{tool} {args} failed:\n{}", output.text);
    output
}

#[test]
fn inspect_tools_read_the_project_as_text() {
    let mut agent = agent();
    insta::assert_snapshot!("inspect_project", ok(&mut agent, "inspect_project", json!({})).text);
    insta::assert_snapshot!("inspect_pattern", ok(&mut agent, "inspect_pattern", json!({ "pattern": "Verse" })).text);
    insta::assert_snapshot!("inspect_mixer", ok(&mut agent, "inspect_mixer", json!({})).text);
    let current = ok(&mut agent, "inspect_pattern", json!({}));
    assert!(current.text.starts_with("Pattern \"Outro\""), "{}", current.text);
}

#[test]
fn generic_tools_reach_every_command() {
    let mut agent = agent();
    insta::assert_snapshot!("list_commands_pattern", ok(&mut agent, "list_commands", json!({ "namespace": "pattern" })).text);
    let all = ok(&mut agent, "list_commands", json!({}));
    assert_eq!(all.structured["commands"].as_array().map(Vec::len), Some(agent.session().registry().len()));
    let described = ok(&mut agent, "describe_command", json!({ "id": "pattern.set_steps" }));
    insta::assert_snapshot!("describe_command", described.text);
    assert_eq!(described.structured["params"]["additionalProperties"], json!(false));

    let toggled = ok(&mut agent, "execute", json!({ "id": "pattern.toggle_step", "params": { "channel": "Kick", "step": 9 } }));
    insta::assert_snapshot!("execute", toggled.text);
    let selected = ok(&mut agent, "execute", json!({ "id": "ui.select_pattern", "params": { "pattern": "Intro" } }));
    assert!(!selected.text.contains("changed:"), "selection is not a change to the project:\n{}", selected.text);

    let batch = ok(
        &mut agent,
        "batch",
        json!({ "label": "Bridge", "commands": [
            { "id": "pattern.new", "params": { "name": "Bridge" } },
            { "id": "pattern.set_steps", "params": { "channel": "Clap", "steps": "x.x. x.x. x.x. x.x." } }
        ] }),
    );
    insta::assert_snapshot!("batch", batch.text);
    let undone = ok(&mut agent, "execute", json!({ "id": "edit.undo" }));
    assert!(undone.text.contains("\"undone\":\"Bridge\""), "{}", undone.text);
}

#[test]
fn errors_say_what_went_wrong_and_change_nothing() {
    let mut agent = agent();
    let before = agent.session().project().clone();
    let errors = [
        agent.call("execute", json!({ "id": "channel.set_volume", "params": { "channel": "Kik", "volume": 0.5 } })),
        agent.call("execute", json!({ "id": "channel.explode" })),
        agent.call("execute", json!({ "id": "pattern.set_steps", "params": { "channel": "Kick" } })),
        agent.call("batch", json!({ "commands": [{ "id": "pattern.new" }, { "id": "mixer.set_volume", "params": { "insert": 99, "db": 0 } }] })),
        agent.call("describe_command", json!({ "id": "pattern.set_step" })),
        agent.call("list_commands", json!({ "namespace": "synth" })),
        agent.call("inspect_project", json!({ "verbose": true })),
        agent.call("render", json!({})),
    ];
    for output in &errors {
        assert!(output.is_error, "{}", output.text);
    }
    let text: Vec<&str> = errors.iter().map(|o| o.text.as_str()).collect();
    insta::assert_snapshot!("errors", text.join("\n---\n"));
    assert!(std::sync::Arc::ptr_eq(agent.session().project(), &before), "nothing changed");
}
