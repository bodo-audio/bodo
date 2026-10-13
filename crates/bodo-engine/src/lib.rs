//! The Bodo command engine (project.md section 12, ADR 0005).
//!
//! Every change to a project goes through [`Session::execute`]: the UI, shortcuts, the command palette, MCP and the
//! assistant all call it with a command id and JSON params.
//!
//! - [`registry`]: the commands, each with a JSON Schema, effects, an optional merge key, menu entry and shortcut.
//! - [`session`]: the current project, transactions and batches, undo and redo, and the events the frontend acts on.
//! - [`state`]: the editor's selection and current pattern, and the transport.
//! - [`params`] and [`resolve`]: positions as ticks, `bar:beat:tick` or seconds; entities by id, number or name.
//! - [`inspect`]: compact text and JSON summaries for agents.
//! - [`library`]: the sounds the browser offers.
//! - [`docs`]: the command reference, generated from the registry.
//!
//! The engine is synchronous and knows nothing about audio devices or the UI: whoever plays audio reads the events.

#![forbid(unsafe_code)]

mod commands;
pub mod demo;
pub mod docs;
pub mod error;
mod history;
pub mod ids;
pub mod inspect;
pub mod library;
pub mod palette;
pub mod params;
pub mod registry;
pub mod resolve;
pub mod samples;
pub mod session;
pub mod state;
pub mod template;

pub use commands::{insert_number, pattern_length, pattern_step_text};
pub use error::{CommandError, ErrorCode};
pub use history::HistoryLimits;
pub use registry::{CommandSpec, Effects, Registry};
pub use session::{BatchError, BatchOutput, Call, CommandOutput, Ctx, Env, Session};
pub use state::{EditorState, Event, PlayMode, TransportState};
pub use template::{Template, locate_factory};
