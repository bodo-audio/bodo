//! The tools without a transport: a name and JSON arguments in; compact text, the same facts as JSON, and images out.
//!
//! The generic tools reach every command in the registry (project.md section 12.3): `list_commands`,
//! `describe_command`, `execute` and `batch`. The inspect tools read the project as text. [`call`] runs a tool on any
//! session, so a host that owns its session (the desktop app) lends it per call; [`Agent`] owns one.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use bodo_engine::{Call, CommandError, ErrorCode, Session, inspect};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::args::{
    BatchArgs, DescribeCommandArgs, ExecuteArgs, InspectPatternArgs, ListCommandsArgs, NoArgs, UiClickArgs, UiDragArgs, UiInspectArgs, UiKeyArgs,
    UiTextArgs,
};

/// A tool's answer: text for the model, the same data as JSON, and images.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolOutput {
    /// Compact text, the main answer.
    pub text: String,
    /// The same facts as JSON, with ids.
    pub structured: Value,
    /// PNG images, such as a screenshot.
    pub images: Vec<Vec<u8>>,
    /// Whether the tool failed. Nothing changed when it did.
    pub is_error: bool,
}

impl ToolOutput {
    /// A successful answer.
    pub fn ok(text: impl Into<String>, structured: Value) -> Self {
        Self { text: text.into(), structured, images: Vec::new(), is_error: false }
    }

    /// A failed command, in compiler style.
    pub fn error(error: &CommandError) -> Self {
        Self { text: error.to_string(), structured: json!({ "error": error }), images: Vec::new(), is_error: true }
    }

    /// A failure that isn't a command's, such as an unreachable app.
    pub fn failure(code: ErrorCode, message: impl Into<String>, hint: Option<&str>) -> Self {
        let mut error = CommandError::new(code, message);
        error.hint = hint.map(str::to_owned);
        Self::error(&error)
    }

    /// The output as JSON for another process (the app answering `mcp.call`), with images as base64 PNG.
    ///
    /// Complexity: O(text and image bytes).
    pub fn to_wire(&self) -> Value {
        let images: Vec<String> = self.images.iter().map(|png| BASE64.encode(png)).collect();
        json!({ "text": self.text, "structured": self.structured, "images": images, "isError": self.is_error })
    }

    /// Reads [`ToolOutput::to_wire`]; `None` when `value` isn't one.
    ///
    /// Complexity: O(text and image bytes).
    pub fn from_wire(value: &Value) -> Option<Self> {
        let text = value.get("text")?.as_str()?.to_owned();
        let images = match value.get("images") {
            None => Vec::new(),
            Some(list) => list.as_array()?.iter().map(|s| s.as_str().and_then(|s| BASE64.decode(s).ok())).collect::<Option<Vec<_>>>()?,
        };
        let structured = value.get("structured").cloned().unwrap_or(Value::Null);
        Some(Self { text, structured, images, is_error: value.get("isError").and_then(Value::as_bool).unwrap_or(false) })
    }
}

/// A tool: its name, a one-sentence description, its input schema, and whether it only reads.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolInfo {
    /// The name clients call it by.
    pub name: &'static str,
    /// What it does, in one sentence.
    pub description: &'static str,
    /// The JSON Schema of its arguments.
    pub schema: Value,
    /// Whether it never changes the project or the app.
    pub read_only: bool,
}

/// The JSON Schema of `T`, without the `$schema` and `title` keys.
fn schema<T: schemars::JsonSchema>() -> Value {
    let mut schema = schemars::schema_for!(T).to_value();
    if let Some(map) = schema.as_object_mut() {
        map.remove("$schema");
        map.remove("title");
    }
    schema
}

/// The tools every server has, in the order clients list them.
pub fn tools() -> Vec<ToolInfo> {
    vec![
        ToolInfo {
            name: "inspect_project",
            description: "The whole project as compact text: tempo, channels, patterns with their steps as x...x... strings, the playlist and the mixer.",
            schema: schema::<NoArgs>(),
            read_only: true,
        },
        ToolInfo {
            name: "inspect_pattern",
            description: "One pattern in detail: every note with its id, key, position, length and velocity, grouped by channel.",
            schema: schema::<InspectPatternArgs>(),
            read_only: true,
        },
        ToolInfo {
            name: "inspect_mixer",
            description: "The mixer: the master, inserts with their channels and settings, and the routes.",
            schema: schema::<NoArgs>(),
            read_only: true,
        },
        ToolInfo {
            name: "list_commands",
            description: "Every command (or one namespace's) with a one-line summary and its effects. Every change to the project is a command.",
            schema: schema::<ListCommandsArgs>(),
            read_only: true,
        },
        ToolInfo {
            name: "describe_command",
            description: "One command in full: what it does, its params with types and descriptions, its effects and an example.",
            schema: schema::<DescribeCommandArgs>(),
            read_only: true,
        },
        ToolInfo {
            name: "execute",
            description: "Run one command with its params as one undoable step; returns its result, or an error that changed nothing.",
            schema: schema::<ExecuteArgs>(),
            read_only: false,
        },
        ToolInfo {
            name: "batch",
            description: "Run several commands in order as one transaction and one undo step; if any fails, nothing changes.",
            schema: schema::<BatchArgs>(),
            read_only: false,
        },
    ]
}

/// The tools a server has only when it is connected to the running app: they read and drive its interface.
pub fn ui_tools() -> Vec<ToolInfo> {
    vec![
        ToolInfo {
            name: "ui_inspect",
            description: "The app's interface: open windows, current pattern and tool, and the on-screen elements with labels, roles and rectangles.",
            schema: schema::<UiInspectArgs>(),
            read_only: true,
        },
        ToolInfo {
            name: "ui_click",
            description: "Click an element (by label or id from ui_inspect) or a point, as a person would with the mouse.",
            schema: schema::<UiClickArgs>(),
            read_only: false,
        },
        ToolInfo {
            name: "ui_drag",
            description: "Drag from one element or point to another, as a person would with the mouse.",
            schema: schema::<UiDragArgs>(),
            read_only: false,
        },
        ToolInfo { name: "ui_key", description: "Press and release a key, with modifiers.", schema: schema::<UiKeyArgs>(), read_only: false },
        ToolInfo { name: "ui_text", description: "Type text into the focused field.", schema: schema::<UiTextArgs>(), read_only: false },
        ToolInfo {
            name: "ui_screenshot",
            description: "A PNG of the app's window, to see what the user sees. Prefer the inspect tools for facts.",
            schema: schema::<NoArgs>(),
            read_only: true,
        },
    ]
}

/// Reads `args` into `T`; missing or null arguments read as `{}`.
fn parse<T: DeserializeOwned>(tool: &str, args: Value) -> Result<T, CommandError> {
    let args = if args.is_null() { Value::Object(serde_json::Map::new()) } else { args };
    serde_json::from_value(args).map_err(|e| CommandError::invalid(format!("{tool}: {e}")).with_hint("see the tool's input schema"))
}

/// Runs `tool` with `args` on `session`. Errors come back as an error output; nothing changes when a tool fails.
///
/// Complexity: the tool's work: a command's own cost, or O(N log N) in the project's entities for the inspect tools.
pub fn call(session: &mut Session, tool: &str, args: Value) -> ToolOutput {
    let span = tracing::info_span!("tool", %tool);
    let _entered = span.enter();
    let result = match tool {
        "inspect_project" => parse::<NoArgs>(tool, args).map(|_| inspection(inspect::project(session))),
        "inspect_pattern" => parse::<InspectPatternArgs>(tool, args).and_then(|a| inspect::pattern(session, a.pattern.as_ref())).map(inspection),
        "inspect_mixer" => parse::<NoArgs>(tool, args).map(|_| inspection(inspect::mixer(session))),
        "list_commands" => parse::<ListCommandsArgs>(tool, args).and_then(|a| list_commands(session, a.namespace.as_deref())),
        "describe_command" => parse::<DescribeCommandArgs>(tool, args).and_then(|a| describe_command(session, &a.id)),
        "execute" => parse::<ExecuteArgs>(tool, args).and_then(|a| execute(session, a)),
        "batch" => return parse::<BatchArgs>(tool, args).map_or_else(|e| ToolOutput::error(&e), |a| batch(session, a)),
        other => {
            let known: Vec<&str> = tools().iter().map(|t| t.name).collect();
            Err(CommandError::new(ErrorCode::UnknownCommand, format!("no tool `{other}`")).with_hint(format!("the tools are {}", known.join(", "))))
        }
    };
    result.unwrap_or_else(|e| ToolOutput::error(&e))
}

fn inspection(inspection: inspect::Inspection) -> ToolOutput {
    ToolOutput::ok(inspection.text, inspection.data)
}

fn list_commands(session: &Session, namespace: Option<&str>) -> Result<ToolOutput, CommandError> {
    let registry = session.registry();
    let specs: Vec<_> = registry.iter().filter(|c| namespace.is_none_or(|ns| c.namespace() == ns)).collect();
    if specs.is_empty() {
        let mut namespaces: Vec<&str> = registry.iter().map(|c| c.namespace()).collect();
        namespaces.dedup();
        return Err(
            CommandError::not_found("namespace", namespace.unwrap_or_default()).with_candidates(namespaces.into_iter().map(str::to_owned).collect())
        );
    }
    let mut text = format!("{} commands; describe_command shows one's params, execute runs it.\n", specs.len());
    for spec in &specs {
        text.push_str(&format!("{}: {} [{}]\n", spec.id, spec.summary, spec.effects.describe()));
    }
    let list: Vec<Value> = specs.iter().map(|c| json!({ "id": c.id, "title": c.title, "summary": c.summary, "effects": c.effects })).collect();
    Ok(ToolOutput::ok(text, json!({ "commands": list })))
}

fn describe_command(session: &Session, id: &str) -> Result<ToolOutput, CommandError> {
    let registry = session.registry();
    let Some(spec) = registry.get(id.trim()) else {
        let needle = id.trim().to_lowercase();
        let close: Vec<String> =
            registry.iter().filter(|c| c.id.contains(&needle) || needle.contains(c.namespace())).map(|c| c.id.to_owned()).take(10).collect();
        return Err(CommandError::new(ErrorCode::UnknownCommand, format!("no command `{id}`"))
            .with_candidates(close)
            .with_hint("list_commands shows every command"));
    };
    let structured = json!({
        "id": spec.id, "title": spec.title, "summary": spec.summary, "effects": spec.effects,
        "params": spec.params, "example": spec.example, "menu": spec.menu, "shortcut": spec.shortcut,
    });
    Ok(ToolOutput::ok(bodo_engine::docs::command_markdown(spec), structured))
}

/// The line that says what an edit changed.
fn changed_line(label: Option<&str>, fallback: &str, revision: u64) -> String {
    format!("changed: {} (revision {revision})", label.unwrap_or(fallback))
}

fn execute(session: &mut Session, args: ExecuteArgs) -> Result<ToolOutput, CommandError> {
    let output = session.invoke(&Call::new(args.id.trim(), args.params))?;
    let mut text = format!("ok {}: {}", output.id, output.result);
    if output.changed {
        text.push('\n');
        text.push_str(&changed_line(output.undo.as_deref(), &output.id, output.revision));
    }
    let structured = serde_json::to_value(&output).unwrap_or(Value::Null);
    Ok(ToolOutput::ok(text, structured))
}

fn batch(session: &mut Session, args: BatchArgs) -> ToolOutput {
    let calls: Vec<Call> = args.commands.into_iter().map(|c| Call::new(c.id.trim(), c.params)).collect();
    match session.batch(&calls, args.label.as_deref()) {
        Ok(output) => {
            let mut text = format!("ok batch of {}", calls.len());
            for (n, (call, result)) in calls.iter().zip(&output.results).enumerate() {
                text.push_str(&format!("\n{} {}: {result}", n + 1, call.id));
            }
            if output.changed {
                text.push('\n');
                text.push_str(&changed_line(output.undo.as_deref(), "batch", output.revision));
            }
            ToolOutput::ok(text, serde_json::to_value(&output).unwrap_or(Value::Null))
        }
        Err(error) => {
            let which = if error.index < calls.len() {
                format!("command {} of {} ({}) failed", error.index + 1, calls.len(), error.id)
            } else {
                String::from("the batch failed as a whole")
            };
            let text = format!("{which}, so nothing changed:\n{}", error.error);
            ToolOutput { text, structured: json!({ "error": error }), images: Vec::new(), is_error: true }
        }
    }
}

/// The tools on a session of its own.
#[derive(Debug)]
pub struct Agent {
    session: Session,
}

impl Agent {
    /// An agent working on `session`.
    pub fn new(session: Session) -> Self {
        Self { session }
    }

    /// The session.
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// The session, to change it directly.
    pub fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }

    /// Gives the session back.
    pub fn into_session(self) -> Session {
        self.session
    }

    /// Runs `tool` with `args`. Errors come back as an error output, never a panic.
    ///
    /// Complexity: the tool's work.
    pub fn call(&mut self, tool: &str, args: Value) -> ToolOutput {
        call(&mut self.session, tool, args)
    }
}
