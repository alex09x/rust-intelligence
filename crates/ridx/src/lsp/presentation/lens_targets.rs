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
