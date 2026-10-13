//! The app's side of the control channel (project.md section 12.4): agents and UI tests read the state and the
//! on-screen elements, drive the interface with real input events, take screenshots and run any command. Requests are
//! answered on the UI thread, a few per frame; input goes into egui over the next frames, as a person would click and
//! type. The design follows pixel3d's.
//!
//! Methods: `ui.inspect` (aliases `ui.state` and `ui.elements`: the state and the elements, filtered by `label` and
//! `role`), `ui.click`, `ui.drag`, `ui.key`, `ui.text`, `ui.screenshot`, `ui.command` (any command), `ui.commands`
//! (the list), and whatever the app's [`Extension`] answers, such as `mcp.call` for `bodo-cli mcp --connect`.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::path::PathBuf;

use egui::{Event, Modifiers, PointerButton, Pos2, pos2};
use serde_json::{Value, json};

use crate::BodoApp;
use crate::control::{Element, element_json, elements};
use crate::windows::WindowKind;

/// Most requests answered per frame.
const PER_FRAME: usize = 32;

/// A request from the control channel, answered once.
pub struct ControlRequest {
    /// The method, such as `ui.click`.
    pub method: String,
    /// Its params.
    pub params: Value,
    reply: Box<dyn FnOnce(Result<Value, String>)>,
}

impl ControlRequest {
    /// A request that `reply` answers.
    pub fn new(method: impl Into<String>, params: Value, reply: impl FnOnce(Result<Value, String>) + 'static) -> Self {
        Self { method: method.into(), params, reply: Box::new(reply) }
    }

    /// Sends the answer back.
    pub fn reply(self, result: Result<Value, String>) {
        (self.reply)(result);
    }
}

/// Where control requests come from: the app shell's control server (`bodo-control`), which the interface can't
/// depend on directly since both sit in the top layer.
pub trait ControlSource {
    /// The next waiting request, if any.
    fn next(&self) -> Option<ControlRequest>;
    /// Sets what wakes the UI when a request arrives.
    fn set_waker(&self, wake: Box<dyn Fn() + Send>);
    /// The address it listens on.
    fn addr(&self) -> SocketAddr;
}

/// Answers control methods the interface doesn't know itself, such as `mcp.call`, which the app shell answers with
/// the MCP tools on the session. Returns `None` for a method it doesn't handle either.
pub type Extension = Box<dyn FnMut(&mut bodo_engine::Session, &str, &Value) -> Option<Result<Value, String>>>;

/// The control server, input still to feed, and screenshots being taken.
#[derive(Default)]
pub struct Remote {
    /// Answers methods the interface doesn't know.
    pub extension: Option<Extension>,
    source: Option<Box<dyn ControlSource>>,
    input: VecDeque<Vec<Event>>,
    shots: VecDeque<(ControlRequest, PathBuf)>,
    waiting: Option<ControlRequest>,
    waker_set: bool,
    ctx: Option<egui::Context>,
    next_shot: u64,
}

impl std::fmt::Debug for Remote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Remote").field("addr", &self.addr()).finish_non_exhaustive()
    }
}

impl Remote {
    /// Takes requests from `source` from now on.
    pub fn attach(&mut self, source: Box<dyn ControlSource>) {
        self.source = Some(source);
        self.waker_set = false;
    }

    /// The address the control channel listens on.
    pub fn addr(&self) -> Option<SocketAddr> {
        self.source.as_ref().map(|s| s.addr())
    }

    /// Remembers the egui context, for commands that need it outside a frame.
    pub fn set_ctx(&mut self, ctx: &egui::Context) {
        if self.ctx.is_none() {
            self.ctx = Some(ctx.clone());
        }
    }

    /// The egui context, once a frame has run.
    pub fn ctx(&self) -> Option<egui::Context> {
        self.ctx.clone()
    }

    /// Feeds this frame's queued input into egui. Call from the raw input hook.
    pub fn feed_input(&mut self, raw: &mut egui::RawInput) {
        if let Some(batch) = self.input.pop_front() {
            raw.events.extend(batch);
        }
    }

    /// Queues input events, one batch per frame.
    pub fn queue_input(&mut self, batches: impl IntoIterator<Item = Vec<Event>>) {
        self.input.extend(batches);
    }

    /// Whether queued input is still waiting to be fed.
    pub fn has_input(&self) -> bool {
        !self.input.is_empty()
    }
}

/// Where to put the pointer: an element `id` or `label` (from `ui.elements`), or a position `at: [x, y]` in points.
fn point(params: &Value, key: &str, list: &[Element]) -> Result<Pos2, String> {
    let p = params.get(key).unwrap_or(params);
    if let Some([x, y]) = p.as_array().filter(|a| a.len() == 2).map(|a| [a[0].as_f64(), a[1].as_f64()])
        && let (Some(x), Some(y)) = (x, y)
    {
        return Ok(pos2(x as f32, y as f32));
    }
    if let Some(at) = p.get("at").and_then(Value::as_array)
        && let (Some(x), Some(y)) = (at.first().and_then(Value::as_f64), at.get(1).and_then(Value::as_f64))
    {
        return Ok(pos2(x as f32, y as f32));
    }
    let id = p.get("id").and_then(Value::as_str);
    let label = p.get("label").and_then(Value::as_str);
    let found = list.iter().rev().find(|e| id.is_some_and(|id| format!("{:016x}", e.id.value()) == id) || label.is_some_and(|l| e.label == l));
    match found {
        Some(e) if e.enabled => Ok(e.rect.center()),
        Some(e) => Err(format!("element `{}` is disabled", e.label)),
        None => Err(String::from("no element matches; pass id, label or at: [x, y] (see ui.elements)")),
    }
}

fn modifiers(params: &Value) -> Modifiers {
    let m = params.get("modifiers");
    let on = |k: &str| m.and_then(|m| m.get(k)).and_then(Value::as_bool).unwrap_or(false);
    Modifiers {
        alt: on("alt"),
        ctrl: on("ctrl"),
        shift: on("shift"),
        mac_cmd: on("cmd") && cfg!(target_os = "macos"),
        command: on("cmd") || on("command"),
    }
}

fn button(params: &Value) -> PointerButton {
    match params.get("button").and_then(Value::as_str) {
        Some("secondary" | "right") => PointerButton::Secondary,
        Some("middle") => PointerButton::Middle,
        _ => PointerButton::Primary,
    }
}

/// The input events of a click at `at`, one batch per frame.
pub fn click_events(at: Pos2, button: PointerButton, modifiers: Modifiers, double: bool) -> Vec<Vec<Event>> {
    let press = |pressed| Event::PointerButton { pos: at, button, pressed, modifiers };
    let mut out = vec![vec![Event::PointerMoved(at)], vec![press(true)], vec![press(false)]];
    if double {
        out.extend([vec![press(true)], vec![press(false)]]);
    }
    out
}

/// The input events of a drag from `from` to `to` in `steps` moves, one batch per frame.
pub fn drag_events(from: Pos2, to: Pos2, steps: usize, button: PointerButton, modifiers: Modifiers) -> Vec<Vec<Event>> {
    let mut out = vec![vec![Event::PointerMoved(from)], vec![Event::PointerButton { pos: from, button, pressed: true, modifiers }]];
    for i in 1..=steps {
        out.push(vec![Event::PointerMoved(from + (to - from) * (i as f32 / steps as f32))]);
    }
    out.push(vec![Event::PointerButton { pos: to, button, pressed: false, modifiers }]);
    out
}

/// Answers waiting requests and finished screenshots. Call once per frame.
///
/// Complexity: O(requests this frame times their work).
pub(crate) fn drain(app: &mut BodoApp, ctx: &egui::Context) {
    if !app.remote.waker_set
        && let Some(source) = &app.remote.source
    {
        let ctx = ctx.clone();
        source.set_waker(Box::new(move || ctx.request_repaint()));
        app.remote.waker_set = true;
    }
    if app.remote.has_input() || app.remote.waiting.is_some() || !app.remote.shots.is_empty() {
        ctx.request_repaint();
    }
    if let Some(request) = app.remote.waiting.take() {
        match app.shots.done.take() {
            Some(result) => request.reply(result.map(|p| json!({ "path": p.display().to_string() }))),
            None => app.remote.waiting = Some(request),
        }
    }
    if app.remote.waiting.is_none()
        && !app.shots.busy()
        && let Some((request, path)) = app.remote.shots.pop_front()
    {
        app.shots.request(path);
        app.remote.waiting = Some(request);
    }
    let Some(source) = &app.remote.source else { return };
    let mut taken = Vec::new();
    while taken.len() < PER_FRAME {
        match source.next() {
            Some(request) => taken.push(request),
            None => break,
        }
    }
    if taken.is_empty() {
        return;
    }
    let list = elements(ctx);
    for request in taken {
        if request.method == "ui.screenshot" {
            let path = request.params.get("path").and_then(Value::as_str).map(PathBuf::from).unwrap_or_else(|| {
                app.remote.next_shot += 1;
                std::env::temp_dir().join(format!("bodo-{}-{}.png", std::process::id(), app.remote.next_shot))
            });
            app.remote.shots.push_back((request, path));
            continue;
        }
        let params = request.params.clone();
        let answer = call(app, &request.method, &params, &list);
        request.reply(answer);
    }
}

/// One control request, answered right away.
pub fn call(app: &mut BodoApp, method: &str, params: &Value, list: &[Element]) -> Result<Value, String> {
    if let Some(mut extension) = app.remote.extension.take() {
        let before = app.session.revision();
        let answer = extension(&mut app.session, method, params);
        app.remote.extension = Some(extension);
        if let Some(answer) = answer {
            if app.session.revision() != before {
                let tool = params.get("tool").and_then(Value::as_str).unwrap_or(method);
                let undo = app.session.undo_labels().last().unwrap_or("a change").to_owned();
                app.set_status(format!("Agent ({tool}): {undo}"), false);
            }
            return answer;
        }
    }
    match method {
        "ui.inspect" | "ui.state" | "ui.elements" => {
            let role = params.get("role").and_then(Value::as_str);
            let text = params.get("label").and_then(Value::as_str).map(str::to_lowercase);
            let found: Vec<Value> = list
                .iter()
                .filter(|e| role.is_none_or(|r| e.role == r))
                .filter(|e| text.as_ref().is_none_or(|t| e.label.to_lowercase().contains(t)))
                .map(element_json)
                .collect();
            Ok(json!({ "state": inspect(app), "elements": found }))
        }
        "ui.click" => {
            let at = point(params, "target", list)?;
            let double = params.get("double").and_then(Value::as_bool).unwrap_or(false);
            app.remote.queue_input(click_events(at, button(params), modifiers(params), double));
            Ok(json!({ "at": [at.x, at.y] }))
        }
        "ui.drag" => {
            let (from, to) = (point(params, "from", list)?, point(params, "to", list)?);
            let steps = params.get("steps").and_then(Value::as_u64).unwrap_or(8).clamp(1, 120) as usize;
            app.remote.queue_input(drag_events(from, to, steps, button(params), modifiers(params)));
            Ok(json!({ "from": [from.x, from.y], "to": [to.x, to.y], "frames": steps + 3 }))
        }
        "ui.key" => {
            let name = params.get("key").and_then(Value::as_str).ok_or("key is required, such as \"Space\" or \"A\"")?;
            let key = egui::Key::from_name(name).ok_or_else(|| format!("unknown key `{name}`"))?;
            let m = modifiers(params);
            let event = |pressed| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: m };
            app.remote.queue_input([vec![event(true)], vec![event(false)]]);
            Ok(json!({ "key": name }))
        }
        "ui.text" => {
            let text = params.get("text").and_then(Value::as_str).ok_or("text is required")?;
            app.remote.queue_input([vec![Event::Text(text.to_owned())]]);
            Ok(json!({ "text": text }))
        }
        "ui.command" => {
            let id = params.get("id").and_then(Value::as_str).ok_or("id is required: a command id")?;
            let params = params.get("params").cloned().unwrap_or(Value::Null);
            app.run(id, params)
        }
        "ui.commands" => {
            let mut list: Vec<Value> = app
                .session
                .registry()
                .iter()
                .map(|c| json!({ "id": c.id, "title": c.title, "summary": c.summary, "shortcut": c.shortcut }))
                .collect();
            list.extend(BodoApp::ui_commands().iter().map(|(id, title, summary)| json!({ "id": id, "title": title, "summary": summary })));
            Ok(json!({ "commands": list }))
        }
        other => Err(format!("unknown control method `{other}`")),
    }
}

/// The interface's state for agents: windows, tools, selection, transport, audio and the project summary.
pub fn inspect(app: &BodoApp) -> Value {
    let windows: serde_json::Map<String, Value> = WindowKind::ALL
        .into_iter()
        .map(|kind| {
            let state = app.prefs.layout.state(kind);
            (kind.id().to_owned(), json!({ "open": state.open, "maximized": state.maximized, "minimized": state.minimized }))
        })
        .collect();
    let summary = bodo_engine::inspect::project(&app.session);
    json!({
        "title": app.title(),
        "file": app.session.file().map(|p| p.display().to_string()),
        "dirty": app.session.is_dirty(),
        "windows": windows,
        "browser": app.prefs.layout.browser,
        "snap": app.view.snap.label(),
        "playlistTool": app.view.playlist.tool.label(),
        "editor": app.session.editor(),
        "transport": app.session.transport(),
        "audio": app.audio.status(),
        "status": app.status().map(|(text, _)| text),
        "palette": app.view.palette.is_some(),
        "history": app.session.undo_labels().collect::<Vec<_>>(),
        "project": summary.text,
    })
}
