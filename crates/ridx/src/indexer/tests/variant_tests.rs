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
use crate::indexer::driver::*;
use crate::indexer::types::*;
use crate::indexer::variant::*;
use std::fs;
use std::path::PathBuf;

#[test]
fn test_compute_variant_id_permutations() {
    let _lock = lock_indexer_test();
    let test_dir = new_test_dir("ridx_variant_test");
    let root_cargo = test_dir.0.join("Cargo.toml");
    fs::write(&root_cargo, "[package]\nname = \"foo\"\nversion = \"0.1.0\"\n").unwrap();

    let v1 = compute_variant_id("dev", None, None, None, false, false, &root_cargo);
    let v2 = compute_variant_id("release", None, None, None, false, false, &root_cargo);
    assert_ne!(v1, v2);

    let v3 = compute_variant_id("dev", Some("x86_64"), None, None, false, false, &root_cargo);
    assert_ne!(v1, v3);

    let v4 = compute_variant_id("dev", None, Some("foo"), None, false, false, &root_cargo);
    assert_ne!(v1, v4);

    let v5 = compute_variant_id("dev", None, None, Some("feat_a"), false, false, &root_cargo);
    assert_ne!(v1, v5);

    let v6 = compute_variant_id("dev", None, None, None, true, false, &root_cargo);
    assert_ne!(v1, v6);

    let v7 = compute_variant_id("dev", None, None, None, false, true, &root_cargo);
    assert_ne!(v1, v7);

    // Idempotency: same inputs give identical variant id
    let v1_again = compute_variant_id("dev", None, None, None, false, false, &root_cargo);
    assert_eq!(v1, v1_again);
}

#[test]
fn test_find_driver_binary() {
    let _ = find_driver_binary();
}

#[test]
fn test_compute_variant_id_with_lock_and_env() {
    let _lock = lock_indexer_test();
    let test_dir = new_test_dir("ridx_variant_lock");
    let root_cargo = test_dir.0.join("Cargo.toml");
    fs::write(&root_cargo, "[package]\nname = \"foo\"\nversion = \"0.1.0\"\n").unwrap();
    let root_lock = test_dir.0.join("Cargo.lock");
    fs::write(&root_lock, "# Lock file content\nversion = 3\n").unwrap();

    let v = compute_variant_id("dev", Some("wasm32"), None, None, false, false, &root_cargo);
    assert!(!v.is_empty());
}

#[test]
fn test_published_variant_and_options() {
    let opts = IndexOptions {
        path: PathBuf::from("."),
        out_dir_opt: None,
        package: Some("test_pkg".to_string()),
        release: true,
        features: Some("feat".to_string()),
        no_default_features: true,
        all_features: false,
        target: Some("x86_64".to_string()),
        cache_dir: None,
        driver_bin: None,
        capture_child_output: false,
        builder: None,
    };
    assert_eq!(opts.package.as_deref(), Some("test_pkg"));

    let pv = PublishedVariant { base_out_dir: PathBuf::from("/target/ridx"), variant_out_dir: PathBuf::from("/target/ridx/variants/dev-123"), variant_id: "dev-123".to_string() };
    let debug_str = format!("{pv:?}");
    assert!(debug_str.contains("dev-123"));
}
