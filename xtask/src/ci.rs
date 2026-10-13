//! `cargo xtask ci`: everything CI runs, in one command (project.md section 19.5).
//!
//! Every step runs even when an earlier one fails, so one run shows every problem.

use std::path::Path;

use anyhow::{Result, bail};

use crate::{deny, layers, text, unsafe_audit, workspace};

/// Runs every step and fails if any of them failed.
pub fn run() -> Result<()> {
    let root = workspace::root();
    let mut failed = Vec::new();
    let mut step = |name: &'static str, result: Result<()>| match result {
        Ok(()) => println!("ci: {name} passed"),
        Err(err) => {
            eprintln!("ci: {name} failed: {err:#}");
            failed.push(name);
        }
    };

    step("fmt", cargo(&root, &["fmt", "--all", "--check"], &[]));
    step("clippy", cargo(&root, &["clippy", "--workspace", "--all-targets", "--locked", "--", "-D", "warnings"], &[]));
    step(
        "clippy (all features)",
        cargo(&root, &["clippy", "--workspace", "--all-targets", "--all-features", "--locked", "--", "-D", "warnings"], &[]),
    );
    step("test", cargo(&root, &["test", "--workspace", "--all-features", "--locked"], &[]));
    step("doc", cargo(&root, &["doc", "--workspace", "--no-deps", "--all-features", "--locked"], &[("RUSTDOCFLAGS", "-D warnings")]));
    step("release build", cargo(&root, &["build", "--workspace", "--release", "--locked"], &[]));
    step("layers", layers::run());
    step("unsafe-audit", unsafe_audit::run());
    step("text", text::run());
    step("deny", deny::run());

    if failed.is_empty() {
        println!("ci: all checks passed");
        Ok(())
    } else {
        bail!("ci: {} step(s) failed: {}", failed.len(), failed.join(", "))
    }
}

fn cargo(root: &Path, args: &[&str], env: &[(&str, &str)]) -> Result<()> {
    let mut command = workspace::cargo();
    command.args(args).current_dir(root);
    for (key, value) in env {
        command.env(key, value);
    }
    workspace::run(&mut command)
}
