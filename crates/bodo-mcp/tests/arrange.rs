//! An agent arranges through MCP alone (project.md section 10.3): it groups tracks, solos the group, cuts a time
//! selection, sorts channels into a rack group and consolidates a clip to audio.

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

#[tokio::test(flavor = "current_thread")]
async fn an_agent_groups_solos_cuts_and_consolidates_through_mcp_only() {
    let dir = common::temp_dir("arrange");
    let (client, task) = connect(Server::new(common::session()).unwrap()).await;
    // The agent keeps the id of the first clip it places, the Intro.
    let mut intro = None;
    for step in common::beat_calls() {
        let placed = execute(&client, &step.id, step.params).await;
        if step.id == "clip.place" && intro.is_none() {
            intro = placed["clip"].as_str().or_else(|| placed["result"]["clip"].as_str()).map(str::to_owned);
        }
    }
    let intro = intro.unwrap_or_else(|| panic!("clip.place gave no clip id"));
    execute(&client, "project.save", json!({ "path": dir.join("Arranged.bodo").display().to_string() })).await;

    execute(&client, "channel.set_group", json!({ "channels": ["Kick", "Snare"], "group": "Drums" })).await;
    execute(&client, "playlist.group_tracks", json!({ "tracks": [1, 2], "name": "Beat" })).await;
    execute(&client, "playlist.set_group_props", json!({ "group": "Beat", "solo": true })).await;
    execute(&client, "edit.cut", json!({ "start": "7:1:0", "end": "8:1:0" })).await;
    let consolidated = execute(&client, "clip.consolidate", json!({ "clips": [intro], "name": "Intro" })).await;
    let file = consolidated["result"]["file"].as_str().or_else(|| consolidated["file"].as_str()).map(str::to_owned);
    let file = file.unwrap_or_else(|| panic!("no file in {consolidated}"));
    assert!(std::path::Path::new(&file).is_file(), "{file}");

    let summary = client.call_tool(CallToolRequestParams::new("inspect_project")).await.unwrap();
    let text = summary.content.first().and_then(|c| c.as_text()).map(|t| t.text.clone()).unwrap_or_default();
    assert!(text.contains("Intro"), "the consolidated clip's channel is named after its name: {text}");
    client.cancel().await.unwrap();
    task.await.unwrap();

    // What the agent made is in the saved file once it saves again.
    let project = bodo_io::load(&dir.join("Arranged.bodo")).unwrap();
    assert!(project.tracks.values().all(|t| t.group.is_none()), "the earlier save had no groups yet");
    std::fs::remove_dir_all(dir).unwrap();
}
