//! The MCP server: rmcp over any transport, stdio in practice (project.md section 12.3).
//!
//! The session is single-owner, so the tools run on a thread of their own and the async side awaits their answers
//! over a channel. Connected to the running app, that thread forwards the calls to the app instead.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc;
use std::sync::{Arc, OnceLock};

use base64::Engine;
use bodo_engine::{ErrorCode, Registry, Session};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation, ListResourcesResult, ListToolsResult,
    PaginatedRequestParams, ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, Resource, ResourceContents, ServerCapabilities,
    ServerConfig, Tool, ToolAnnotations,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, ServiceExt};
use serde_json::{Map, Value, json};
use tokio::sync::oneshot;

use crate::tools::{Agent, ToolInfo, ToolOutput, tools};

/// What the server tells clients when they connect.
pub const INSTRUCTIONS: &str = "Bodo is a music production app with an FL Studio style workflow: channels in the channel rack play \
patterns, patterns are placed as clips on playlist tracks, and every channel plays into a mixer insert. Every change is a command: \
list_commands and describe_command show them, execute runs one, batch runs several as one undo step, and inspect_project reads \
the project as text. The fastest beat: browser.search finds sounds (the factory kit is under factory:Packs/Bodo Drums), \
channel.add loads one into a new channel, pattern.set_steps sets its steps with a string such as \"x... x... x... x...\" (one \
character per sixteenth note), pattern.new makes the next pattern, clip.place arranges patterns on the playlist, \
transport.set_mode with mode song plays the arrangement, inspect_project checks the result and project.save saves it. Positions \
are \"bar:beat:tick\" counted from 1 (\"3:1:0\" is the start of bar 3) or ticks (960 per beat). Name things by name, by number \
(\"channel:2\", \"insert:3\") or by id. Empty params act on the current selection, such as the current pattern.";

/// Added to [`INSTRUCTIONS`] when connected to the running app.
pub const CONNECTED: &str = " You are connected to the running Bodo app: the user sees every change live and may edit between your \
calls, so inspect again before relying on an older answer. Your changes join the user's undo history. The ui_* tools read and \
drive the interface.";

/// Why the server stopped with an error.
#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    /// The async runtime or the tool thread could not start.
    #[error("runtime: {0}")]
    Runtime(std::io::Error),
    /// The MCP connection failed.
    #[error("mcp: {0}")]
    Mcp(String),
}

type Request = (String, Value, oneshot::Sender<ToolOutput>);

/// The rmcp handler: the tool list and a channel to the thread that answers calls.
pub struct Server {
    tx: mpsc::Sender<Request>,
    tools: Vec<Tool>,
    connected: bool,
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Server").field("tools", &self.tools.len()).field("connected", &self.connected).finish_non_exhaustive()
    }
}

impl Server {
    /// A server whose tools work on `session`, on a thread of their own.
    pub fn new(session: Session) -> Result<Self, ServeError> {
        let mut agent = Agent::new(session);
        Self::with_backend("bodo-mcp-session", tools(), false, move |tool, args| agent.call(tool, args))
    }

    /// A server that lists `list` and answers every call with `answer` on a thread named `thread`. `connected` says
    /// the tools act on the running app, which changes the instructions.
    pub fn with_backend(
        thread: &str,
        list: Vec<ToolInfo>,
        connected: bool,
        mut answer: impl FnMut(&str, Value) -> ToolOutput + Send + 'static,
    ) -> Result<Self, ServeError> {
        let (tx, rx) = mpsc::channel::<Request>();
        std::thread::Builder::new()
            .name(thread.into())
            .spawn(move || {
                while let Ok((tool, args, reply)) = rx.recv() {
                    let output = catch_unwind(AssertUnwindSafe(|| answer(&tool, args))).unwrap_or_else(|_| {
                        ToolOutput::failure(ErrorCode::Internal, format!("tool {tool} failed unexpectedly; the project is unchanged"), None)
                    });
                    // The client may have gone; nothing to do then.
                    let _ = reply.send(output);
                }
            })
            .map_err(ServeError::Runtime)?;
        let tools = list
            .into_iter()
            .map(|t| {
                let schema = match t.schema {
                    Value::Object(map) => map,
                    _ => Map::new(),
                };
                Tool::new(t.name, t.description, Arc::new(schema)).with_annotations(ToolAnnotations::new().read_only(t.read_only).open_world(false))
            })
            .collect();
        Ok(Self { tx, tools, connected })
    }

    /// Asks the tool thread to run a tool.
    async fn ask(&self, tool: &str, args: Value) -> Result<ToolOutput, McpError> {
        let (tx, rx) = oneshot::channel();
        self.tx.send((tool.to_owned(), args, tx)).map_err(|_| McpError::internal_error("the session stopped", None))?;
        rx.await.map_err(|_| McpError::internal_error("the session stopped", None))
    }
}

/// The command reference, generated once.
fn commands_doc() -> &'static str {
    static DOC: OnceLock<String> = OnceLock::new();
    DOC.get_or_init(|| bodo_engine::docs::commands_markdown(&Registry::builtin()))
}

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().enable_resources().build())
            .with_server_info(Implementation::new("bodo", env!("CARGO_PKG_VERSION")))
            .with_instructions(if self.connected { format!("{INSTRUCTIONS}{CONNECTED}") } else { INSTRUCTIONS.to_owned() })
    }

    async fn list_tools(&self, _request: Option<PaginatedRequestParams>, _context: RequestContext<RoleServer>) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(self.tools.clone()))
    }

    async fn call_tool(&self, request: CallToolRequestParams, _context: RequestContext<RoleServer>) -> Result<CallToolResponse, McpError> {
        let args = request.arguments.map_or_else(|| Value::Object(Map::new()), Value::Object);
        let output = self.ask(&request.name, args).await?;
        let mut content = vec![ContentBlock::text(output.text)];
        for png in &output.images {
            content.push(ContentBlock::image(base64::engine::general_purpose::STANDARD.encode(png), "image/png"));
        }
        let mut result = if output.is_error { CallToolResult::error(content) } else { CallToolResult::success(content) };
        result.structured_content = Some(output.structured);
        Ok(result.into())
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        let mut list = vec![
            Resource::new("bodo://project", "project").with_description("the whole project as compact text").with_mime_type("text/plain"),
            Resource::new("bodo://mixer", "mixer").with_description("the mixer: inserts, channels and routes").with_mime_type("text/plain"),
            Resource::new("bodo://docs/commands", "commands").with_description("the command reference").with_mime_type("text/markdown"),
        ];
        let project = self.ask("inspect_project", json!({})).await?;
        for pattern in project.structured.get("patterns").and_then(Value::as_array).into_iter().flatten() {
            let number = pattern.get("number").and_then(Value::as_u64).unwrap_or(0);
            let name = pattern.get("name").and_then(Value::as_str).unwrap_or("pattern");
            list.push(
                Resource::new(format!("bodo://pattern/{number}"), format!("pattern {number}"))
                    .with_description(format!("pattern {number}, \"{name}\": every note"))
                    .with_mime_type("text/plain"),
            );
        }
        Ok(ListResourcesResult::with_all_items(list))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, McpError> {
        let uri = request.uri.as_str();
        let text = match uri {
            "bodo://docs/commands" => commands_doc().to_owned(),
            "bodo://project" => self.ask("inspect_project", json!({})).await?.text,
            "bodo://mixer" => self.ask("inspect_mixer", json!({})).await?.text,
            _ => {
                let Some(number) = uri.strip_prefix("bodo://pattern/").and_then(|n| n.parse::<u64>().ok()) else {
                    return Err(McpError::resource_not_found(format!("no resource {uri}"), None));
                };
                let output = self.ask("inspect_pattern", json!({ "pattern": number })).await?;
                if output.is_error {
                    return Err(McpError::resource_not_found(output.text, None));
                }
                output.text
            }
        };
        Ok(ReadResourceResult::new(vec![ResourceContents::text(text, uri)]).into())
    }
}

/// Serves MCP over stdin and stdout until the client disconnects.
pub fn serve_stdio(server: Server) -> Result<(), ServeError> {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(ServeError::Runtime)?;
    runtime.block_on(async {
        let running = server.serve(rmcp::transport::stdio()).await.map_err(|e| ServeError::Mcp(e.to_string()))?;
        running.waiting().await.map_err(|e| ServeError::Mcp(e.to_string()))?;
        Ok(())
    })
}
