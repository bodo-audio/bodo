//! `bodo-cli mcp --connect` over a real control channel: a stand-in for the app answers `mcp.call` on its own session,
//! as the app does, and the agent's changes land in that session's history.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use bodo_cli::link::{Link, Target};
use bodo_engine::Session;
use bodo_mcp::ToolOutput;
use bodo_mcp::connect::{self, Failure, Remote};
use serde_json::{Value, json};

/// A control server and a thread that answers it like the app, until `stop` is set.
fn fake_app() -> (Target, Arc<AtomicBool>, std::thread::JoinHandle<Session>) {
    let server = bodo_control::Server::start(0).unwrap();
    let target = Target::Address { addr: server.addr(), token: server.token().to_owned() };
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = Arc::clone(&stop);
    let app = std::thread::spawn(move || {
        let mut session = common::session();
        while !stopping.load(Ordering::Relaxed) {
            let Some(request) = server.next() else {
                std::thread::sleep(Duration::from_millis(2));
                continue;
            };
            let answer = match request.method.as_str() {
                connect::METHOD => connect::answer(&mut session, &request.params),
                connect::methods::KEY => Ok(json!({ "key": request.params["key"].clone() })),
                other => Err(format!("unknown control method `{other}`")),
            };
            request.reply(answer);
        }
        session
    });
    (target, stop, app)
}

#[test]
fn tools_run_in_the_app_through_the_control_channel() {
    let (target, stop, app) = fake_app();
    let mut link = Link::open(target).unwrap();
    let mut remote = Remote::new(move |method: &str, params: Value| link.request(method, params));
    let call = |remote: &mut Remote<_>, tool: &str, args: Value| -> ToolOutput {
        let output = remote.call(tool, args);
        assert!(!output.is_error, "{tool} failed: {}", output.text);
        output
    };
    call(&mut remote, "execute", json!({ "id": "pattern.set_steps", "params": { "channel": "Kick", "steps": "x... x... x... x..." } }));
    let seen = call(&mut remote, "inspect_project", json!({}));
    assert!(seen.text.contains("Kick         x...x...x...x..."), "{}", seen.text);
    let pressed = call(&mut remote, "ui_key", json!({ "key": "Space" }));
    assert!(pressed.text.starts_with("pressed: {\"key\":\"Space\"}"), "{}", pressed.text);
    let refused = remote.call("ui_text", json!({ "text": "hi" }));
    assert!(refused.is_error && refused.text.contains("unknown control method `ui.text`"), "{}", refused.text);
    stop.store(true, Ordering::Relaxed);
    let session = app.join().unwrap();
    assert!(session.undo_labels().any(|l| l == "Set steps"), "the change is in the app's own history");
}

#[test]
fn a_missing_app_is_explained() {
    // Bind and drop: the port is free and nothing listens there.
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    assert!(Link::open(Target::Port(port)).is_err());
    let error = bodo_cli::link::connected_server(Target::Port(port)).unwrap_err();
    assert!(format!("{error:#}").contains("cannot reach the running Bodo app"), "{error:#}");
    let mut remote = Remote::new(|_: &str, _: Value| Err(Failure::Unreachable(String::from("gone"))));
    assert!(remote.call("inspect_project", json!({})).text.contains("start Bodo"));
}
