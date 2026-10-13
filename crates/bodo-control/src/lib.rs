//! The control channel: JSON lines over loopback, so agents and UI tests can drive the running app (project.md
//! section 12.4).
//!
//! The protocol follows pixel3d's control server. One request per line, `{"id": 1, "method": "ui.inspect",
//! "params": {}}`; one response per line, `{"id": 1, "ok": true, "result": ...}` or `{"id": 1, "ok": false,
//! "error": "...", "status": "..."}`. The [`Server`] listens on 127.0.0.1 only, serves at most [`MAX_CONNECTIONS`]
//! clients at once, and hands requests to the app over a channel; the app answers them from its UI thread
//! ([`Server::next`]). A request the app does not answer within [`TIMEOUT`] fails with the status
//! [`status::NOT_RUN`] if the app never picked it up, or [`status::MAY_HAVE_RUN`] if it did.
//!
//! **Access.** Loopback alone is not a boundary: any local process, and any web page through a browser request, can
//! reach 127.0.0.1. So every connection must first send `{"method": "hello", "params": {"token": ...}}` with the
//! server's random token, and anything else closes it. A line that is not JSON also closes the connection, so an
//! HTTP request (whose first line is `POST / HTTP/1.1`) never gets to its body.
//!
//! **Discovery.** The app listens on [`DEFAULT_PORT`] when it is free, or on a random port ([`Server::start_default`]).
//! [`Server::publish`] writes the token to `<data>/control/<port>.token` and the port and process id to
//! `<data>/control/app.json`, both readable by the current user only, and removes them when the server drops.
//! `bodo-cli mcp --connect` finds the app with [`Client::connect_running`]. `<data>` is Bodo's local data folder
//! ([`data_dir`]); the [`DATA_DIR_ENV`] environment variable moves it, which keeps tests away from the user's files.

#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

/// The method of the first request of every connection; its params carry the token.
pub const HELLO: &str = "hello";

/// The port the app listens on when it is free.
pub const DEFAULT_PORT: u16 = 7870;
/// The most clients served at once.
pub const MAX_CONNECTIONS: usize = 16;
/// How long a request waits for the app's answer.
pub const TIMEOUT: Duration = Duration::from_secs(60);
/// The longest request line accepted, in bytes.
pub const MAX_LINE: usize = 16 * 1024 * 1024;

/// The environment variable that moves Bodo's local data folder, for tests and portable setups.
pub const DATA_DIR_ENV: &str = "BODO_DATA_DIR";

/// The status of a failed request.
pub mod status {
    /// The app never saw the request, so it is safe to send it again.
    pub const NOT_RUN: &str = "not-run";
    /// The app took the request but did not answer in time; it may have run.
    pub const MAY_HAVE_RUN: &str = "may-have-run";
    /// The app ran the request and answered with an error.
    pub const FAILED: &str = "failed";
}

/// Bodo's local data folder: the [`DATA_DIR_ENV`] variable when it is set and not empty, otherwise the platform's
/// local data folder for Bodo (for example `~/Library/Application Support/dev.bodo.Bodo` on macOS).
pub fn data_dir() -> Option<PathBuf> {
    data_dir_from(std::env::var_os(DATA_DIR_ENV))
}

fn data_dir_from(over: Option<OsString>) -> Option<PathBuf> {
    match over.filter(|value| !value.is_empty()) {
        Some(dir) => Some(PathBuf::from(dir)),
        None => directories::ProjectDirs::from("dev", "bodo", "Bodo").map(|dirs| dirs.data_local_dir().to_path_buf()),
    }
}

/// Where the app publishes how to reach it: a folder holding `app.json` and one `<port>.token` per server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Discovery {
    dir: PathBuf,
}

/// What `app.json` says about the running app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunningApp {
    /// The port its control server listens on.
    pub port: u16,
    /// Its process id.
    pub pid: u32,
}

impl Discovery {
    /// The user's discovery folder, `<data>/control` ([`data_dir`]).
    pub fn user() -> Option<Self> {
        data_dir().map(|dir| Self::new(dir.join("control")))
    }

    /// A discovery folder at `dir`.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The folder.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The token file of the server on `port`.
    pub fn token_path(&self, port: u16) -> PathBuf {
        self.dir.join(format!("{port}.token"))
    }

    /// The file that names the running app.
    pub fn app_path(&self) -> PathBuf {
        self.dir.join("app.json")
    }

    /// The running app, from `app.json`. Fails with [`ControlError::NotRunning`] when no app published itself.
    ///
    /// Complexity: one small file read.
    pub fn running(&self) -> Result<RunningApp, ControlError> {
        let path = self.app_path();
        let text = std::fs::read_to_string(&path)
            .map_err(|e| ControlError::NotRunning(format!("Bodo is not running (cannot read {}: {e}); start Bodo and try again", path.display())))?;
        let value: Value = serde_json::from_str(&text).map_err(|e| ControlError::Protocol(format!("{} is not valid JSON: {e}", path.display())))?;
        let field = |key: &str| value.get(key).and_then(Value::as_u64);
        match (field("port").and_then(|p| u16::try_from(p).ok()), field("pid").and_then(|p| u32::try_from(p).ok())) {
            (Some(port), Some(pid)) => Ok(RunningApp { port, pid }),
            _ => Err(ControlError::Protocol(format!("{} has no valid port and pid", path.display()))),
        }
    }
}

/// The token file of the server on `port` in the user's discovery folder.
pub fn token_path(port: u16) -> Option<PathBuf> {
    Discovery::user().map(|d| d.token_path(port))
}

/// The user's `app.json`, which names the running app.
pub fn app_path() -> Option<PathBuf> {
    Discovery::user().map(|d| d.app_path())
}

/// 32 random bytes from the operating system, as hex.
fn new_token() -> Result<String, ControlError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| ControlError::Io(std::io::Error::other(e.to_string())))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Whether two strings are equal, compared without an early exit.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Writes a file only the current user can read: owner read and write on Unix, the per-user data folder's
/// permissions on Windows.
fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // A stale file may carry wider permissions, so start fresh. It may also not exist.
    let _ = std::fs::remove_file(path);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(text.as_bytes())
}

/// Why the server could not start, or a request failed.
#[derive(Debug, thiserror::Error)]
pub enum ControlError {
    /// Reading or writing the socket or a file failed.
    #[error("control channel I/O: {0}")]
    Io(#[from] std::io::Error),
    /// The other side answered something that is not the protocol.
    #[error("bad control message: {0}")]
    Protocol(String),
    /// The app answered with an error.
    #[error("{error} ({status})")]
    Failed {
        /// What went wrong.
        error: String,
        /// One of the [`status`] values.
        status: String,
    },
    /// No app is running, or none answers where it said it would.
    #[error("{0}")]
    NotRunning(String),
}

/// A request waiting for the app's answer.
pub struct Request {
    /// The method, such as `ui.inspect`.
    pub method: String,
    /// Its params; `null` when the client sent none.
    pub params: Value,
    reply: Sender<Result<Value, String>>,
}

impl Request {
    /// Sends the answer back to the client. It is dropped if the client gave up waiting.
    pub fn reply(self, result: Result<Value, String>) {
        // The client may have timed out and left; nothing to do then.
        let _ = self.reply.send(result);
    }
}

impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Request").field("method", &self.method).finish_non_exhaustive()
    }
}

/// A request queued with the flag that tells whether the app took it.
type Queued = (Request, Arc<AtomicBool>);

/// Wakes the app's UI thread when a request arrives.
type Waker = Arc<Mutex<Option<Box<dyn Fn() + Send>>>>;

/// The running server: the address it listens on, its access token and the request queue. Dropping it stops
/// accepting connections and removes what [`Server::publish`] wrote.
pub struct Server {
    addr: SocketAddr,
    token: Arc<str>,
    published: Mutex<Option<Discovery>>,
    requests: Mutex<Receiver<Queued>>,
    stop: Arc<AtomicBool>,
    waker: Waker,
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Server").field("addr", &self.addr).finish_non_exhaustive()
    }
}

impl Server {
    /// Listens on 127.0.0.1:`port` (0 picks a free port) with a new random token. Clients need the token:
    /// [`Server::publish`] it, or hand [`Server::token`] over directly.
    pub fn start(port: u16) -> Result<Self, ControlError> {
        let token: Arc<str> = new_token()?.into();
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        let addr = listener.local_addr()?;
        let (tx, rx) = mpsc::channel::<Queued>();
        let stop = Arc::new(AtomicBool::new(false));
        let waker: Waker = Arc::new(Mutex::new(None));
        let (accept_token, accept_stop, accept_waker) = (Arc::clone(&token), Arc::clone(&stop), Arc::clone(&waker));
        std::thread::Builder::new()
            .name("control server".into())
            .spawn(move || accept(&listener, &tx, &accept_stop, &accept_waker, &accept_token))?;
        tracing::info!(%addr, "control server listening");
        Ok(Self { addr, token, published: Mutex::new(None), requests: Mutex::new(rx), stop, waker })
    }

    /// Listens on [`DEFAULT_PORT`] when it is free, otherwise on a random free port.
    pub fn start_default() -> Result<Self, ControlError> {
        Self::start(DEFAULT_PORT).or_else(|err| {
            tracing::debug!(%err, port = DEFAULT_PORT, "the default control port is taken; using a random one");
            Self::start(0)
        })
    }

    /// The address the server listens on.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The token clients must send in their `hello`.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Publishes the token and this app's port in the user's discovery folder ([`Discovery::user`]), readable by
    /// the current user only. Returns the token file.
    ///
    /// Complexity: a few small file writes.
    pub fn publish(&self) -> Result<PathBuf, ControlError> {
        let discovery = Discovery::user().ok_or_else(|| ControlError::Io(std::io::Error::other("no user data folder to publish the token in")))?;
        self.publish_to(&discovery)
    }

    /// Publishes the token as `<port>.token` and this app as `app.json` in `discovery`, readable by the current
    /// user only. Returns the token file. Both files go away with the server; `app.json` only if it still names it.
    ///
    /// Complexity: a few small file writes.
    pub fn publish_to(&self, discovery: &Discovery) -> Result<PathBuf, ControlError> {
        let token = discovery.token_path(self.addr.port());
        write_private(&token, &self.token)?;
        let app = json!({ "port": self.addr.port(), "pid": std::process::id() });
        write_private(&discovery.app_path(), &app.to_string())?;
        if let Ok(mut published) = self.published.lock() {
            *published = Some(discovery.clone());
        }
        Ok(token)
    }

    /// Sets what wakes the app when a request arrives, since an idle UI draws no frames on its own.
    pub fn set_waker(&self, wake: impl Fn() + Send + 'static) {
        if let Ok(mut waker) = self.waker.lock() {
            *waker = Some(Box::new(wake));
        }
    }

    /// The next waiting request, if any. Call it from the UI thread and answer with [`Request::reply`].
    pub fn next(&self) -> Option<Request> {
        let requests = self.requests.lock().ok()?;
        match requests.try_recv() {
            Ok((request, taken)) => {
                taken.store(true, Ordering::Relaxed);
                Some(request)
            }
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => None,
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Ok(mut published) = self.published.lock()
            && let Some(discovery) = published.take()
        {
            // Best effort: a leftover file names a port nothing answers on, which clients report.
            let _ = std::fs::remove_file(discovery.token_path(self.addr.port()));
            let ours = RunningApp { port: self.addr.port(), pid: std::process::id() };
            if discovery.running().is_ok_and(|app| app == ours) {
                let _ = std::fs::remove_file(discovery.app_path());
            }
        }
        // Wake the accept loop so it sees the flag.
        let _ = TcpStream::connect(self.addr);
    }
}

/// Accepts connections until the server stops, one thread per client.
fn accept(listener: &TcpListener, tx: &Sender<Queued>, stop: &AtomicBool, waker: &Waker, token: &Arc<str>) {
    let open = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming() {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let Ok(mut stream) = stream else { continue };
        if open.load(Ordering::Relaxed) >= MAX_CONNECTIONS {
            let _ = writeln!(stream, "{}", json!({ "id": null, "ok": false, "error": "too many connections", "status": status::NOT_RUN }));
            continue;
        }
        open.fetch_add(1, Ordering::Relaxed);
        let (tx, open, waker, token) = (tx.clone(), Arc::clone(&open), Arc::clone(waker), Arc::clone(token));
        let spawned = std::thread::Builder::new().name("control client".into()).spawn(move || {
            if let Err(err) = serve(stream, &tx, &waker, &token) {
                tracing::debug!(%err, "control client left");
            }
            open.fetch_sub(1, Ordering::Relaxed);
        });
        if spawned.is_err() {
            tracing::warn!("could not start a control client thread");
        }
    }
}

/// Serves one client: a `hello` with the token, then one request per line, answered in order. A wrong or missing
/// token, or a line that is not JSON, ends the connection.
fn serve(stream: TcpStream, tx: &Sender<Queued>, waker: &Waker, token: &str) -> std::io::Result<()> {
    let mut out = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let mut allowed = false;
    loop {
        line.clear();
        let read = (&mut reader).take(MAX_LINE as u64 + 1).read_line(&mut line)?;
        if read == 0 {
            return Ok(());
        }
        if line.len() > MAX_LINE {
            writeln!(out, "{}", json!({ "id": null, "ok": false, "error": "request line too long", "status": status::NOT_RUN }))?;
            return Ok(());
        }
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(err) => {
                writeln!(out, "{}", json!({ "id": null, "ok": false, "error": format!("not JSON: {err}"), "status": status::NOT_RUN }))?;
                return Ok(());
            }
        };
        if !allowed {
            let id = request.get("id").cloned().unwrap_or(Value::Null);
            let given = request.get("params").and_then(|p| p.get("token")).and_then(Value::as_str).unwrap_or("");
            if request.get("method").and_then(Value::as_str) != Some(HELLO) || !same(given, token) {
                let error = "send {\"method\": \"hello\", \"params\": {\"token\": ...}} first, with the token the app published";
                writeln!(out, "{}", json!({ "id": id, "ok": false, "error": error, "status": status::NOT_RUN }))?;
                return Ok(());
            }
            allowed = true;
            writeln!(out, "{}", json!({ "id": id, "ok": true, "result": { "app": "bodo", "version": env!("CARGO_PKG_VERSION") } }))?;
            out.flush()?;
            continue;
        }
        let response = answer(&request, tx, waker);
        writeln!(out, "{response}")?;
        out.flush()?;
    }
}

/// Queues one request for the app and waits for its answer.
fn answer(request: &Value, tx: &Sender<Queued>, waker: &Waker) -> Value {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let Some(method) = request.get("method").and_then(Value::as_str) else {
        return json!({ "id": id, "ok": false, "error": "method is required", "status": status::NOT_RUN });
    };
    let (reply, wait) = mpsc::channel();
    let taken = Arc::new(AtomicBool::new(false));
    let queued = Request { method: method.to_owned(), params: request.get("params").cloned().unwrap_or(Value::Null), reply };
    if tx.send((queued, Arc::clone(&taken))).is_err() {
        return json!({ "id": id, "ok": false, "error": "the app is closing", "status": status::NOT_RUN });
    }
    if let Ok(waker) = waker.lock()
        && let Some(wake) = waker.as_ref()
    {
        wake();
    }
    match wait.recv_timeout(TIMEOUT) {
        Ok(Ok(result)) => json!({ "id": id, "ok": true, "result": result }),
        Ok(Err(error)) => json!({ "id": id, "ok": false, "error": error, "status": status::FAILED }),
        Err(_) => {
            let status = if taken.load(Ordering::Relaxed) { status::MAY_HAVE_RUN } else { status::NOT_RUN };
            json!({ "id": id, "ok": false, "error": "the app did not answer in time", "status": status })
        }
    }
}

/// A client of a running app, such as `bodo-cli mcp --connect` or a UI test.
pub struct Client {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    next_id: u64,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client").field("next_id", &self.next_id).finish_non_exhaustive()
    }
}

impl Client {
    /// Connects to `addr` and introduces itself with `token`.
    pub fn connect(addr: SocketAddr, token: &str) -> Result<Self, ControlError> {
        let stream = TcpStream::connect(addr)?;
        stream.set_read_timeout(Some(TIMEOUT + Duration::from_secs(5)))?;
        let mut client = Self { writer: stream.try_clone()?, reader: BufReader::new(stream), next_id: 1 };
        client.call(HELLO, json!({ "token": token }))?;
        Ok(client)
    }

    /// Connects to `addr` with the token the app published in the user's discovery folder. The app must run on
    /// this machine.
    pub fn connect_published(addr: SocketAddr) -> Result<Self, ControlError> {
        let discovery = Discovery::user().ok_or_else(|| ControlError::Protocol("no user data folder to read the app's token from".into()))?;
        Self::connect_published_with(&discovery, addr)
    }

    /// Connects to `addr` with the token the app published in `discovery`.
    pub fn connect_published_with(discovery: &Discovery, addr: SocketAddr) -> Result<Self, ControlError> {
        let path = discovery.token_path(addr.port());
        let token = std::fs::read_to_string(&path).map_err(|e| {
            ControlError::Io(std::io::Error::new(
                e.kind(),
                format!("no access token at {} ({e}); is Bodo listening on port {}?", path.display(), addr.port()),
            ))
        })?;
        Self::connect(addr, token.trim())
    }

    /// Connects to the Bodo app running on this machine, as named in the user's discovery folder.
    pub fn connect_running() -> Result<Self, ControlError> {
        let discovery = Discovery::user().ok_or_else(|| ControlError::NotRunning("no user data folder to look for Bodo in".into()))?;
        Self::connect_running_with(&discovery)
    }

    /// Connects to the app named in `discovery`. Fails with [`ControlError::NotRunning`] when none published
    /// itself, or when nothing answers on its port.
    pub fn connect_running_with(discovery: &Discovery) -> Result<Self, ControlError> {
        let app = discovery.running()?;
        let addr = SocketAddr::from(([127, 0, 0, 1], app.port));
        // Check that something listens first, so a stale file from a crashed app gets a clear message.
        if let Err(err) = TcpStream::connect(addr) {
            return Err(ControlError::NotRunning(format!(
                "Bodo is not running: nothing answers on port {} ({err}), where process {} said it would listen; start Bodo and try again",
                app.port, app.pid
            )));
        }
        Self::connect_published_with(discovery, addr)
    }

    /// Calls `method` with `params` and returns its result.
    ///
    /// Complexity: one round trip, linear in the size of the request and the answer.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, ControlError> {
        let id = self.next_id;
        self.next_id += 1;
        writeln!(self.writer, "{}", json!({ "id": id, "method": method, "params": params }))?;
        self.writer.flush()?;
        let mut line = String::new();
        if self.reader.read_line(&mut line)? == 0 {
            return Err(ControlError::Protocol("the app closed the connection".into()));
        }
        let response: Value = serde_json::from_str(&line).map_err(|e| ControlError::Protocol(e.to_string()))?;
        if response.get("ok") == Some(&Value::Bool(true)) {
            return Ok(response.get("result").cloned().unwrap_or(Value::Null));
        }
        let text = |key: &str| response.get(key).and_then(Value::as_str).unwrap_or("").to_owned();
        Err(ControlError::Failed { error: text("error"), status: text("status") })
    }
}

#[cfg(test)]
mod tests;
