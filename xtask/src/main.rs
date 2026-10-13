//! Build tooling for the Bodo workspace. Run `cargo xtask help` to list the commands.

#![forbid(unsafe_code)]
#![allow(clippy::print_stdout, clippy::print_stderr, reason = "xtask is a terminal tool that reports to the console")]

mod layers;
mod text;
mod unsafe_audit;
mod workspace;

use std::process::ExitCode;

const USAGE: &str = "\
Usage: cargo xtask <command>

Commands:
  layers        Check crate layering and the dependency rules (AGENTS.md section 2)
  unsafe-audit  Check that unsafe code stays in ffi modules of FFI crates and is documented
  text          Check text style in tracked files (no em-dashes)
  help          Show this message";

fn main() -> ExitCode {
    let command = std::env::args().nth(1);
    let result = match command.as_deref() {
        Some("layers") => layers::run(),
        Some("unsafe-audit") => unsafe_audit::run(),
        Some("text") => text::run(),
        None | Some("help" | "-h" | "--help") => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Some(other) => {
            eprintln!("unknown command `{other}`\n\n{USAGE}");
            return ExitCode::FAILURE;
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}
