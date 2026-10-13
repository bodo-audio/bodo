//! Shared helpers: workspace metadata, running cargo, and reporting findings.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use cargo_metadata::{CargoOpt, Metadata, MetadataCommand, Package};

/// The repository root, the directory above this crate.
pub fn root() -> PathBuf {
    let xtask_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    xtask_dir.parent().unwrap_or(xtask_dir).to_path_buf()
}

/// Metadata for the workspace at `manifest`, resolved with every feature enabled so that optional dependencies are
/// checked too.
pub fn metadata(manifest: &Path) -> Result<Metadata> {
    MetadataCommand::new()
        .manifest_path(manifest)
        .features(CargoOpt::AllFeatures)
        .exec()
        .with_context(|| format!("cargo metadata failed for {}", manifest.display()))
}

/// The directory of `package` relative to the workspace root, with `/` separators.
pub fn relative_dir(metadata: &Metadata, package: &Package) -> String {
    let dir = package.manifest_path.parent().unwrap_or(&package.manifest_path);
    let relative = dir.strip_prefix(&metadata.workspace_root).unwrap_or(dir);
    relative.as_str().replace('\\', "/")
}

/// Whether `relative_dir` belongs to Bodo Kit.
pub fn is_kit(relative_dir: &str) -> bool {
    relative_dir.starts_with("kit/")
}

/// A `cargo` command, using the same cargo that started the xtask.
pub fn cargo() -> Command {
    Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
}

/// Runs `command` with inherited output and fails if it exits unsuccessfully.
pub fn run(command: &mut Command) -> Result<()> {
    let status = command.status().with_context(|| format!("could not run {command:?}"))?;
    if !status.success() {
        bail!("{command:?} exited with {status}");
    }
    Ok(())
}

/// Prints `findings` and fails when there are any.
pub fn report(check: &str, findings: Vec<String>) -> Result<()> {
    if findings.is_empty() {
        println!("{check}: ok");
        return Ok(());
    }
    for finding in &findings {
        eprintln!("error[{check}]: {finding}");
    }
    bail!("{check}: {} problem(s)", findings.len())
}
