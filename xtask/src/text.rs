//! `cargo xtask text`: text style in every tracked or new file (AGENTS.md section 5).
//!
//! No em-dashes in code, comments or docs. Binary files are skipped: known binary formats by extension, and anything
//! else that isn't valid UTF-8.

use std::fs;
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::workspace;

const EM_DASH: char = '\u{2014}';

/// Extensions of binary formats the repository holds, such as factory audio and UI snapshots. Their bytes can
/// happen to be valid UTF-8, so they are skipped by name rather than by content.
const BINARY_EXTENSIONS: &[&str] = &["wav", "flac", "aif", "aiff", "mp3", "ogg", "png", "jpg", "jpeg", "ico", "icns", "ttf", "otf", "bodo", "zip"];

/// Whether `path` names a binary format by its extension.
fn is_binary(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| BINARY_EXTENSIONS.iter().any(|binary| binary.eq_ignore_ascii_case(extension)))
}

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
        if is_binary(&path) {
            continue;
        }
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

    #[test]
    fn skips_binary_formats_by_extension() {
        assert!(is_binary("assets/factory/Packs/Bodo Drums/Kick.wav"));
        assert!(is_binary("crates/bodo-ui/tests/snapshots/rack.PNG"));
        assert!(!is_binary("docs/formats/README.md"));
        assert!(!is_binary("xtask/src/text.rs"));
        assert!(!is_binary("LICENSE"));
    }
}
