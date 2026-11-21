/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::*;

/// Review of 6914b9cf: a target Cargo declares under its build directory (`[[test]] path =
/// "build-cache/cases/caller.rs"` with `build-cache` the target directory) is source, the rest of that directory not.
#[test]
fn test_a_declared_target_under_the_build_directory_is_searched() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join("build-cache/cases")).unwrap();
    std::fs::create_dir_all(root.join("build-cache/debug")).unwrap();
    let manifest = "[package]\nname = \"w\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[[test]]\nname = \"caller\"\npath = \"build-cache/cases/caller.rs\"\n";
    std::fs::write(root.join("Cargo.toml"), manifest).unwrap();
    std::fs::write(root.join("src/lib.rs"), "pub fn unused() {}\n").unwrap();
    std::fs::write(root.join("build-cache/cases/caller.rs"), "#[test]\nfn t() { w::unused() }\n").unwrap();
    std::fs::write(root.join("build-cache/debug/artifact.rs"), "// unused\n").unwrap();
    // As `inventory` makes it of Cargo's answer with `CARGO_TARGET_DIR=build-cache` (the environment of a test run
    // may set its own).
    let cases = root.join("build-cache/cases");
    let inv = Inventory {
        roots: vec![root.clone(), cases.clone()],
        skip: vec![root.join("build-cache"), root.join("target/ridx")],
        members: [root.clone()].into_iter().collect(),
        named: [root.clone(), cases.clone()].into_iter().collect(),
        declared: vec![root.join("src"), cases],
    };
    let files = rust_files(&inv).unwrap();
    assert!(files.iter().any(|f| f.ends_with("build-cache/cases/caller.rs")), "{files:?}");
    assert!(!files.iter().any(|f| f.ends_with("artifact.rs")), "{files:?}");
}
