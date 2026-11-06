/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The Cargo package and target a source file is compiled into, by Cargo's target auto-discovery layout: what a
//! runnable lens needs to build its `cargo` command line.

use std::path::{Path, PathBuf};

/// The Cargo target a file is compiled into.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum TargetKind {
    Lib,
    Bin(String),
    Test(String),
    Bench(String),
    Example(String),
}

pub(super) struct FileTarget {
    pub package: String,
    pub pkg_dir: PathBuf,
    pub workspace_root: PathBuf,
    pub kind: TargetKind,
    pub is_root: bool,
    pub module_prefix: Vec<String>,
    /// The path follows Cargo's target layout, so `#[test]` and docs read off the text belong to a compiled target.
    pub layout_known: bool,
}

fn package_name(manifest: &str) -> Option<String> {
    let mut in_package = false;
    for line in manifest.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_package = t == "[package]";
            continue;
        }
        if in_package {
            if let Some(rest) = t.strip_prefix("name") {
                let value = rest.trim_start().strip_prefix('=')?.trim().trim_matches('"');
                return (!value.is_empty()).then(|| value.to_string());
            }
        }
    }
    None
}

/// The target a `[[bin]]`, `[[example]]`, `[[test]]` or `[[bench]]` section of `manifest` declares with `path` equal
/// to `rel` (relative to the package, `/`-separated).
fn declared_target(manifest: &str, rel: &str) -> Option<TargetKind> {
    let mut section = "";
    let (mut name, mut path): (Option<String>, Option<String>) = (None, None);
    let finish = |section: &str, name: &Option<String>, path: &Option<String>| -> Option<TargetKind> {
        let (n, p) = (name.clone()?, path.as_deref()?);
        if p.trim_start_matches("./") != rel {
            return None;
        }
        match section {
            "[[bin]]" => Some(TargetKind::Bin(n)),
            "[[example]]" => Some(TargetKind::Example(n)),
            "[[test]]" => Some(TargetKind::Test(n)),
            "[[bench]]" => Some(TargetKind::Bench(n)),
            _ => None,
        }
    };
    for line in manifest.lines().chain(std::iter::once("[end]")) {
        let t = line.trim();
        if t.starts_with('[') {
            if let Some(k) = finish(section, &name, &path) {
                return Some(k);
            }
            section = if matches!(t, "[[bin]]" | "[[example]]" | "[[test]]" | "[[bench]]") { t } else { "" };
            (name, path) = (None, None);
            continue;
        }
        let value = |key: &str| t.strip_prefix(key).map(str::trim_start).and_then(|r| r.strip_prefix('=')).map(|v| v.trim().trim_matches('"').to_string());
        if let Some(v) = value("name") {
            name = Some(v);
        } else if let Some(v) = value("path") {
            path = Some(v);
        }
    }
    None
}

pub(super) fn file_target(file: &Path) -> Option<FileTarget> {
    let (pkg_dir, package) = file.ancestors().skip(1).find_map(|dir| {
        let manifest = std::fs::read_to_string(dir.join("Cargo.toml")).ok()?;
        package_name(&manifest).map(|name| (dir.to_path_buf(), name))
    })?;
    let workspace_root = pkg_dir
        .ancestors()
        .find(|dir| std::fs::read_to_string(dir.join("Cargo.toml")).is_ok_and(|m| m.lines().any(|l| l.trim() == "[workspace]")))
        .unwrap_or(&pkg_dir)
        .to_path_buf();
    let rel: Vec<String> = file.strip_prefix(&pkg_dir).ok()?.iter().map(|c| c.to_string_lossy().into_owned()).collect();
    // A target the manifest declares with this file as its `path` (`examples/tutorial_derive/04_02_parse.rs`).
    if let Some(kind) = std::fs::read_to_string(pkg_dir.join("Cargo.toml")).ok().and_then(|m| declared_target(&m, &rel.join("/"))) {
        return Some(FileTarget { package, pkg_dir, workspace_root, kind, is_root: true, module_prefix: Vec::new(), layout_known: true });
    }
    let stem = |s: &str| s.strip_suffix(".rs").unwrap_or(s).to_string();
    let (kind, root_len) = match rel.as_slice() {
        [src, main] if src == "src" && main == "main.rs" => (TargetKind::Bin(package.clone()), 2),
        [src, bin, rest @ ..] if src == "src" && bin == "bin" && !rest.is_empty() => (TargetKind::Bin(stem(&rest[0])), 3),
        [src, ..] if src == "src" => (TargetKind::Lib, 1),
        [dir, rest @ ..] if !rest.is_empty() && matches!(dir.as_str(), "tests" | "benches" | "examples") => {
            let name = stem(&rest[0]);
            let kind = match dir.as_str() {
                "tests" => TargetKind::Test(name),
                "benches" => TargetKind::Bench(name),
                _ => TargetKind::Example(name),
            };
            (kind, 2)
        }
        // Outside Cargo's layout a file reaches a target only through `#[path]`, or it is test data never compiled;
        // only the index can tell, so such files get the runnables it reports and nothing read off the text.
        _ => (TargetKind::Lib, rel.len()),
    };
    let layout_known = rel.first().is_some_and(|d| d == "src") || (rel.len() >= 2 && matches!(rel[0].as_str(), "tests" | "benches" | "examples"));
    // Module path inside the target: `src/a/b.rs` is `a::b`, `src/a/mod.rs` is `a`, a target root is the crate root.
    let mut module_prefix: Vec<String> = rel[root_len.min(rel.len())..].iter().map(|s| stem(s)).collect();
    if module_prefix.last().is_some_and(|l| l == "mod") {
        module_prefix.pop();
    }
    let is_root = match &kind {
        TargetKind::Lib => rel.len() == 2 && rel[1] == "lib.rs",
        TargetKind::Bin(_) if rel.len() == 2 => true,
        _ => rel.len() == root_len || (rel.len() == root_len + 1 && rel[root_len] == "main.rs"),
    };
    if is_root || module_prefix.first().is_some_and(|f| f == "main" || f == "lib") {
        module_prefix.clear();
    }
    Some(FileTarget { package, pkg_dir, workspace_root, kind, is_root, module_prefix, layout_known })
}

#[cfg(test)]
mod declared_target_tests {
    use super::*;

    #[test]
    fn test_declared_target_by_path() {
        let m = "[package]\nname = \"clap\"\n\n[[example]]\nname = \"04_02_parse_derive\"\npath = \"examples/tutorial_derive/04_02_parse.rs\"\nrequired-features = [\"derive\"]\n\n[[bin]]\nname = \"tool\"\npath = \"./src/tool.rs\"\n";
        assert!(matches!(declared_target(m, "examples/tutorial_derive/04_02_parse.rs"), Some(TargetKind::Example(n)) if n == "04_02_parse_derive"));
        assert!(matches!(declared_target(m, "src/tool.rs"), Some(TargetKind::Bin(n)) if n == "tool"));
        assert!(declared_target(m, "src/lib.rs").is_none());
    }
}
