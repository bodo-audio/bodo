//! The MCP server end to end in one process: an rmcp client talks to the real server over an in-memory pipe.
//!
//! The agent-task test is the M2 exit criterion for agents: an external MCP client builds the same 4-pattern beat,
//! saves it, and reopens it, using nothing but MCP.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::sync::{Arc, Mutex};

use bodo_engine::inspect;
use bodo_mcp::connect::{self, Failure, Remote};
use bodo_mcp::{Server, tools, ui_tools};
use rmcp::model::{CallToolRequestParams, ReadResourceRequestParams};
use rmcp::service::RunningService;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};

/// Starts `server` and a client connected to it.
async fn connect(server: Server) -> (RunningService<RoleClient, ()>, tokio::task::JoinHandle<()>) {
    let (server_io, client_io) = tokio::io::duplex(1 << 20);
    let task = tokio::spawn(async move {
        let running = server.serve(server_io).await.unwrap();
        running.waiting().await.unwrap();
    });
    (().serve(client_io).await.unwrap(), task)
}

/// Calls `tool`, returning its text, structured content and whether it failed.
async fn call(client: &RunningService<RoleClient, ()>, tool: &str, args: Value) -> (String, Value, bool) {
    let arguments = args.as_object().cloned().unwrap_or_default();
    let result = client.call_tool(CallToolRequestParams::new(tool.to_owned()).with_arguments(arguments)).await.unwrap();
    let text = result.content.first().and_then(|c| c.as_text()).map(|t| t.text.clone()).unwrap_or_default();
    (text, result.structured_content.unwrap_or(Value::Null), result.is_error == Some(true))
}

/// Calls `tool`, which must succeed, and returns its text.
async fn ok(client: &RunningService<RoleClient, ()>, tool: &str, args: Value) -> String {
    let (text, _, failed) = call(client, tool, args.clone()).await;
    assert!(!failed, "{tool} {args} failed:\n{text}");
    text
}

/// The project summary without its first two lines (file, dirty state, transport) and the current-pattern marker,
/// which are session state rather than the document.
fn body(text: &str) -> String {
    text.lines().skip(2).collect::<Vec<_>>().join("\n").replace(" (current)", "")
}

#[tokio::test(flavor = "current_thread")]
async fn an_agent_builds_saves_and_reopens_the_beat_through_mcp_only() {
    let dir = common::temp_dir("beat");
    let path = dir.join("First Beat.bodo");

    let (client, task) = connect(Server::new(common::session()).unwrap()).await;
    let info = client.peer_info().unwrap();
    assert_eq!(info.server_info.as_ref().map(|i| i.name.as_str()), Some("bodo"));
    assert!(info.instructions.as_deref().unwrap_or("").contains("pattern.set_steps"));
    let listed = client.list_all_tools().await.unwrap();
    let names: Vec<&str> = listed.iter().map(|t| t.name.as_ref()).collect();
    assert_eq!(names, ["inspect_project", "inspect_pattern", "inspect_mixer", "list_commands", "describe_command", "execute", "batch"]);

    for step in common::beat_calls() {
        ok(&client, "execute", json!({ "id": step.id, "params": step.params })).await;
    }
    let saved = ok(&client, "execute", json!({ "id": "project.save", "params": { "path": path.display().to_string() } })).await;
    assert!(saved.contains("First Beat.bodo"), "{saved}");
    let built = ok(&client, "inspect_project", json!({})).await;
    assert!(built.contains("Kick         x...x...x...x..."), "{built}");

    let resources = client.list_all_resources().await.unwrap();
    let uris: Vec<&str> = resources.iter().map(|r| r.uri.as_str()).collect();
    assert!(uris.contains(&"bodo://pattern/4") && uris.contains(&"bodo://docs/commands"), "{uris:?}");
    let verse = client.read_resource(ReadResourceRequestParams::new("bodo://pattern/2")).await.unwrap();
    assert!(format!("{:?}", verse.contents[0]).contains("Pattern \\\"Verse\\\""), "{:?}", verse.contents[0]);
    let docs = client.read_resource(ReadResourceRequestParams::new("bodo://docs/commands")).await.unwrap();
    assert!(format!("{:?}", docs.contents[0]).contains("pattern.set_steps"));
    assert!(client.read_resource(ReadResourceRequestParams::new("bodo://pattern/9")).await.is_err());
    client.cancel().await.unwrap();
    task.await.unwrap();

    // A fresh server opens the file and sees the same beat.
    let (client, task) = connect(Server::new(common::session()).unwrap()).await;
    ok(&client, "execute", json!({ "id": "project.open", "params": { "path": path.display().to_string() } })).await;
    let reopened = ok(&client, "inspect_project", json!({})).await;
    assert_eq!(body(&reopened), body(&built));
    client.cancel().await.unwrap();
    task.await.unwrap();

    // What the agent saved is exactly what the engine builds from the same calls.
    let engine = common::engine_beat();
    assert_eq!(bodo_io::load(&path).unwrap(), **engine.project());
    assert_eq!(body(&built), body(&inspect::project(&engine).text));
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn the_whole_beat_in_one_batch_is_one_undo_step() {
    let (client, task) = connect(Server::new(common::session()).unwrap()).await;
    let commands: Vec<Value> = common::beat_calls().into_iter().map(|c| json!({ "id": c.id, "params": c.params })).collect();
    let count = commands.len();
    let (text, structured, failed) = call(&client, "batch", json!({ "commands": commands, "label": "First beat" })).await;
    assert!(!failed, "{text}");
    assert_eq!(structured["results"].as_array().map(Vec::len), Some(count));
    assert!(text.ends_with("changed: First beat (revision 1)"), "{text}");
    let built = ok(&client, "inspect_project", json!({})).await;
    assert_eq!(body(&built), body(&inspect::project(&common::engine_beat()).text));
    let undone = ok(&client, "execute", json!({ "id": "edit.undo" })).await;
    assert!(undone.contains("\"canUndo\":false"), "one step undoes the whole beat: {undone}");
    client.cancel().await.unwrap();
    task.await.unwrap();
}

/// A stand-in for the app's control channel: answers `mcp.call` on its own session as the app does, and the
/// interface methods with fixed answers.
fn fake_app(session: Arc<Mutex<bodo_engine::Session>>) -> impl FnMut(&str, Value) -> Result<Value, Failure> + Send + 'static {
    move |method, params| match method {
        connect::METHOD => connect::answer(&mut session.lock().unwrap(), &params).map_err(Failure::Refused),
        connect::methods::INSPECT => Ok(json!({
            "state": { "windows": ["Channel rack", "Playlist"] },
            "elements": [{ "id": "00ff", "label": "Kick step 1", "role": "toggle", "rect": [10.0, 20.0, 18.0, 22.0], "enabled": true, "selected": false }]
        })),
        connect::methods::CLICK => Ok(json!({ "at": params["target"]["at"].clone() })),
        connect::methods::SCREENSHOT => {
            let path = params["path"].as_str().unwrap().to_owned();
            let mut png = vec![0x89, b'P', b'N', b'G', 13, 10, 26, 10, 0, 0, 0, 13, b'I', b'H', b'D', b'R'];
            png.extend(64u32.to_be_bytes());
            png.extend(32u32.to_be_bytes());
            std::fs::write(&path, png).unwrap();
            Ok(json!({ "path": path }))
        }
        other => Err(Failure::Refused(format!("unknown control method `{other}`"))),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn connected_tools_act_on_the_app() {
    let app = Arc::new(Mutex::new(common::session()));
    let mut remote = Remote::new(fake_app(Arc::clone(&app)));
    let mut list = tools();
    list.extend(ui_tools());
    let server = Server::with_backend("test-connected", list, true, move |tool, args| remote.call(tool, args)).unwrap();
    let (client, task) = connect(server).await;
    assert!(client.peer_info().unwrap().instructions.as_deref().unwrap_or("").contains("running Bodo app"));
    let names: Vec<String> = client.list_all_tools().await.unwrap().iter().map(|t| t.name.to_string()).collect();
    assert_eq!(names.len(), 13, "{names:?}");

    ok(&client, "execute", json!({ "id": "pattern.set_steps", "params": { "channel": "Kick", "steps": "x..." } })).await;
    assert!(app.lock().unwrap().undo_labels().any(|l| l == "Set steps"), "the change is in the app's own history");
    let inspected = ok(&client, "ui_inspect", json!({})).await;
    assert!(inspected.contains("toggle \"Kick step 1\" [10, 20, 18, 22] id 00ff"), "{inspected}");
    let clicked = ok(&client, "ui_click", json!({ "target": { "at": [12.0, 30.0] } })).await;
    assert!(clicked.starts_with("clicked: {\"at\":[12.0,30.0]}"), "{clicked}");
    let (shot, _, failed) = call(&client, "ui_screenshot", json!({})).await;
    assert!(!failed && shot == "window screenshot, 64 x 32 px", "{shot}");
    let (refused, _, failed) = call(&client, "ui_key", json!({ "key": "Space" })).await;
    assert!(failed && refused.contains("unknown control method `ui.key`"), "{refused}");
    client.cancel().await.unwrap();
    task.await.unwrap();
}
