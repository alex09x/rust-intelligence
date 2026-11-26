/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! A dependency that does not compile no longer leaves its dependents unindexed. serde's `test_suite` at
//! 69072f24: its dev-dependency `trybuild` failed against the registry `serde_core`, cargo built none of the
//! test targets, and every file of `test_suite/tests` had no index although only `compiletest.rs` uses trybuild.

use std::fs;
use std::path::Path;

use ridx::indexer::{stand_ins, try_run_indexing, IndexOptions};

use crate::l2_harness::{ensure_fresh_driver, repo_root};

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn shards(variants: &Path) -> Vec<String> {
    let mut names = Vec::new();
    for variant in fs::read_dir(variants).into_iter().flatten().flatten() {
        for f in fs::read_dir(variant.path()).into_iter().flatten().flatten() {
            names.push(f.file_name().to_string_lossy().into_owned());
        }
    }
    names
}

#[test]
fn test_targets_of_a_dependency_that_does_not_compile_are_indexed() {
    let root = std::env::temp_dir().join(format!("ridx_failed_dependency_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    // A path dependency unpacked where Cargo keeps registry sources is a registry dependency to the driver.
    let broken = root.join(".cargo/registry/src/index.example-0/broken-1.0.0");
    write(&broken.join("Cargo.toml"), "[package]\nname = \"broken\"\nversion = \"1.0.0\"\nedition = \"2021\"\n");
    write(&broken.join("src/lib.rs"), "pub fn check() -> u8 { \"not a number\" }\n");
    let ws = root.join("app");
    write(
        &ws.join("Cargo.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dev-dependencies]\nbroken = { path = \"../.cargo/registry/src/index.example-0/broken-1.0.0\" }\n\n[workspace]\n",
    );
    write(&ws.join("src/lib.rs"), "pub fn helper() -> u32 {\n    1\n}\n");
    write(&ws.join("tests/uses_app.rs"), "#[test]\nfn uses_helper() {\n    assert_eq!(app::helper(), 1);\n}\n");
    write(&ws.join("tests/uses_broken.rs"), "#[test]\nfn uses_check() {\n    assert_eq!(broken::check(), app::helper() as u8);\n}\n");

    let opts = IndexOptions {
        path: ws.clone(),
        out_dir_opt: None,
        package: None,
        release: false,
        features: None,
        no_default_features: false,
        all_features: false,
        target: None,
        cache_dir: Some(repo_root().join("target/ridx-test-cache")),
        driver_bin: Some(ensure_fresh_driver()),
        capture_child_output: true,
        builder: None,
    };
    try_run_indexing(opts).unwrap_or_else(|e| panic!("indexing a workspace whose dev-dependency does not compile: {e}"));

    let names = shards(&ws.join("target/ridx/variants"));
    assert!(names.iter().any(|n| n.starts_with("uses_app")), "the test target that does not use the broken crate: {names:?}");
    // Its use of the broken crate is unresolved, the rest of it is indexed like any crate with errors.
    assert!(names.iter().any(|n| n.starts_with("uses_broken")), "the test target that uses it: {names:?}");
    assert!(!names.iter().any(|n| n.starts_with("broken")), "no index of the crate that did not compile: {names:?}");
    assert_eq!(stand_ins::active(&ws.join("target/ridx_cargo")), vec!["broken".to_string()]);
    let _ = fs::remove_dir_all(&root);
}
