//! `bodo-cli`: Bodo from the command line (project.md section 2).
//!
//! - [`session`]: sessions on project files, with Bodo's factory content.
//! - [`render`]: offline rendering of a project's song or one pattern to a WAV file.
//! - [`link`]: the control channel to the running app, behind `bodo-cli mcp --connect`.
//!
//! The binary in `main.rs` parses the command line and calls these.

#![forbid(unsafe_code)]

pub mod link;
pub mod render;
pub mod session;
