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
use crate::indexer::orchestrate::*;
use crate::indexer::types::*;
use crate::indexer::variant::*;
use std::fs;

#[test]
fn test_indexer_additional_edge_cases() {
    let _lock = lock_indexer_test();

    let temp = new_test_dir("indexer_add_edges");
    let manifest_file = temp.0.join("Cargo.toml");
    fs::write(&manifest_file, "[package]\nname = \"add_edges\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();

    // 1. collect_all_workspace_manifests with subdirectory
    let sub = temp.0.join("member_pkg");
    fs::create_dir_all(&sub).unwrap();
    fs::write(sub.join("Cargo.toml"), "[package]\nname = \"sub\"\nversion = \"0.1.0\"\n").unwrap();
    let mut manifests = Vec::new();
    collect_all_workspace_manifests(&temp.0, &mut manifests);
    assert_eq!(manifests.len(), 2);

    // 2. compute_variant_id with non-standard manifest path
    let custom_manifest = temp.0.join("Custom.toml");
    fs::write(&custom_manifest, "[package]\nname = \"cust\"\n").unwrap();
    let var_id = compute_variant_id("dev", None, None, None, false, false, &custom_manifest);
    assert!(!var_id.is_empty());

    // 3. run_indexing with base_out_dir having a file named "variants" (triggers variant directory creation failure)
    let dummy_driver = temp.0.join("dummy_driver");
    fs::write(&dummy_driver, b"dummy").unwrap();

    let out_dir_with_variants_file = temp.0.join("out_dir_var_err");
    fs::create_dir_all(&out_dir_with_variants_file).unwrap();
    fs::write(out_dir_with_variants_file.join("variants"), b"blocking file").unwrap();
    let err_mkdir = try_run_indexing(IndexOptions {
        path: manifest_file.clone(),
        out_dir_opt: Some(out_dir_with_variants_file),
        package: None,
        release: false,
        features: None,
        no_default_features: false,
        all_features: false,
        target: None,
        cache_dir: None,
        driver_bin: Some(dummy_driver),
        capture_child_output: false,
    });
    assert!(err_mkdir.unwrap_err().contains("failed to create variant directory"));

    // 4. run_indexing when driver binary is not found
    let err_no_driver = try_run_indexing(IndexOptions {
        path: manifest_file,
        out_dir_opt: None,
        package: None,
        release: false,
        features: None,
        no_default_features: false,
        all_features: false,
        target: None,
        cache_dir: None,
        driver_bin: Some(PathBuf::from("/nonexistent/ridx-driver-bogus")),
        capture_child_output: false,
    });
    assert!(err_no_driver.unwrap_err().contains("could not find ridx-driver binary"));
}
