//! `cargo xtask unsafe-audit`: the unsafe-code boundary in AGENTS.md section 5.
//!
//! - Every workspace crate inherits the workspace lints, which deny `unsafe_code`.
//! - Lib and bin roots declare `#![forbid(unsafe_code)]`, unless the crate is an FFI crate with an `ffi` module.
//! - `#[allow(unsafe_code)]` (or `expect`, or `warn`) appears only on `mod ffi`, and only in FFI crates.
//! - Unsafe code appears only in `ffi.rs` files and `ffi/` directories of FFI crates.
//! - Every unsafe block and unsafe impl is preceded by a `// SAFETY:` comment.
//!
//! The compiler enforces the lints themselves; this audit keeps the escape hatches where the rules allow them.
//! Unsafe code inside macro bodies is invisible here, but the `unsafe_code` lint still catches it once expanded.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use cargo_metadata::{Metadata, Package, TargetKind};
use proc_macro2::Span;
use syn::visit::{self, Visit};

use crate::workspace;

/// Crates allowed to contain FFI code (project.md section 4.1).
const FFI_CRATES: &[&str] = &[
    "bodo-kit-core",
    "bodo-kit-host",
    "bodo-kit-device",
    "bodo-kit-plugin-clap",
    "bodo-kit-plugin-vst3",
    "bodo-kit-plugin-au",
    "bodo-kit-plugin-standalone",
    "bodo-plugin-bridge",
];

/// Runs the audit on the repository's workspace.
pub fn run() -> Result<()> {
    let metadata = workspace::metadata(&workspace::root().join("Cargo.toml"))?;
    let mut findings = Vec::new();
    for package in metadata.workspace_packages() {
        findings.extend(audit_package(&metadata, package)?);
    }
    workspace::report("unsafe-audit", findings)
}

/// One Rust source file of a crate.
struct SourceFile {
    /// Path relative to the workspace root, for messages.
    display: String,
    /// Whether the path is `ffi.rs` or inside an `ffi/` directory.
    in_ffi_module: bool,
    /// Whether this file is a lib or bin root.
    is_root: bool,
    source: String,
}

fn audit_package(metadata: &Metadata, package: &Package) -> Result<Vec<String>> {
    let name = package.name.as_str();
    let mut findings = Vec::new();

    let manifest: toml::Table = fs::read_to_string(&package.manifest_path)
        .with_context(|| format!("could not read {}", package.manifest_path))?
        .parse()
        .with_context(|| format!("could not parse {}", package.manifest_path))?;
    let inherits_lints = manifest.get("lints").and_then(|lints| lints.get("workspace")).and_then(toml::Value::as_bool) == Some(true);
    if !inherits_lints {
        findings.push(format!("{name}: Cargo.toml must set `[lints] workspace = true`"));
    }

    let package_dir = package.manifest_path.parent().map(|dir| dir.as_std_path()).unwrap_or(Path::new("."));
    let roots: Vec<PathBuf> = package
        .targets
        .iter()
        .filter(|target| target.kind.iter().any(|kind| matches!(kind, TargetKind::Lib | TargetKind::Bin | TargetKind::ProcMacro)))
        .map(|target| target.src_path.clone().into_std_path_buf())
        .collect();
    let mut files = Vec::new();
    for path in rust_files(package_dir)? {
        let source = fs::read_to_string(&path).with_context(|| format!("could not read {}", path.display()))?;
        let within_package = path.strip_prefix(package_dir).unwrap_or(&path);
        let in_ffi_module = within_package.file_name().is_some_and(|file| file == "ffi.rs")
            || within_package.parent().is_some_and(|dir| dir.components().any(|part| part.as_os_str() == "ffi"));
        let display = path.strip_prefix(&metadata.workspace_root).unwrap_or(&path).display().to_string().replace('\\', "/");
        files.push(SourceFile { display, in_ffi_module, is_root: roots.contains(&path), source });
    }
    findings.extend(audit_files(name, &files));
    Ok(findings)
}

/// Every `.rs` file under `dir`, skipping `target`, hidden directories and nested packages. Sorted.
fn rust_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).with_context(|| format!("could not list {}", dir.display()))? {
            let path = entry?.path();
            let file_name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
            if path.is_dir() {
                if file_name != "target" && !file_name.starts_with('.') && !path.join("Cargo.toml").exists() {
                    pending.push(path);
                }
            } else if file_name.ends_with(".rs") {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

/// The findings for one crate, given its source files.
fn audit_files(crate_name: &str, files: &[SourceFile]) -> Vec<String> {
    let ffi_crate = FFI_CRATES.contains(&crate_name);
    let mut findings = Vec::new();
    let mut has_ffi_module = false;
    let mut roots_without_forbid = Vec::new();

    for file in files {
        let syntax = match syn::parse_file(&file.source) {
            Ok(syntax) => syntax,
            Err(err) => {
                findings.push(format!("{}:{}: could not parse: {err}", file.display, err.span().start().line));
                continue;
            }
        };
        let scan = Scan::run(&syntax, &file.source);
        for site in &scan.unsafe_sites {
            if !ffi_crate {
                findings.push(format!("{}:{}: {} in {crate_name}, which is not an FFI crate", file.display, site.line, site.what));
            } else if !file.in_ffi_module {
                findings.push(format!("{}:{}: {} outside an ffi module", file.display, site.line, site.what));
            }
        }
        for line in &scan.undocumented {
            findings.push(format!("{}:{line}: unsafe code without a `// SAFETY:` comment", file.display));
        }
        for allow in &scan.lint_relaxations {
            if allow.on_ffi_module && ffi_crate {
                has_ffi_module = true;
            } else {
                findings
                    .push(format!("{}:{}: `{}(unsafe_code)` is allowed only on `mod ffi` in an FFI crate", file.display, allow.line, allow.level));
            }
        }
        if file.is_root && !scan.forbids_unsafe {
            roots_without_forbid.push(&file.display);
        }
    }
    if !has_ffi_module {
        for root in roots_without_forbid {
            findings.push(format!("{root}: crate root must declare `#![forbid(unsafe_code)]`"));
        }
    }
    findings
}

struct UnsafeSite {
    line: usize,
    what: &'static str,
}

/// An attribute that relaxes the `unsafe_code` lint.
struct LintRelaxation {
    line: usize,
    level: String,
    on_ffi_module: bool,
}

/// What one file contains, found by walking its syntax tree.
struct Scan<'a> {
    lines: Vec<&'a str>,
    unsafe_sites: Vec<UnsafeSite>,
    /// Lines of unsafe blocks and impls with no `// SAFETY:` comment.
    undocumented: Vec<usize>,
    lint_relaxations: Vec<LintRelaxation>,
    forbids_unsafe: bool,
}

impl<'a> Scan<'a> {
    fn run(syntax: &syn::File, source: &'a str) -> Self {
        let forbids_unsafe = syntax.attrs.iter().any(|attr| unsafe_code_lint_level(attr).as_deref() == Some("forbid"));
        let mut scan = Self {
            lines: source.lines().collect(),
            unsafe_sites: Vec::new(),
            undocumented: Vec::new(),
            lint_relaxations: Vec::new(),
            forbids_unsafe,
        };
        scan.visit_file(syntax);
        scan
    }

    fn site(&mut self, span: Span, what: &'static str) {
        self.unsafe_sites.push(UnsafeSite { line: span.start().line, what });
    }

    fn require_safety_comment(&mut self, span: Span) {
        let line = span.start().line;
        if !has_safety_comment(&self.lines, line) {
            self.undocumented.push(line);
        }
    }
}

/// Whether line `line` (1-based) or the comment lines directly above it contain `SAFETY:`.
fn has_safety_comment(lines: &[&str], line: usize) -> bool {
    let Some(index) = line.checked_sub(1) else { return false };
    if lines.get(index).is_some_and(|text| text.contains("SAFETY:")) {
        return true;
    }
    lines[..index.min(lines.len())]
        .iter()
        .rev()
        .map(|text| text.trim_start())
        .take_while(|text| text.starts_with("//"))
        .any(|text| text.contains("SAFETY:"))
}

/// The level (`allow`, `deny`, ...) of a lint attribute that names `unsafe_code`, if `attr` is one.
fn unsafe_code_lint_level(attr: &syn::Attribute) -> Option<String> {
    let level = attr.path().get_ident()?.to_string();
    if !matches!(level.as_str(), "allow" | "expect" | "warn" | "deny" | "forbid") {
        return None;
    }
    let metas = attr.parse_args_with(syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated).ok()?;
    metas.iter().any(|meta| meta.path().is_ident("unsafe_code")).then_some(level)
}

fn relaxes(level: &str) -> bool {
    matches!(level, "allow" | "expect" | "warn")
}

impl<'ast> Visit<'ast> for Scan<'_> {
    fn visit_expr_unsafe(&mut self, node: &'ast syn::ExprUnsafe) {
        self.site(node.unsafe_token.span, "unsafe block");
        self.require_safety_comment(node.unsafe_token.span);
        visit::visit_expr_unsafe(self, node);
    }

    fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
        if let Some(token) = &node.unsafety {
            self.site(token.span, "unsafe impl");
            self.require_safety_comment(token.span);
        }
        visit::visit_item_impl(self, node);
    }

    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        if let syn::Safety::Unsafe(token) = &node.sig.safety {
            self.site(token.span, "unsafe fn");
        }
        visit::visit_item_fn(self, node);
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        if let syn::Safety::Unsafe(token) = &node.sig.safety {
            self.site(token.span, "unsafe fn");
        }
        visit::visit_impl_item_fn(self, node);
    }

    fn visit_trait_item_fn(&mut self, node: &'ast syn::TraitItemFn) {
        if let syn::Safety::Unsafe(token) = &node.sig.safety {
            self.site(token.span, "unsafe fn");
        }
        visit::visit_trait_item_fn(self, node);
    }

    fn visit_item_trait(&mut self, node: &'ast syn::ItemTrait) {
        if let Some(token) = &node.unsafety {
            self.site(token.span, "unsafe trait");
        }
        visit::visit_item_trait(self, node);
    }

    fn visit_item_foreign_mod(&mut self, node: &'ast syn::ItemForeignMod) {
        self.site(node.abi.extern_token.span, "extern block");
        visit::visit_item_foreign_mod(self, node);
    }

    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        // A module's own attributes are judged here, where its name is known; everything inside goes through the
        // generic visitor.
        for attr in &node.attrs {
            if let Some(level) = unsafe_code_lint_level(attr).filter(|level| relaxes(level)) {
                let on_ffi_module = node.ident == "ffi" && level != "warn";
                self.lint_relaxations.push(LintRelaxation { line: attr.pound_token.spans[0].start().line, level, on_ffi_module });
            }
        }
        if let Some((_, items)) = &node.content {
            for item in items {
                self.visit_item(item);
            }
        }
    }

    fn visit_attribute(&mut self, attr: &'ast syn::Attribute) {
        if let Some(level) = unsafe_code_lint_level(attr).filter(|level| relaxes(level)) {
            self.lint_relaxations.push(LintRelaxation { line: attr.pound_token.spans[0].start().line, level, on_ffi_module: false });
        }
        visit::visit_attribute(self, attr);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(display: &str, is_root: bool, source: &str) -> SourceFile {
        let in_ffi_module = display.ends_with("/ffi.rs") || display.contains("/ffi/");
        SourceFile { display: display.to_owned(), in_ffi_module, is_root, source: source.to_owned() }
    }

    const SAFE_ROOT: &str = "//! Docs.\n#![forbid(unsafe_code)]\npub fn f() {}\n";

    #[test]
    fn a_safe_crate_passes() {
        assert!(audit_files("bodo-model", &[file("crates/bodo-model/src/lib.rs", true, SAFE_ROOT)]).is_empty());
    }

    #[test]
    fn roots_must_forbid_unsafe_code() {
        let findings = audit_files("bodo-model", &[file("crates/bodo-model/src/lib.rs", true, "pub fn f() {}\n")]);
        assert_eq!(findings, ["crates/bodo-model/src/lib.rs: crate root must declare `#![forbid(unsafe_code)]`"]);
        // Other files and `deny` don't count.
        let deny = "#![deny(unsafe_code)]\n";
        assert_eq!(audit_files("bodo-model", &[file("crates/bodo-model/src/lib.rs", true, deny)]).len(), 1);
        assert!(audit_files("bodo-model", &[file("crates/bodo-model/src/util.rs", false, "fn f() {}\n")]).is_empty());
    }

    #[test]
    fn an_ffi_crate_keeps_unsafe_code_in_its_ffi_module() {
        let root = "//! Docs.\n#[allow(unsafe_code)]\nmod ffi;\npub fn f() {}\n";
        let ffi = "pub fn now() -> u64 {\n    // SAFETY: the call has no preconditions.\n    unsafe { clock() }\n}\nunsafe extern \"C\" {\n    fn clock() -> u64;\n}\n";
        let files = [file("kit/bodo-kit-core/src/lib.rs", true, root), file("kit/bodo-kit-core/src/ffi.rs", false, ffi)];
        assert!(audit_files("bodo-kit-core", &files).is_empty(), "{:?}", audit_files("bodo-kit-core", &files));
        let nested = [file("kit/bodo-kit-core/src/lib.rs", true, root), file("kit/bodo-kit-core/src/ffi/mach.rs", false, ffi)];
        assert!(audit_files("bodo-kit-core", &nested).is_empty());
    }

    #[test]
    fn unsafe_code_outside_ffi_is_reported() {
        let source = "#![forbid(unsafe_code)]\nfn f() {\n    // SAFETY: fine.\n    unsafe { g() }\n}\nunsafe fn g() {}\n";
        assert_eq!(
            audit_files("bodo-model", &[file("crates/bodo-model/src/lib.rs", true, source)]),
            [
                "crates/bodo-model/src/lib.rs:4: unsafe block in bodo-model, which is not an FFI crate",
                "crates/bodo-model/src/lib.rs:6: unsafe fn in bodo-model, which is not an FFI crate",
            ]
        );
        let in_ffi_crate = audit_files("bodo-kit-core", &[file("kit/bodo-kit-core/src/pool.rs", false, source)]);
        assert_eq!(in_ffi_crate[0], "kit/bodo-kit-core/src/pool.rs:4: unsafe block outside an ffi module");
    }

    #[test]
    fn unsafe_blocks_and_impls_need_safety_comments() {
        let source = "fn f() {\n    // Calls g.\n    // SAFETY: g has no preconditions.\n    let x = unsafe { g() };\n    let y = unsafe { g() };\n    let z = unsafe { g() }; // SAFETY: same.\n}\n\n// SAFETY: Foo holds no pointers.\nunsafe impl Send for Foo {}\nunsafe impl Sync for Foo {}\n";
        let findings = audit_files("bodo-kit-core", &[file("kit/bodo-kit-core/src/ffi.rs", false, source)]);
        assert_eq!(
            findings,
            [
                "kit/bodo-kit-core/src/ffi.rs:5: unsafe code without a `// SAFETY:` comment",
                "kit/bodo-kit-core/src/ffi.rs:11: unsafe code without a `// SAFETY:` comment",
            ]
        );
    }

    #[test]
    fn unsafe_code_may_be_allowed_only_on_mod_ffi_in_ffi_crates() {
        let source = "#![allow(unsafe_code)]\n#[allow(unsafe_code)]\nmod sys;\n#[allow(dead_code, unsafe_code, reason = \"x\")]\nfn f() {}\n#[allow(unsafe_code)]\nmod ffi;\n";
        assert_eq!(
            audit_files("bodo-kit-core", &[file("kit/bodo-kit-core/src/lib.rs", true, source)]),
            [
                "kit/bodo-kit-core/src/lib.rs:1: `allow(unsafe_code)` is allowed only on `mod ffi` in an FFI crate",
                "kit/bodo-kit-core/src/lib.rs:2: `allow(unsafe_code)` is allowed only on `mod ffi` in an FFI crate",
                "kit/bodo-kit-core/src/lib.rs:4: `allow(unsafe_code)` is allowed only on `mod ffi` in an FFI crate",
            ]
        );
        let not_ffi_crate = audit_files("bodo-model", &[file("crates/bodo-model/src/lib.rs", true, "#[allow(unsafe_code)]\nmod ffi;\n")]);
        assert_eq!(
            not_ffi_crate,
            [
                "crates/bodo-model/src/lib.rs:1: `allow(unsafe_code)` is allowed only on `mod ffi` in an FFI crate",
                "crates/bodo-model/src/lib.rs: crate root must declare `#![forbid(unsafe_code)]`",
            ]
        );
    }

    #[test]
    fn safety_comments_must_sit_directly_above() {
        let lines = ["// SAFETY: ok", "", "unsafe {}"];
        assert!(!has_safety_comment(&lines, 3));
        assert!(has_safety_comment(&lines[..1], 1));
        assert!(!has_safety_comment(&lines, 0));
    }
}
