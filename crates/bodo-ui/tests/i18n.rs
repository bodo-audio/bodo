//! The translation catalogs against the code (project.md section 14.7, ADR 0033): every English text the interface
//! translates is in every catalog, no catalog keeps text the code no longer uses, each translation has its key's
//! placeholders, no key is listed twice, every `tf!` fills exactly the placeholders its text has, and no egui label,
//! button or hint takes an English literal without going through `t`.
//!
//! The keys are found the way people write them: the literal passed to `t`, `tf!` or `en`, or to a helper that
//! translates what it gets, such as `app.hint(&response, "...")`. Command titles and summaries come from the command
//! registry and the interface's own commands.

#![allow(clippy::unwrap_used, clippy::panic, reason = "tests")]

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use bodo_ui::i18n::{self, Language};
use egui::Theme;

/// Calls that translate English text they are given, with how many arguments come before the text.
const SINKS: &[(&str, usize)] = &[
    ("t", 0),
    ("tf!", 0),
    ("en", 0),
    ("hint", 1),
    ("set_status", 0),
    ("icon_button", 2),
    ("painted_button", 1),
    ("placeholder", 2),
    ("item", 1),
    ("command", 2),
    ("submenu", 1),
    ("field", 1),
    ("field", 2),
    ("heading", 1),
    ("labeled", 1),
    ("row", 1),
    ("empty", 2),
];

/// egui calls that show text, with how many arguments come before the text. A literal with letters in any of these
/// places is English that skipped translation.
const SHOWN: &[(&str, usize)] = &[
    (".label", 0),
    (".button", 0),
    (".small_button", 0),
    (".heading", 0),
    (".menu_button", 0),
    (".on_hover_text", 0),
    (".on_disabled_hover_text", 0),
    (".hint_text", 0),
    (".selected_text", 0),
    (".suffix", 0),
    (".prefix", 0),
    ("Button::new", 0),
    ("RichText::new", 0),
    ("Label::new", 0),
    ("Window::new", 0),
    ("ComboBox::from_label", 0),
    (".selectable_label", 1),
    (".checkbox", 1),
    (".radio", 1),
    (".toggle_value", 1),
    (".radio_value", 2),
    (".selectable_value", 2),
];

/// Literals that read the same in every language: the wordmark and one-letter marks such as S for solo.
const NEUTRAL: &[&str] = &["bodo", "Bodo", "S", "M", "BPM", " BPM", "Hz", "dB", "ms", " ms", "MIDI"];

/// Every Rust source of the interface and the app, with its path relative to its `src`, comments blanked out and any
/// tests at its end cut off.
fn sources() -> Vec<(String, String)> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    for root in [manifest.join("src"), manifest.join("../../apps/bodo/src")] {
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path: PathBuf = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let relative = path.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
                    let text = std::fs::read_to_string(&path).unwrap();
                    let text = strip_comments(&text);
                    let text = match text.find("#[cfg(test)]") {
                        Some(at) => text[..at].to_owned(),
                        None => text,
                    };
                    out.push((relative, text));
                }
            }
        }
    }
    out.sort();
    out
}

/// `text` with its comments replaced by spaces, keeping strings, character literals and line numbers.
fn strip_comments(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' {
            let end = end_of_string(&chars, i);
            out.extend(&chars[i..end]);
            i = end;
        } else if c == '\'' && chars.get(i + 2) == Some(&'\'') {
            out.extend(&chars[i..i + 3]);
            i += 3;
        } else if c == '\'' && chars.get(i + 1) == Some(&'\\') {
            let end = chars[i + 2..].iter().position(|c| *c == '\'').map_or(chars.len(), |p| i + 3 + p);
            out.extend(&chars[i..end]);
            i = end;
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                out.push(' ');
                i += 1;
            }
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                out.push(if chars[i] == '\n' { '\n' } else { ' ' });
                i += 1;
            }
            out.push_str("  ");
            i += 2;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// The index just past the string literal that starts at `start`.
fn end_of_string(chars: &[char], start: usize) -> usize {
    let mut i = start + 1;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 2,
            '"' => return i + 1,
            _ => i += 1,
        }
    }
    chars.len()
}

/// The string literal's value, escapes read.
fn unescape(literal: &[char]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < literal.len() {
        if literal[i] == '\\' {
            match literal.get(i + 1) {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(*other),
                None => {}
            }
            i += 2;
        } else {
            out.push(literal[i]);
            i += 1;
        }
    }
    out
}

/// From just inside a call's parenthesis at `i`, the index of argument number `n` (counted from 0), or `None` when
/// the call has fewer.
fn argument(chars: &[char], mut i: usize, mut n: usize) -> Option<usize> {
    let mut depth = 0usize;
    while n > 0 {
        let c = *chars.get(i)?;
        match c {
            '"' => {
                i = end_of_string(chars, i);
                continue;
            }
            '\'' if chars.get(i + 2) == Some(&'\'') => {
                // A character literal such as ','.
                i += 3;
                continue;
            }
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
            }
            '|' if depth == 0 => {
                // A closure's parameters, `|a, b|`, aren't separate arguments.
                let close = chars[i + 1..].iter().position(|c| *c == '|')?;
                i += close + 2;
                continue;
            }
            ',' if depth == 0 => n -= 1,
            _ => {}
        }
        i += 1;
    }
    while chars.get(i).is_some_and(|c| c.is_whitespace() || *c == '&') {
        i += 1;
    }
    Some(i)
}

/// The string literal at `i`, if one starts there, with its line.
fn literal_at(chars: &[char], i: usize) -> Option<String> {
    (chars.get(i) == Some(&'"')).then(|| unescape(&chars[i + 1..end_of_string(chars, i) - 1]))
}

fn line_of(chars: &[char], i: usize) -> usize {
    chars[..i].iter().filter(|c| **c == '\n').count() + 1
}

/// Every call of `name(` in `chars` that isn't part of a longer name, with the index just inside its parenthesis.
fn calls<'a>(chars: &'a [char], name: &'a str) -> impl Iterator<Item = (usize, usize)> + 'a {
    let pattern: Vec<char> = name.chars().chain(std::iter::once('(')).collect();
    let bare = !name.starts_with('.') && !name.contains("::");
    (0..chars.len().saturating_sub(pattern.len())).filter_map(move |i| {
        if chars[i..i + pattern.len()] != pattern[..] {
            return None;
        }
        let before = if i == 0 { ' ' } else { chars[i - 1] };
        if bare && (before.is_alphanumeric() || before == '_' || before == '.' && name.len() < 3) {
            return None;
        }
        if !bare && name.contains("::") && (before.is_alphanumeric() || before == '_') && !chars[..i].ends_with(&[':', ':']) {
            return None;
        }
        // `fn t(` defines rather than calls.
        if chars[..i].ends_with(&['f', 'n', ' ']) {
            return None;
        }
        Some((i, i + pattern.len()))
    })
}

/// The English keys the sources translate, each with the first file that uses it.
fn keys(sources: &[(String, String)]) -> BTreeMap<String, String> {
    let mut keys = BTreeMap::new();
    for (path, text) in sources {
        let chars: Vec<char> = text.chars().collect();
        for (name, skip) in SINKS {
            for (_, inside) in calls(&chars, name) {
                if let Some(key) = argument(&chars, inside, *skip).and_then(|at| literal_at(&chars, at))
                    && key.chars().any(char::is_alphabetic)
                {
                    keys.entry(key).or_insert_with(|| path.clone());
                }
            }
        }
        // Menu titles: `title: "Edit"`.
        let mut from = 0;
        while let Some(at) = text[from..].find("title: \"") {
            let start = from + at + "title: ".len();
            let start_chars = text[..start].chars().count();
            if let Some(key) = literal_at(&chars, start_chars) {
                keys.entry(key).or_insert_with(|| path.clone());
            }
            from = start + 1;
        }
    }
    keys
}

/// Command titles and summaries, from the registry and the interface's own commands, and the texts of the native
/// effects people see: their names, descriptions, parameter names and choices.
fn command_texts() -> BTreeSet<String> {
    let app = common::app(Theme::Dark);
    let mut texts = BTreeSet::new();
    for spec in app.session.registry().iter() {
        texts.insert(spec.title.to_owned());
        texts.insert(spec.summary.to_owned());
    }
    for (_, title, summary) in bodo_ui::BodoApp::ui_commands() {
        texts.insert((*title).to_owned());
        texts.insert((*summary).to_owned());
    }
    for info in bodo_mix::fx::catalog().iter().filter(|i| !i.hidden) {
        texts.insert(info.name.to_owned());
        texts.insert(info.description.to_owned());
        for param in info.params {
            texts.insert(param.name.to_owned());
            if let bodo_mix::fx::ParamKind::Choice { choices } = param.kind {
                texts.extend(choices.iter().map(|c| (*c).to_owned()));
            }
        }
    }
    texts
}

/// The placeholders in `text`, such as `name` in "Delete {name}".
fn placeholders(text: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(close) if !after[..close].is_empty() && after[..close].chars().all(|c| c.is_ascii_alphanumeric() || c == '_') => {
                found.insert(after[..close].to_owned());
                rest = &after[close + 1..];
            }
            _ => rest = after,
        }
    }
    found
}

fn turkish() -> &'static i18n::Catalog {
    i18n::catalog(Language::Turkish).unwrap()
}

#[test]
fn every_key_the_interface_uses_is_in_every_catalog() {
    let mut wanted: BTreeMap<String, String> = keys(&sources());
    for text in command_texts() {
        wanted.entry(text).or_insert_with(|| String::from("(commands)"));
    }
    let missing: Vec<String> =
        wanted.iter().filter(|(key, _)| !turkish().contains_key(key.as_str())).map(|(key, path)| format!("{path}\t{key}")).collect();
    assert!(missing.is_empty(), "{} keys have no Turkish translation:\n{}", missing.len(), missing.join("\n"));
}

#[test]
fn no_catalog_keeps_keys_the_code_no_longer_uses() {
    let mut used: BTreeSet<String> = keys(&sources()).into_keys().collect();
    used.extend(command_texts());
    let stale: Vec<&String> = turkish().keys().filter(|key| !used.contains(*key)).collect();
    assert!(stale.is_empty(), "stale Turkish keys: {stale:#?}");
}

#[test]
fn translations_keep_their_keys_placeholders() {
    let wrong: Vec<String> =
        turkish().iter().filter(|(key, value)| placeholders(key) != placeholders(value)).map(|(key, value)| format!("{key} -> {value}")).collect();
    assert!(wrong.is_empty(), "translations whose placeholders differ from their keys':\n{}", wrong.join("\n"));
}

#[test]
fn no_catalog_lists_a_key_twice() {
    let source = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("i18n/tr.tsv")).unwrap();
    assert!(source.starts_with("en\ttr\n"), "the header names the languages");
    let mut seen = BTreeSet::new();
    let mut twice = Vec::new();
    for (number, line) in source.lines().enumerate().skip(1) {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('\t') else { panic!("line {}: no tab between the key and its translation", number + 1) };
        assert!(!value.trim().is_empty(), "line {}: `{key}` has an empty translation", number + 1);
        assert!(!value.contains('\t'), "line {}: more than one tab", number + 1);
        if !seen.insert(key.to_owned()) {
            twice.push(key.to_owned());
        }
    }
    assert!(twice.is_empty(), "keys listed twice: {twice:?}");
}

#[test]
fn every_tf_fills_exactly_its_placeholders() {
    let mut wrong = Vec::new();
    for (path, text) in sources() {
        let chars: Vec<char> = text.chars().collect();
        for (at, inside) in calls(&chars, "tf!") {
            let Some(key) = literal_at(&chars, argument(&chars, inside, 0).unwrap_or(inside)) else { continue };
            // The names before `=` at the call's top level.
            let mut names = BTreeSet::new();
            let mut n = 1;
            while let Some(start) = argument(&chars, inside, n) {
                let name: String = chars[start..].iter().take_while(|c| c.is_alphanumeric() || **c == '_').collect();
                if name.is_empty() {
                    break;
                }
                names.insert(name);
                n += 1;
            }
            if names != placeholders(&key) {
                wrong.push(format!("{path}:{}: `{key}` fills {names:?}", line_of(&chars, at)));
            }
        }
    }
    assert!(wrong.is_empty(), "tf! calls that leave a placeholder unfilled or fill one the text lacks:\n{}", wrong.join("\n"));
}

/// The English literals `text` shows through egui without translating them, as `path:line: call("text")`.
fn untranslated(path: &str, text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut found = Vec::new();
    for (name, skip) in SHOWN {
        for (at, inside) in calls(&chars, name) {
            let Some(literal) = argument(&chars, inside, *skip).and_then(|i| literal_at(&chars, i)) else { continue };
            if literal.chars().any(char::is_alphabetic) && !NEUTRAL.contains(&literal.as_str()) {
                found.push(format!("{path}:{}: {name}(\"{literal}\")", line_of(&chars, at)));
            }
        }
    }
    found.sort();
    found
}

#[test]
fn no_egui_text_skips_translation() {
    let found: Vec<String> = sources().into_iter().flat_map(|(path, text)| untranslated(&path, &text)).collect();
    assert!(found.is_empty(), "English shown without t():\n{}", found.join("\n"));
}

#[test]
fn the_scanner_finds_keys_the_ways_they_are_written() {
    let text = strip_comments(
        r#"
        // t("A comment")
        ui.button(t("Save"));
        app.hint(&response, "Click to play");
        let label = tf!("Delete {name}", name = channel.name);
        let shown = en("Major");
        ui.label("Raw");
        Menu { title: "Edit", items }
        "#,
    );
    let keys = keys(&[(String::from("x.rs"), text)]);
    let keys: Vec<&str> = keys.keys().map(String::as_str).collect();
    assert_eq!(keys, ["Click to play", "Delete {name}", "Edit", "Major", "Save"]);
    assert_eq!(placeholders("{count} clips from {at}, {not a placeholder}"), BTreeSet::from([String::from("at"), String::from("count")]));
    let shown = "ui.label(\"Raw\");\nui.checkbox(&mut on, \"Loop\");\nui.label(t(\"Fine\"));\nui.selectable_label(a == b, \"S\");";
    assert_eq!(untranslated("x.rs", shown), ["x.rs:1: .label(\"Raw\")", "x.rs:2: .checkbox(\"Loop\")"]);
}
