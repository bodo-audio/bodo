//! `cargo xtask deny`: the license and dependency policy (project.md section 3.1, ADR 0002).
//!
//! 1. First-party licenses: Kit crates are `MIT OR Apache-2.0` and carry `LICENSE-MIT` and `LICENSE-APACHE`; every
//!    other crate is `GPL-3.0-or-later` and carries `LICENSE`. The copies match the files at the repository root.
//! 2. `deny.toml` over the whole workspace: GPL-compatible licenses, bans, advisories and sources.
//! 3. `kit/deny.toml` over the Kit crates only: permissive and MPL-2.0 licenses.
//! 4. A self-check on `xtask/fixtures/licenses`, proving that step 3 rejects a GPL dependency of a Kit crate while
//!    step 2 accepts the same dependency in a DAW crate.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use anyhow::{Context, Result, bail};
use cargo_metadata::Metadata;

use crate::workspace;

/// Runs every step on the repository's workspace.
pub fn run() -> Result<()> {
    let root = workspace::root();
    let manifest = root.join("Cargo.toml");
    let metadata = workspace::metadata(&manifest)?;
    workspace::report("deny: first-party licenses", first_party(&root, &metadata))?;

    require_cargo_deny()?;
    let workspace_policy = root.join("deny.toml");
    let kit_policy = root.join("kit/deny.toml");
    workspace::run(cargo_deny(&manifest, &workspace_policy, &[], false).arg("check"))?;
    workspace::run(cargo_deny(&manifest, &kit_policy, &non_kit_members(&metadata), false).args(["check", "licenses"]))?;

    self_check(&root, &workspace_policy, &kit_policy)?;
    println!("deny: ok");
    Ok(())
}

/// Problems with the license fields and files of the workspace's own crates.
fn first_party(root: &Path, metadata: &Metadata) -> Vec<String> {
    let mut findings = Vec::new();
    for package in metadata.workspace_packages() {
        let name = package.name.as_str();
        let dir = workspace::relative_dir(metadata, package);
        let (expected, files): (&str, &[&str]) =
            if workspace::is_kit(&dir) { ("MIT OR Apache-2.0", &["LICENSE-MIT", "LICENSE-APACHE"]) } else { ("GPL-3.0-or-later", &["LICENSE"]) };
        if package.license.as_deref() != Some(expected) {
            findings.push(format!("{name} must set `license = \"{expected}\"`, found {:?}", package.license));
        }
        for file in files {
            match (fs::read(root.join(&dir).join(file)), fs::read(root.join(file))) {
                (Ok(copy), Ok(original)) if copy == original => {}
                (Ok(_), Ok(_)) => findings.push(format!("{dir}/{file} differs from the {file} at the repository root")),
                (Err(_), _) => findings.push(format!("{dir}/{file} is missing; copy it from the repository root")),
                (_, Err(err)) => findings.push(format!("{file} at the repository root could not be read: {err}")),
            }
        }
    }
    findings
}

/// Workspace crates outside `kit/`, which the Kit license policy leaves out.
fn non_kit_members(metadata: &Metadata) -> Vec<String> {
    metadata
        .workspace_packages()
        .into_iter()
        .filter(|package| !workspace::is_kit(&workspace::relative_dir(metadata, package)))
        .map(|package| package.name.to_string())
        .collect()
}

fn require_cargo_deny() -> Result<()> {
    let found = workspace::cargo().args(["deny", "--version"]).output().is_ok_and(|output| output.status.success());
    if !found {
        bail!("cargo-deny is not installed; install it with `cargo install cargo-deny --locked`");
    }
    Ok(())
}

/// `cargo deny` over every crate of the workspace at `manifest` except `exclude`, with the policy in `config`.
fn cargo_deny(manifest: &Path, config: &Path, exclude: &[String], offline: bool) -> Command {
    let mut command = workspace::cargo();
    command.arg("deny").arg("--manifest-path").arg(manifest).args(["--workspace", "--locked"]);
    if offline {
        command.arg("--offline");
    }
    for name in exclude {
        command.args(["--exclude", name]);
    }
    command.arg("--config").arg(config);
    command
}

/// Runs the license checks on the fixture and fails unless each one gives the expected verdict.
fn self_check(root: &Path, workspace_policy: &Path, kit_policy: &Path) -> Result<()> {
    let manifest = root.join("xtask/fixtures/licenses/Cargo.toml");
    let licenses = |config: &Path, exclude: &[&str]| -> Result<Output> {
        let exclude: Vec<String> = exclude.iter().map(|name| (*name).to_owned()).collect();
        cargo_deny(&manifest, config, &exclude, true)
            .args(["check", "licenses"])
            .output()
            .with_context(|| format!("could not run cargo deny on {}", manifest.display()))
    };
    let explain = |output: &Output| String::from_utf8_lossy(&output.stderr).into_owned();

    let kit = licenses(kit_policy, &["bodo-model"])?;
    if kit.status.success() || !explain(&kit).contains("gpl-thing") {
        bail!("deny self-check: the Kit policy should reject gpl-thing in the fixture's bodo-kit-core\n{}", explain(&kit));
    }
    let kit_without_violation = licenses(kit_policy, &["bodo-model", "bodo-kit-core"])?;
    if !kit_without_violation.status.success() {
        bail!("deny self-check: the Kit policy should ignore the dependencies of excluded crates\n{}", explain(&kit_without_violation));
    }
    let daw = licenses(workspace_policy, &[])?;
    if !daw.status.success() {
        bail!("deny self-check: the workspace policy should allow gpl-thing in the fixture's bodo-model\n{}", explain(&daw));
    }
    println!("deny: self-check ok (the Kit policy rejects a GPL dependency that DAW crates may use)");
    Ok(())
}
