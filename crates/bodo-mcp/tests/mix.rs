//! An agent mixes through MCP alone (project.md sections 9 and 12.2): it builds the beat, puts a delay on a bus,
//! sends the snare to it, ducks the hats from the kick with a sidechain, shapes an EQ, groups two faders, saves the
//! delay as a plugin preset, loads it elsewhere, and exports the mix and the stems.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use bodo_mcp::Server;
use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};

async fn connect(server: Server) -> (RunningService<RoleClient, ()>, tokio::task::JoinHandle<()>) {
    let (server_io, client_io) = tokio::io::duplex(1 << 20);
    let task = tokio::spawn(async move {
        let running = server.serve(server_io).await.unwrap();
        running.waiting().await.unwrap();
    });
    (().serve(client_io).await.unwrap(), task)
}

/// Runs command `id` through the `execute` tool, which must succeed, and returns its structured result.
async fn execute(client: &RunningService<RoleClient, ()>, id: &str, params: Value) -> Value {
    let arguments = json!({ "id": id, "params": params }).as_object().cloned().unwrap_or_default();
    let result = client.call_tool(CallToolRequestParams::new("execute").with_arguments(arguments)).await.unwrap();
    let text = result.content.first().and_then(|c| c.as_text()).map(|t| t.text.clone()).unwrap_or_default();
    assert!(result.is_error != Some(true), "{id} {params} failed:\n{text}");
    result.structured_content.unwrap_or(Value::Null)
}

/// The value at `key` of a result, whether the tool wraps it in `result` or not.
fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.get("result").and_then(|r| r.get(key)).or_else(|| value.get(key)).unwrap_or(&Value::Null)
}

#[tokio::test(flavor = "current_thread")]
async fn an_agent_sets_up_a_mix_and_exports_it_through_mcp_only() {
    let dir = common::temp_dir("mix");
    let (client, task) = connect(Server::new(common::session()).unwrap()).await;
    for step in common::beat_calls() {
        execute(&client, &step.id, step.params).await;
    }
    execute(&client, "project.save", json!({ "path": dir.join("Mixed.bodo").display().to_string() })).await;

    // A delay bus on insert 5, fed by the snare's insert at half level.
    execute(&client, "mixer.rename", json!({ "insert": 5, "name": "Delay bus" })).await;
    execute(&client, "fx.load", json!({ "insert": 5, "effect": "Delay", "params": { "Mix": 100 } })).await;
    execute(&client, "mixer.add_route", json!({ "from": "insert:4", "to": "insert:5", "level": 0.5 })).await;
    // The hats duck under the kick.
    execute(&client, "fx.load", json!({ "insert": 3, "slot": 1, "effect": "Ducker" })).await;
    execute(&client, "fx.set_sidechain", json!({ "insert": 3, "slot": 1, "from": "insert:1" })).await;
    // A brighter top on the master, and the kick and snare faders down together.
    execute(&client, "mixer.set_eq", json!({ "insert": "master", "band": "high", "gainDb": 2.0 })).await;
    execute(&client, "mixer.adjust", json!({ "inserts": [1, 4], "volumeDb": -2.0 })).await;
    // The delay saved as a preset and loaded on another insert.
    execute(&client, "preset.save_plugin", json!({ "insert": 5, "slot": 1, "path": dir.join("Bus delay.bodopreset").display().to_string() })).await;
    let loaded = execute(&client, "preset.load_plugin", json!({ "insert": 6, "path": dir.join("Bus delay.bodopreset").display().to_string() })).await;
    assert_eq!(field(&loaded, "slot"), 1);

    let mixer = client.call_tool(CallToolRequestParams::new("inspect_mixer")).await.unwrap();
    let text = mixer.content.first().and_then(|c| c.as_text()).map(|t| t.text.clone()).unwrap_or_default();
    assert!(text.contains("Delay bus") && text.contains("Delay") && text.contains("Ducker"), "{text}");

    let mix = execute(&client, "render.export_mix", json!({ "path": dir.join("Mix.wav").display().to_string(), "end": "3:1:0" })).await;
    assert!(std::path::Path::new(field(&mix, "path").as_str().unwrap_or_default()).is_file(), "{mix}");
    let stems =
        execute(&client, "render.export_stems", json!({ "folder": dir.join("Stems").display().to_string(), "start": "2:1:0", "end": "3:1:0" })).await;
    let files = field(&stems, "stems").as_array().cloned().unwrap_or_default();
    assert!(!files.is_empty(), "{stems}");
    for stem in files {
        assert!(std::path::Path::new(stem["path"].as_str().unwrap_or_default()).is_file(), "{stem}");
    }
    client.cancel().await.unwrap();
    task.await.unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}
