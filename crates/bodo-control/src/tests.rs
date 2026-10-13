//! Tests for the control channel: answers reach clients, strangers never reach the app, and discovery files are
//! private and cleaned up.

use super::*;

/// A fresh folder for one test's files.
fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bodo-control-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Answers requests like an app would, until the client thread finishes: echoes the method and params, and refuses
/// the method `fail`.
fn run_app(server: &Server, client: &std::thread::JoinHandle<()>) {
    while !client.is_finished() {
        match server.next() {
            Some(request) => {
                let answer = if request.method == "fail" {
                    Err("refused".to_owned())
                } else {
                    Ok(json!({ "echo": request.method, "params": request.params }))
                };
                request.reply(answer);
            }
            None => std::thread::sleep(Duration::from_millis(2)),
        }
    }
}

#[test]
fn requests_reach_the_app_and_answers_come_back() {
    let server = Server::start(0).unwrap();
    let (addr, token) = (server.addr(), server.token().to_owned());
    let woken = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&woken);
    server.set_waker(move || {
        counter.fetch_add(1, Ordering::Relaxed);
    });
    let client = std::thread::spawn(move || {
        let mut client = Client::connect(addr, &token).unwrap();
        assert_eq!(client.call("ui.inspect", json!({ "a": 1 })).unwrap(), json!({ "echo": "ui.inspect", "params": { "a": 1 } }));
        match client.call("fail", Value::Null) {
            Err(ControlError::Failed { error, status }) => assert_eq!((error.as_str(), status.as_str()), ("refused", status::FAILED)),
            other => panic!("{other:?}"),
        }
    });
    run_app(&server, &client);
    client.join().unwrap();
    assert_eq!(woken.load(Ordering::Relaxed), 2, "every request wakes the app");
}

#[test]
fn hello_names_the_app() {
    let server = Server::start(0).unwrap();
    let mut stream = TcpStream::connect(server.addr()).unwrap();
    writeln!(stream, "{}", json!({ "id": 7, "method": HELLO, "params": { "token": server.token() } })).unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    let reply: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(reply, json!({ "id": 7, "ok": true, "result": { "app": "bodo", "version": env!("CARGO_PKG_VERSION") } }));
}

/// The lines the server sends back on a raw connection until it closes.
fn exchange(addr: SocketAddr, text: &str) -> Vec<Value> {
    let mut stream = TcpStream::connect(addr).unwrap();
    stream.write_all(text.as_bytes()).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    BufReader::new(stream).lines().map_while(Result::ok).map(|line| serde_json::from_str(&line).unwrap()).collect()
}

#[test]
fn strangers_and_web_pages_never_reach_the_app() {
    let server = Server::start(0).unwrap();
    let addr = server.addr();
    let call = r#"{"id": 1, "method": "mcp.call", "params": {"tool": "execute"}}"#;
    // No hello: one refusal, then the server hangs up.
    let replies = exchange(addr, &format!("{call}\n{call}\n"));
    assert_eq!(replies.len(), 1, "{replies:?}");
    assert_eq!((replies[0]["ok"].clone(), replies[0]["status"].clone()), (json!(false), json!(status::NOT_RUN)));
    // A wrong token.
    let wrong = format!("{{\"method\": \"hello\", \"params\": {{\"token\": \"{}\"}}}}\n{call}\n", "0".repeat(64));
    assert_eq!(exchange(addr, &wrong).len(), 1);
    // A browser's POST: the request line is not JSON, so its JSON body is never read.
    let post = format!("POST / HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: text/plain\r\n\r\n{call}\n");
    let replies = exchange(addr, &post);
    assert_eq!(replies.len(), 1);
    assert!(replies[0]["error"].as_str().is_some_and(|e| e.starts_with("not JSON")), "{replies:?}");
    // Garbage after a good hello also ends the connection.
    let hello = format!("{{\"method\": \"hello\", \"params\": {{\"token\": \"{}\"}}}}\n", server.token());
    let replies = exchange(addr, &format!("{hello}not json\n{call}\n"));
    assert_eq!(replies.len(), 2, "{replies:?}");
    assert_eq!(replies[0]["result"]["app"], json!("bodo"));
    // Nothing reached the app.
    assert!(server.next().is_none());
}

#[test]
fn tokens_are_random_and_files_private() {
    let (a, b) = (new_token().unwrap(), new_token().unwrap());
    assert_eq!(a.len(), 64);
    assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
    assert_ne!(a, b);
    assert!(same(&a, &a) && !same(&a, &b) && !same(&a, &a[..63]));
    let dir = temp_dir("private");
    let path = dir.join("nested").join("7.token");
    write_private(&path, &a).unwrap();
    write_private(&path, &b).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), b);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_data_folder_can_be_moved() {
    assert_eq!(data_dir_from(Some(OsString::from("/tmp/bodo-data"))), Some(PathBuf::from("/tmp/bodo-data")));
    let default = data_dir_from(None);
    assert_eq!(data_dir_from(Some(OsString::new())), default, "an empty variable means the default");
    let discovery = Discovery::new("/x/control");
    assert_eq!(discovery.token_path(7870), PathBuf::from("/x/control/7870.token"));
    assert_eq!(discovery.app_path(), PathBuf::from("/x/control/app.json"));
}

#[test]
fn the_default_port_falls_back_to_a_free_one() {
    let first = Server::start_default().unwrap();
    let second = Server::start_default().unwrap();
    assert_ne!(first.addr(), second.addr());
    assert!(first.addr().ip().is_loopback() && second.addr().ip().is_loopback());
}

#[test]
fn clients_find_the_published_app_and_the_files_go_away() {
    let dir = temp_dir("discovery");
    let discovery = Discovery::new(&dir);
    match Client::connect_running_with(&discovery) {
        Err(ControlError::NotRunning(message)) => assert!(message.contains("start Bodo"), "{message}"),
        other => panic!("{other:?}"),
    }

    let server = Server::start(0).unwrap();
    let token = server.publish_to(&discovery).unwrap();
    assert_eq!(token, discovery.token_path(server.addr().port()));
    assert_eq!(std::fs::read_to_string(&token).unwrap(), server.token());
    assert_eq!(discovery.running().unwrap(), RunningApp { port: server.addr().port(), pid: std::process::id() });
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for path in [&token, &discovery.app_path()] {
            assert_eq!(std::fs::metadata(path).unwrap().permissions().mode() & 0o777, 0o600, "{}", path.display());
        }
    }
    let client_discovery = discovery.clone();
    let client = std::thread::spawn(move || {
        let mut client = Client::connect_running_with(&client_discovery).unwrap();
        assert_eq!(client.call("ui.state", Value::Null).unwrap(), json!({ "echo": "ui.state", "params": null }));
        let by_port = Client::connect_published_with(&client_discovery, SocketAddr::from(([127, 0, 0, 1], client_discovery.running().unwrap().port)));
        assert!(by_port.is_ok());
    });
    run_app(&server, &client);
    client.join().unwrap();

    // A second app publishes over the first: dropping the first keeps the second's app.json.
    let newer = Server::start(0).unwrap();
    newer.publish_to(&discovery).unwrap();
    drop(server);
    assert!(!token.exists(), "the token goes with its server");
    assert_eq!(discovery.running().unwrap().port, newer.addr().port());
    let newer_token = discovery.token_path(newer.addr().port());
    drop(newer);
    assert!(!newer_token.exists() && !discovery.app_path().exists());

    // A stale app.json from an app that crashed.
    std::fs::write(discovery.app_path(), json!({ "port": 1, "pid": 1 }).to_string()).unwrap();
    match Client::connect_running_with(&discovery) {
        Err(ControlError::NotRunning(message)) => assert!(message.contains("nothing answers on port 1"), "{message}"),
        other => panic!("{other:?}"),
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
