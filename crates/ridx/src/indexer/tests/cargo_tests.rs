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
use crate::indexer::cargo::*;
use std::fs;

#[test]
fn test_extract_config_target() {
    let content1 = r#"
            # Some comments
            [build]
            target = "x86_64-unknown-linux-gnu"
        "#;
    assert_eq!(
        extract_config_target(content1),
        Some("x86_64-unknown-linux-gnu".to_string())
    );

    let content_single_quote = r#"
            [build]
            target = 'aarch64-apple-darwin'
        "#;
    assert_eq!(
        extract_config_target(content_single_quote),
        Some("aarch64-apple-darwin".to_string())
    );

    let content_with_flags = r#"
            [build]
            rustflags = ["-C", "target-cpu=native"]
            target = "x86_64-unknown-linux-gnu"
        "#;
    assert_eq!(
        extract_config_target(content_with_flags),
        Some("x86_64-unknown-linux-gnu".to_string())
    );

    let content_other_section = r#"
            [target.x86_64-unknown-linux-gnu]
            rustflags = ["-C", "opt-level=3"]
        "#;
    assert_eq!(extract_config_target(content_other_section), None);

    let content_empty = "";
    assert_eq!(extract_config_target(content_empty), None);
}

#[test]
fn test_resolve_cargo_home() {
    let _lock = lock_indexer_test();
    let home = resolve_cargo_home();
    assert!(home.is_some());
}

#[test]
fn test_find_cargo_configs() {
    let test_dir = new_test_dir("ridx_cfg_test");
    let cargo_dir = test_dir.0.join(".cargo");
    fs::create_dir_all(&cargo_dir).unwrap();
    let cfg_file = cargo_dir.join("config.toml");
    fs::write(&cfg_file, "[build]\ntarget = \"x86_64-linux-android\"\n").unwrap();

    let configs = find_cargo_configs(&test_dir.0);
    assert!(configs.contains(&cfg_file));
}

#[test]
fn test_collect_all_workspace_manifests() {
    let test_dir = new_test_dir("ridx_manifest_scan");
    let root_cargo = test_dir.0.join("Cargo.toml");
    fs::write(&root_cargo, "[workspace]\nmembers = [\"crate_a\"]\n").unwrap();

    let sub_crate = test_dir.0.join("crate_a");
    fs::create_dir_all(&sub_crate).unwrap();
    let sub_cargo = sub_crate.join("Cargo.toml");
    fs::write(
        &sub_cargo,
        "[package]\nname = \"crate_a\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();

    let target_dir = test_dir.0.join("target/sub");
    fs::create_dir_all(&target_dir).unwrap();
    let target_cargo = target_dir.join("Cargo.toml");
    fs::write(&target_cargo, "invalid").unwrap();

    let mut manifests = Vec::new();
    collect_all_workspace_manifests(&test_dir.0, &mut manifests);

    assert!(manifests.contains(&root_cargo));
    assert!(manifests.contains(&sub_cargo));
    assert!(!manifests.contains(&target_cargo));
}
