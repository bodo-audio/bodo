//! Tools that act on the running app (`bodo-cli mcp --connect`), so the user watches the agent work live
//! (project.md sections 12.3 and 12.4).
//!
//! The app answers requests on its control channel. Every project tool travels as [`METHOD`] and the app runs it with
//! [`answer`] on its own session, so the agent's changes land in the app's own undo history. The `ui_*` tools map to
//! the app's interface methods in [`methods`].
//!
//! This crate doesn't open the channel itself: [`Remote`] takes any function that sends one request and returns its
//! result, and the `bodo-cli` app supplies one built on `bodo-control`.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;

use bodo_engine::{ErrorCode, Session};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::args::{NoArgs, UiClickArgs, UiDragArgs, UiInspectArgs, UiKeyArgs, UiTextArgs};
use crate::tools::{self, ToolOutput};

/// The control method that runs a project tool in the app: params `{"tool": name, "args": {...}}`, result
/// [`ToolOutput::to_wire`].
pub const METHOD: &str = "mcp.call";

/// The app's interface methods behind the `ui_*` tools.
pub mod methods {
    /// The interface state and on-screen elements. Params `{"label"?, "role"?}`; result `{"state": {...},
    /// "elements": [{"id", "label", "role", "rect": [x, y, w, h], "enabled", "selected"}]}`.
    pub const INSPECT: &str = "ui.inspect";
    /// A click. Params `{"target": {"label" | "id" | "at": [x, y]}, "button", "modifiers", "double"}`; result
    /// `{"at": [x, y]}`.
    pub const CLICK: &str = "ui.click";
    /// A drag. Params `{"from": target, "to": target, "steps"?, "button", "modifiers"}`; result `{"from", "to"}`.
    pub const DRAG: &str = "ui.drag";
    /// A key press. Params `{"key", "modifiers"}`; result `{"key"}`.
    pub const KEY: &str = "ui.key";
    /// Typed text. Params `{"text"}`; result `{"text"}`.
    pub const TEXT: &str = "ui.text";
    /// A screenshot of the window. Params `{"path"}`, where the app writes a PNG; result `{"path"}`.
    pub const SCREENSHOT: &str = "ui.screenshot";
}

/// The app's side of [`METHOD`]: runs `params.tool` with `params.args` on the app's `session` and answers in the form
/// [`ToolOutput::from_wire`] reads.
///
/// Complexity: the tool's work.
pub fn answer(session: &mut Session, params: &Value) -> Result<Value, String> {
    let tool = params.get("tool").and_then(Value::as_str).ok_or("tool is required")?;
    let args = params.get("args").filter(|a| !a.is_null()).cloned().unwrap_or_else(|| json!({}));
    let output = catch_unwind(AssertUnwindSafe(|| tools::call(session, tool, args)))
        .unwrap_or_else(|_| ToolOutput::failure(ErrorCode::Internal, format!("tool {tool} failed unexpectedly; the project is unchanged"), None));
    Ok(output.to_wire())
}

/// Why a request to the app didn't give an answer.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Failure {
    /// The app isn't running or the connection is gone.
    #[error("Bodo is not reachable: {0}")]
    Unreachable(String),
    /// The app was busy and never took the request, so it is safe to send again.
    #[error("Bodo was busy and did not run it: {0}")]
    NotRun(String),
    /// The app took the request but didn't answer in time; it may have run.
    #[error("Bodo did not answer in time and the call may have run: {0}")]
    MayHaveRun(String),
    /// The app ran the request and refused it.
    #[error("{0}")]
    Refused(String),
    /// The app answered in a form this server doesn't read.
    #[error("Bodo answered in another form ({0}); are the app and bodo-cli the same version?")]
    Protocol(String),
}

impl Failure {
    fn output(&self) -> ToolOutput {
        let hint = match self {
            Self::Unreachable(_) => Some("start Bodo, which lets agents connect by default, then call again"),
            Self::NotRun(_) => Some("it is safe to call again"),
            Self::MayHaveRun(_) => Some("check with inspect_project before calling again"),
            Self::Refused(_) | Self::Protocol(_) => None,
        };
        let code = if matches!(self, Self::Refused(_)) { ErrorCode::Rejected } else { ErrorCode::Io };
        ToolOutput::failure(code, self.to_string(), hint)
    }
}

/// Tools that act on the running app through `request`, which sends one control request and returns its result.
pub struct Remote<R> {
    request: R,
    shots: u64,
}

impl<R> std::fmt::Debug for Remote<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Remote").field("shots", &self.shots).finish_non_exhaustive()
    }
}

fn parse<T: DeserializeOwned>(tool: &str, args: Value) -> Result<T, ToolOutput> {
    let args = if args.is_null() { json!({}) } else { args };
    serde_json::from_value(args)
        .map_err(|e| ToolOutput::failure(ErrorCode::InvalidParams, format!("{tool}: {e}"), Some("see the tool's input schema")))
}

impl<R: FnMut(&str, Value) -> Result<Value, Failure>> Remote<R> {
    /// Tools that send their requests through `request`.
    pub fn new(request: R) -> Self {
        Self { request, shots: 0 }
    }

    /// Runs `tool` in the app. Errors come back as an error output.
    ///
    /// Complexity: one request to the app, plus reading a screenshot file for `ui_screenshot`.
    pub fn call(&mut self, tool: &str, args: Value) -> ToolOutput {
        let result = match tool {
            "ui_inspect" => parse::<UiInspectArgs>(tool, args).and_then(|a| self.inspect(&a)),
            "ui_click" => parse::<UiClickArgs>(tool, args).and_then(|a| self.input(methods::CLICK, json!(a), "clicked")),
            "ui_drag" => parse::<UiDragArgs>(tool, args).and_then(|a| self.input(methods::DRAG, json!(a), "dragged")),
            "ui_key" => parse::<UiKeyArgs>(tool, args).and_then(|a| self.input(methods::KEY, json!(a), "pressed")),
            "ui_text" => parse::<UiTextArgs>(tool, args).and_then(|a| self.input(methods::TEXT, json!(a), "typed")),
            "ui_screenshot" => parse::<NoArgs>(tool, args).and_then(|_| self.screenshot()),
            _ => self
                .request(METHOD, json!({ "tool": tool, "args": args }))
                .and_then(|answer| ToolOutput::from_wire(&answer).ok_or_else(|| Failure::Protocol(String::from("not a tool output")).output())),
        };
        result.unwrap_or_else(|failure| failure)
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, ToolOutput> {
        (self.request)(method, params).map_err(|f| f.output())
    }

    fn inspect(&mut self, args: &UiInspectArgs) -> Result<ToolOutput, ToolOutput> {
        let result = self.request(methods::INSPECT, json!(args))?;
        let mut text = format!("state: {}\n", result.get("state").cloned().unwrap_or(Value::Null));
        let elements = result.get("elements").and_then(Value::as_array).cloned().unwrap_or_default();
        text.push_str(&format!("{} elements:\n", elements.len()));
        for element in &elements {
            let s = |k: &str| element.get(k).and_then(Value::as_str).unwrap_or("");
            let rect: Vec<String> = element
                .get("rect")
                .and_then(Value::as_array)
                .map(|r| r.iter().filter_map(Value::as_f64).map(|v| format!("{v:.0}")).collect())
                .unwrap_or_default();
            let disabled = if element.get("enabled") == Some(&Value::Bool(false)) { " (disabled)" } else { "" };
            let selected = if element.get("selected") == Some(&Value::Bool(true)) { " (selected)" } else { "" };
            text.push_str(&format!("  {} \"{}\" [{}] id {}{disabled}{selected}\n", s("role"), s("label"), rect.join(", "), s("id")));
        }
        Ok(ToolOutput::ok(text, result))
    }

    fn input(&mut self, method: &str, params: Value, verb: &str) -> Result<ToolOutput, ToolOutput> {
        let result = self.request(method, params)?;
        Ok(ToolOutput::ok(format!("{verb}: {result}; the app applies input over the next frames, so inspect again to see the effect"), result))
    }

    fn screenshot(&mut self) -> Result<ToolOutput, ToolOutput> {
        self.shots += 1;
        let path = std::env::temp_dir().join(format!("bodo-mcp-{}-{}.png", std::process::id(), self.shots));
        let result = self.request(methods::SCREENSHOT, json!({ "path": path.display().to_string() }))?;
        let written = result.get("path").and_then(Value::as_str).map_or(path, PathBuf::from);
        let png = std::fs::read(&written)
            .map_err(|e| ToolOutput::failure(ErrorCode::Io, format!("cannot read the screenshot {}: {e}", written.display()), None))?;
        // Best effort: a leftover file in the temp folder is harmless.
        let _ = std::fs::remove_file(&written);
        let text = match png_size(&png) {
            Some((w, h)) => format!("window screenshot, {w} x {h} px"),
            None => String::from("window screenshot"),
        };
        Ok(ToolOutput { text, structured: json!({ "bytes": png.len() }), images: vec![png], is_error: false })
    }
}

/// Width and height from a PNG's header.
fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    let be = |at: usize| png.get(at..at + 4).and_then(|b| b.try_into().ok()).map(u32::from_be_bytes);
    (png.get(1..4) == Some(b"PNG")).then_some(())?;
    Some((be(16)?, be(20)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_header_gives_the_size() {
        let mut png = vec![0x89, b'P', b'N', b'G', 13, 10, 26, 10, 0, 0, 0, 13, b'I', b'H', b'D', b'R'];
        png.extend(640u32.to_be_bytes());
        png.extend(480u32.to_be_bytes());
        assert_eq!(png_size(&png), Some((640, 480)));
        assert_eq!(png_size(b"not a png"), None);
    }

    #[test]
    fn wire_form_round_trips() {
        let output = ToolOutput { text: "t".into(), structured: json!({ "a": 1 }), images: vec![vec![1, 2, 3]], is_error: true };
        assert_eq!(ToolOutput::from_wire(&output.to_wire()), Some(output));
        assert_eq!(ToolOutput::from_wire(&json!({ "nope": 1 })), None);
    }

    #[test]
    fn failures_explain_what_to_do() {
        let mut remote = Remote::new(|_: &str, _: Value| Err(Failure::Unreachable("no app".into())));
        let output = remote.call("inspect_project", json!({}));
        assert!(output.is_error && output.text.contains("start Bodo"), "{}", output.text);
        let mut busy = Remote::new(|_: &str, _: Value| Err(Failure::MayHaveRun("timeout".into())));
        assert!(busy.call("execute", json!({})).text.contains("check with inspect_project"));
        let strict = Remote::new(|_: &str, _: Value| Ok(json!({}))).call("ui_key", json!({ "key": "Space", "x": 1 }));
        assert!(strict.is_error, "arguments stay strict");
    }
}
