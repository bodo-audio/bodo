//! `docs/agents/commands.md` is generated from the registry and must match it. Run with `BODO_BLESS=1` to rewrite it.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::PathBuf;

use bodo_engine::Registry;

#[test]
fn the_command_reference_matches_the_registry() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/agents/commands.md");
    let expected = bodo_engine::docs::commands_markdown(&Registry::builtin());
    if std::env::var_os("BODO_BLESS").is_some() {
        std::fs::write(&path, &expected).unwrap();
        return;
    }
    let found = std::fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n");
    assert!(found == expected, "docs/agents/commands.md is out of date; run `BODO_BLESS=1 cargo test -p bodo-engine --test docs`");
}

#[test]
fn every_command_is_documented_well() {
    for spec in Registry::builtin().iter() {
        assert!(spec.summary.ends_with('.'), "{}: the summary is a sentence", spec.id);
        assert!(spec.example.is_some(), "{} has an example", spec.id);
        assert!(!spec.summary.contains('\u{2014}'), "{}: no em-dashes", spec.id);
        for (name, _, _, description) in bodo_engine::docs::params_table(spec) {
            assert!(!description.is_empty(), "{}: param {name} is documented", spec.id);
        }
    }
}
