//! `cargo xtask text`: text style in every tracked or new file (AGENTS.md section 5).
//!
//! No em-dashes in code, comments or docs. Binary files are skipped.

use std::fs;
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::workspace;

const EM_DASH: char = '\u{2014}';

/// Runs the check on every file git tracks or would track.
pub fn run() -> Result<()> {
    let root = workspace::root();
    let output = Command::new("git")
        .args(["ls-files", "-z", "--cached", "--others", "--exclude-standard"])
        .current_dir(&root)
        .output()
        .context("could not run git ls-files")?;
    if !output.status.success() {
        bail!("git ls-files failed: {}", String::from_utf8_lossy(&output.stderr));
    }
    let mut findings = Vec::new();
    for path in output.stdout.split(|&byte| byte == 0).filter(|path| !path.is_empty()) {
        let path = String::from_utf8_lossy(path);
        // Deleted files and binary files fail to read as text; neither can hold a style problem.
        let Ok(text) = fs::read_to_string(root.join(&*path)) else { continue };
        findings.extend(check(&path, &text));
    }
    workspace::report("text", findings)
}

/// The style problems in `text`, which was read from `path`.
fn check(path: &str, text: &str) -> Vec<String> {
    text.lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let column = line.chars().position(|c| c == EM_DASH)?;
            Some(format!("{path}:{}:{}: em-dash; use a colon, a comma, parentheses or two sentences", index + 1, column + 1))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_each_line_with_an_em_dash() {
        let text = format!("fine - this is a hyphen\nnot fine {EM_DASH} at all\nalso {EM_DASH}{EM_DASH}\n");
        assert_eq!(
            check("docs/x.md", &text),
            [
                "docs/x.md:2:10: em-dash; use a colon, a comma, parentheses or two sentences",
                "docs/x.md:3:6: em-dash; use a colon, a comma, parentheses or two sentences",
            ]
        );
        assert!(check("a.rs", "en dash \u{2013} is fine").is_empty());
    }
}
