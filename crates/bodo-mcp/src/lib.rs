//! Bodo's MCP server (project.md section 12.3, ADR 0005).
//!
//! External agents such as Claude Code, Codex or Cursor get the same command registry the UI uses:
//!
//! - [`mod@tools`]: the tools without a transport. The generic ones (`list_commands`, `describe_command`, `execute`,
//!   `batch`) reach every command; the inspect ones (`inspect_project`, `inspect_pattern`, `inspect_mixer`) read the
//!   project as compact text. [`Agent`] runs them on a session of its own.
//! - [`server`]: the rmcp server, with the tools, resources (`bodo://project`, `bodo://mixer`, `bodo://pattern/{n}`,
//!   `bodo://docs/commands`) and instructions for agents. [`serve_stdio`] serves it over stdin and stdout.
//! - [`connect`]: tools that act on the running app through its control channel, so the user watches the agent work;
//!   the app answers them with [`connect::answer`] on its own session.
//! - [`args`]: every tool's arguments; their schemas are the tools' input schemas.

#![forbid(unsafe_code)]

pub mod args;
pub mod connect;
pub mod server;
pub mod tools;

pub use server::{CONNECTED, INSTRUCTIONS, ServeError, Server, serve_stdio};
pub use tools::{Agent, ToolInfo, ToolOutput, tools, ui_tools};
