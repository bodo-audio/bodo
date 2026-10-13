//! `cargo xtask layers`: crate layering and the dependency rules in AGENTS.md section 2.
//!
//! - Every workspace crate has a place: a layer from the table below, a macro crate, or plugins, apps or tooling by
//!   directory.
//! - Macro crates (proc macros) sit below every layer: any crate may use them, and they use no workspace crate.
//! - A crate depends only on crates in strictly lower layers. Kit (L0 to L4) never depends on DAW crates (L5 and
//!   up), stock plugins depend on Kit only, and only apps depend on plugins.
//! - `tokio`, `reqwest` and `rmcp` are reachable only from L8 and apps, through any chain of dependencies.
//! - Only `bodo-kit-ui`, `bodo-ui` and apps depend on egui directly.

use std::collections::{HashMap, VecDeque};
use std::fmt;

use anyhow::Result;
use cargo_metadata::{DependencyKind, Metadata, Node, PackageId};

use crate::workspace;

/// The layer of every library crate. Keep in sync with AGENTS.md section 2.
const LAYERS: &[(&str, u8)] = &[
    ("bodo-kit-core", 0),
    ("bodo-kit-midi", 0),
    ("bodo-kit-dsp", 1),
    ("bodo-kit-audio", 1),
    ("bodo-kit-analysis", 2),
    ("bodo-kit-music", 2),
    ("bodo-kit-graph", 2),
    ("bodo-kit-plugin", 3),
    ("bodo-kit-host", 3),
    ("bodo-kit-device", 3),
    ("bodo-kit-ui", 3),
    ("bodo-kit-plugin-clap", 4),
    ("bodo-kit-plugin-vst3", 4),
    ("bodo-kit-plugin-au", 4),
    ("bodo-kit-plugin-standalone", 4),
    ("bodo-model", 5),
    ("bodo-io", 6),
    ("bodo-mix", 6),
    ("bodo-engine", 7),
    ("bodo-llm", 8),
    ("bodo-mcp", 8),
    ("bodo-agent", 8),
    ("bodo-control", 8),
    ("bodo-market", 8),
    ("bodo-ui", 8),
];

/// Proc-macro crates. They only generate code, so they sit below every layer. Keep in sync with AGENTS.md section 2.
const MACRO_CRATES: &[&str] = &["bodo-kit-macros"];

/// The highest Kit layer. Layers above it are DAW crates.
const KIT_TOP: u8 = 4;
/// The lowest layer allowed to reach an async runtime or network stack.
const ASYNC_FROM: u8 = 8;
/// Crates that make a crate asynchronous.
const ASYNC_CRATES: &[&str] = &["tokio", "reqwest", "rmcp"];
/// Library crates allowed to depend on egui directly. Apps are allowed too.
const EGUI_USERS: &[&str] = &["bodo-kit-ui", "bodo-ui"];

/// Where a workspace crate sits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tier {
    Macro,
    Layer(u8),
    Plugin,
    App,
    Tooling,
}

impl fmt::Display for Tier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Macro => f.write_str("macros"),
            Self::Layer(layer) => write!(f, "L{layer}"),
            Self::Plugin => f.write_str("plugin"),
            Self::App => f.write_str("app"),
            Self::Tooling => f.write_str("tooling"),
        }
    }
}

fn tier(name: &str, dir: &str) -> Option<Tier> {
    if MACRO_CRATES.contains(&name) {
        return Some(Tier::Macro);
    }
    if let Some(&(_, layer)) = LAYERS.iter().find(|(crate_name, _)| *crate_name == name) {
        return Some(Tier::Layer(layer));
    }
    match dir.split('/').next() {
        Some("plugins") => Some(Tier::Plugin),
        Some("apps") => Some(Tier::App),
        Some("xtask") => Some(Tier::Tooling),
        _ => None,
    }
}

/// The top-level directory a crate of this tier lives in.
fn home(tier: Tier) -> &'static str {
    match tier {
        Tier::Macro => "kit",
        Tier::Layer(layer) if layer <= KIT_TOP => "kit",
        Tier::Layer(_) => "crates",
        Tier::Plugin => "plugins",
        Tier::App => "apps",
        Tier::Tooling => "xtask",
    }
}

/// Why a crate in `from` may not depend on a crate in `to`, or `None` when it may.
fn edge_violation(from: Tier, to: Tier) -> Option<&'static str> {
    match (from, to) {
        (_, Tier::Tooling) => Some("nothing may depend on build tooling"),
        (Tier::Tooling, _) => None,
        (Tier::Macro, _) => Some("macro crates may not depend on other workspace crates"),
        (_, Tier::Macro) => None,
        (Tier::App, Tier::App) => Some("an app may not depend on another app"),
        (Tier::App, _) => None,
        (Tier::Plugin, Tier::Layer(layer)) if layer <= KIT_TOP => None,
        (Tier::Plugin, _) => Some("stock plugins may depend on Bodo Kit (L0 to L4) only"),
        (Tier::Layer(_), Tier::App | Tier::Plugin) => Some("only apps may depend on apps and stock plugins"),
        (Tier::Layer(from), Tier::Layer(to)) if from <= KIT_TOP && to > KIT_TOP => Some("Bodo Kit may never depend on DAW crates (license boundary)"),
        (Tier::Layer(from), Tier::Layer(to)) if to >= from => Some("a crate may depend only on crates in strictly lower layers"),
        (Tier::Layer(_), Tier::Layer(_)) => None,
    }
}

fn is_egui(name: &str) -> bool {
    name == "eframe" || name.starts_with("egui")
}

/// Runs the check on the repository's workspace.
pub fn run() -> Result<()> {
    let metadata = workspace::metadata(&workspace::root().join("Cargo.toml"))?;
    workspace::report("layers", check(&metadata))
}

/// Every layering problem in the workspace described by `metadata`, sorted.
pub fn check(metadata: &Metadata) -> Vec<String> {
    let mut findings = Vec::new();
    let names: HashMap<&PackageId, &str> = metadata.packages.iter().map(|package| (&package.id, package.name.as_str())).collect();
    let name_of = |id: &PackageId| names.get(id).copied().unwrap_or("<unknown>");

    let mut tiers: HashMap<&PackageId, Tier> = HashMap::new();
    for package in metadata.workspace_packages() {
        let name = package.name.as_str();
        let dir = workspace::relative_dir(metadata, package);
        let Some(tier) = tier(name, &dir) else {
            findings.push(format!("{name} ({dir}) has no layer: add it to xtask/src/layers.rs and AGENTS.md section 2"));
            continue;
        };
        let expected = home(tier);
        if dir.split('/').next() != Some(expected) {
            findings.push(format!("{name} is {tier} and belongs under {expected}/, not {dir}/"));
        }
        tiers.insert(&package.id, tier);
    }

    let Some(resolve) = &metadata.resolve else {
        findings.push(String::from("cargo metadata returned no dependency graph"));
        return findings;
    };
    let nodes: HashMap<&PackageId, &Node> = resolve.nodes.iter().map(|node| (&node.id, node)).collect();

    for (&id, &from) in &tiers {
        let Some(node) = nodes.get(id) else { continue };
        let from_name = name_of(id);
        for dep in &node.deps {
            let dep_name = name_of(&dep.pkg);
            if let Some(&to) = tiers.get(&dep.pkg) {
                if let Some(reason) = edge_violation(from, to) {
                    let dev_only = dep.dep_kinds.iter().all(|kind| kind.kind == DependencyKind::Development);
                    let how = if dev_only { " as a dev-dependency" } else { "" };
                    findings.push(format!("{from_name} ({from}) depends on {dep_name} ({to}){how}: {reason}"));
                }
            } else if is_egui(dep_name) && from != Tier::App && !EGUI_USERS.contains(&from_name) {
                findings.push(format!("{from_name} depends on {dep_name}: egui is allowed only in bodo-kit-ui, bodo-ui and apps"));
            }
        }
        let synchronous = matches!(from, Tier::Layer(layer) if layer < ASYNC_FROM) || from == Tier::Plugin;
        if synchronous && let Some(path) = async_path(id, &nodes, &tiers, &name_of) {
            findings.push(format!("{from_name} ({from}) reaches {}: tokio, reqwest and rmcp are allowed only in L8 and apps", path.join(" -> ")));
        }
    }
    findings.sort();
    findings
}

/// The shortest chain of normal dependencies from `start` through external crates to an async crate, if any.
/// Other workspace crates are not followed, since they are checked on their own.
///
/// Complexity: O(V + E) in the dependency graph.
fn async_path<'a>(
    start: &'a PackageId,
    nodes: &HashMap<&'a PackageId, &'a Node>,
    tiers: &HashMap<&'a PackageId, Tier>,
    name_of: &dyn Fn(&PackageId) -> &'a str,
) -> Option<Vec<&'a str>> {
    let mut parent: HashMap<&PackageId, &PackageId> = HashMap::new();
    let mut queue = VecDeque::from([start]);
    while let Some(id) = queue.pop_front() {
        let Some(node) = nodes.get(id) else { continue };
        for dep in &node.deps {
            let normal = dep.dep_kinds.iter().any(|kind| kind.kind == DependencyKind::Normal);
            if !normal || tiers.contains_key(&dep.pkg) || parent.contains_key(&dep.pkg) || &dep.pkg == start {
                continue;
            }
            parent.insert(&dep.pkg, id);
            if ASYNC_CRATES.contains(&name_of(&dep.pkg)) {
                let mut path = vec![name_of(&dep.pkg)];
                let mut current = &dep.pkg;
                while let Some(&previous) = parent.get(current) {
                    path.push(name_of(previous));
                    current = previous;
                }
                path.reverse();
                return Some(path);
            }
            queue.push_back(&dep.pkg);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges_follow_the_rules() {
        use Tier::*;
        assert_eq!(edge_violation(Layer(1), Layer(0)), None);
        assert!(edge_violation(Layer(0), Layer(0)).is_some());
        assert!(edge_violation(Layer(0), Layer(1)).is_some());
        assert_eq!(edge_violation(Layer(4), Layer(5)), Some("Bodo Kit may never depend on DAW crates (license boundary)"));
        assert_eq!(edge_violation(Plugin, Layer(4)), None);
        assert!(edge_violation(Plugin, Layer(5)).is_some());
        assert!(edge_violation(Plugin, Plugin).is_some());
        assert!(edge_violation(Layer(8), Plugin).is_some());
        assert_eq!(edge_violation(App, Plugin), None);
        assert_eq!(edge_violation(App, Layer(8)), None);
        assert!(edge_violation(App, App).is_some());
        assert!(edge_violation(App, Tooling).is_some());
        assert_eq!(edge_violation(Tooling, App), None);
        assert_eq!(edge_violation(Layer(0), Macro), None);
        assert_eq!(edge_violation(Plugin, Macro), None);
        assert!(edge_violation(Macro, Layer(0)).is_some());
    }

    #[test]
    fn tiers_come_from_the_table_or_the_directory() {
        assert_eq!(tier("bodo-kit-core", "kit/bodo-kit-core"), Some(Tier::Layer(0)));
        assert_eq!(tier("bodo-kit-macros", "kit/bodo-kit-macros"), Some(Tier::Macro));
        assert_eq!(tier("bodo-engine", "crates/bodo-engine"), Some(Tier::Layer(7)));
        assert_eq!(tier("bodo-eq", "plugins/bodo-eq"), Some(Tier::Plugin));
        assert_eq!(tier("bodo-cli", "apps/bodo-cli"), Some(Tier::App));
        assert_eq!(tier("xtask", "xtask"), Some(Tier::Tooling));
        assert_eq!(tier("bodo-mystery", "crates/bodo-mystery"), None);
    }

    /// The fixture workspace breaks every rule once; the check must report exactly those breaks.
    #[test]
    fn catches_every_deliberate_violation_in_the_fixture() {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/layers/Cargo.toml");
        let metadata = workspace::metadata(&manifest).unwrap();
        let expected = [
            "bodo-engine is L7 and belongs under crates/, not kit/bodo-engine/",
            "bodo-eq (plugin) depends on bodo-model (L5): stock plugins may depend on Bodo Kit (L0 to L4) only",
            "bodo-io (L6) reaches bodo-io -> async-wrapper -> tokio: tokio, reqwest and rmcp are allowed only in L8 and apps",
            "bodo-kit-core (L0) depends on bodo-kit-dsp (L1): a crate may depend only on crates in strictly lower layers",
            "bodo-kit-dsp (L1) reaches bodo-kit-dsp -> tokio: tokio, reqwest and rmcp are allowed only in L8 and apps",
            "bodo-kit-dsp depends on egui: egui is allowed only in bodo-kit-ui, bodo-ui and apps",
            "bodo-kit-midi (L0) depends on bodo-model (L5): Bodo Kit may never depend on DAW crates (license boundary)",
            "bodo-model (L5) depends on bodo-engine (L7) as a dev-dependency: a crate may depend only on crates in strictly lower layers",
            "bodo-mystery (crates/bodo-mystery) has no layer: add it to xtask/src/layers.rs and AGENTS.md section 2",
        ];
        assert_eq!(check(&metadata), expected);
    }
}
