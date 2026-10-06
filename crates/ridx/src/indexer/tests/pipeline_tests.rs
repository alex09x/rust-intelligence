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
use crate::indexer::driver::*;
use crate::indexer::orchestrate::*;
use crate::indexer::types::*;
use std::fs;

#[test]
fn test_run_indexing_on_sample_crate() {
    let _lock = lock_indexer_test();
    let test_dir = new_test_dir("ridx_run_index_test");
    let cargo_toml = test_dir.0.join("Cargo.toml");
    fs::write(
        &cargo_toml,
        "[package]\nname = \"sample_crate\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    let src_dir = test_dir.0.join("src");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(
        src_dir.join("lib.rs"),
        "pub fn add(a: i32, b: i32) -> i32 { a + b }\n",
    )
    .unwrap();

    if find_driver_binary().is_some() {
        let out = run_indexing(IndexOptions {
            path: test_dir.0.clone(),
            out_dir_opt: None,
            package: None,
            release: false,
            features: None,
            no_default_features: false,
            all_features: false,
            target: None,
        });
        assert!(out.variant_out_dir.exists());

        // Also test passing manifest file directly
        let out_file = run_indexing(IndexOptions {
            path: cargo_toml,
            out_dir_opt: Some(test_dir.0.join("custom_out")),
            package: None,
            release: false,
            features: None,
            no_default_features: false,
            all_features: false,
            target: None,
        });
        assert!(out_file.variant_out_dir.exists());
    }
}

#[test]
fn test_try_run_indexing_error_cases() {
    let test_dir = new_test_dir("ridx_err_cases");
    let non_existent = test_dir.0.join("non_existent");
    let res_missing = try_run_indexing(IndexOptions {
        path: non_existent,
        out_dir_opt: None,
        package: None,
        release: false,
        features: None,
        no_default_features: false,
        all_features: false,
        target: None,
    });
    assert!(res_missing.is_err());

    let not_cargo = test_dir.0.join("not_cargo.txt");
    fs::write(&not_cargo, "hello").unwrap();
    let res_not_cargo = try_run_indexing(IndexOptions {
        path: not_cargo,
        out_dir_opt: None,
        package: None,
        release: false,
        features: None,
        no_default_features: false,
        all_features: false,
        target: None,
    });
    assert!(res_not_cargo.is_err());

    let empty_dir = test_dir.0.join("empty_dir");
    fs::create_dir_all(&empty_dir).unwrap();
    let res_empty = try_run_indexing(IndexOptions {
        path: empty_dir,
        out_dir_opt: None,
        package: None,
        release: false,
        features: None,
        no_default_features: false,
        all_features: false,
        target: None,
    });
    assert!(res_empty.is_err());
}

#[test]
fn test_try_run_indexing_with_options_and_deps() {
    let _lock = lock_indexer_test();
    let test_dir = new_test_dir("ridx_opts_test");
    let cargo_toml = test_dir.0.join("Cargo.toml");
    fs::write(
            &cargo_toml,
            "[package]\nname = \"opt_crate\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[features]\nfeat_a = []\n",
        )
        .unwrap();
    let src_dir = test_dir.0.join("src");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(
        src_dir.join("lib.rs"),
        "#[cfg(feature = \"feat_a\")]\npub fn feat() {}\n",
    )
    .unwrap();

    if find_driver_binary().is_some() {
        let res = try_run_indexing(IndexOptions {
            path: test_dir.0.clone(),
            out_dir_opt: None,
            package: Some("opt_crate".to_string()),
            release: false,
            features: Some("feat_a".to_string()),
            no_default_features: true,
            all_features: true,
            target: None,
        });
        assert!(res.is_ok());

        // Create mock .deps files in variant_out_dir to test external dependency shard parsing and validation
        let pv = res.unwrap();
        let deps_dir = pv.variant_out_dir.join(".deps");
        fs::create_dir_all(&deps_dir).unwrap();

        let dep_ridx = test_dir.0.join("external_dep.ridx");
        let mut writer = ridx_format::IndexWriter::new("external_dep", "1.0.0");
        writer.set_unit_key("ext_key");
        writer.set_generation("gen123");
        writer.add_file_hash("src/lib.rs", 12345);
        let mut f = fs::File::create(&dep_ridx).unwrap();
        writer.write_to(&mut f).unwrap();
        drop(f);

        let cargo_cfg_dir = test_dir.0.join(".cargo");
        fs::create_dir_all(&cargo_cfg_dir).unwrap();
        fs::write(
            cargo_cfg_dir.join("config.toml"),
            "[build]\ntarget = \"x86_64-unknown-linux-gnu\"\n",
        )
        .unwrap();

        let mock_shard_info = serde_json::json!({
            "name": "external_dep",
            "version": "1.0.0",
            "unit_key": "ext_key",
            "object_path": dep_ridx.display().to_string(),
            "source_root": test_dir.0.display().to_string(),
            "is_dependency": true,
            "generation": "gen123",
            "file_hashes": { "src/lib.rs": 12345 },
            "dependencies": ["dep_b"],
            "extern_units": { "dep_b": "unit_b" }
        });
        fs::write(
            deps_dir.join("external_dep.json"),
            mock_shard_info.to_string(),
        )
        .unwrap();

        let mismatch_shard_info = serde_json::json!({
            "name": "mismatch_dep",
            "version": "1.0.0",
            "unit_key": "ext_key2",
            "object_path": dep_ridx.display().to_string(),
            "source_root": null,
            "is_dependency": true,
            "generation": "wrong_gen",
            "file_hashes": { "src/lib.rs": 99999 },
            "dependencies": ["dep_c"],
            "extern_units": { "dep_c": "unit_c" }
        });
        fs::write(
            deps_dir.join("mismatch_dep.json"),
            mismatch_shard_info.to_string(),
        )
        .unwrap();

        let hash_gen_shard_info = serde_json::json!({
            "name": "hash_gen_dep",
            "version": "1.0.0",
            "unit_key": "ext_key3",
            "object_path": dep_ridx.display().to_string(),
            "source_root": null,
            "is_dependency": true,
            "generation": null,
            "file_hashes": { "src/lib.rs": 12345 },
            "dependencies": [],
            "extern_units": {}
        });
        fs::write(
            deps_dir.join("hash_gen_dep.json"),
            hash_gen_shard_info.to_string(),
        )
        .unwrap();

        // Run again with matching options to cover dependency shard collection and validation logic
        let res2 = try_run_indexing(IndexOptions {
            path: test_dir.0.clone(),
            out_dir_opt: Some(pv.base_out_dir),
            package: Some("opt_crate".to_string()),
            release: false,
            features: Some("feat_a".to_string()),
            no_default_features: true,
            all_features: true,
            target: None,
        });
        assert!(res2.is_ok());
    }
}

#[test]
fn test_try_run_indexing_syntax_error() {
    let _lock = lock_indexer_test();
    let test_dir = new_test_dir("ridx_syntax_err_test");
    let cargo_toml = test_dir.0.join("Cargo.toml");
    fs::write(
        &cargo_toml,
        "[package]\nname = \"syntax_err_crate\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    let src_dir = test_dir.0.join("src");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(
        src_dir.join("lib.rs"),
        "pub fn broken() { invalid syntax here {{{ \n",
    )
    .unwrap();

    if find_driver_binary().is_some() {
        let res = try_run_indexing(IndexOptions {
            path: test_dir.0.clone(),
            out_dir_opt: None,
            package: None,
            release: false,
            features: None,
            no_default_features: false,
            all_features: false,
            target: None,
        });
        assert!(res.is_err());
        let err_msg = res.unwrap_err();
        assert!(
            err_msg.contains("cargo check exited with status")
                || err_msg.contains("failed to open generated variant index")
        );
    }
}

#[test]
fn test_cargo_config_and_manifest_discovery() {
    let test_dir = new_test_dir("ridx_cargo_disc_test");
    let home_dir = test_dir.0.join("home");
    let home_cargo = home_dir.join(".cargo");
    fs::create_dir_all(&home_cargo).unwrap();
    fs::write(
        home_cargo.join("config"),
        "[build]\ntarget = \"thumbv7m-none-eabi\"\n",
    )
    .unwrap();
    std::env::set_var("CARGO_HOME", &home_cargo);

    let configs = find_cargo_configs(&test_dir.0);
    assert!(!configs.is_empty());

    let mut manifests = Vec::new();
    let sub = test_dir.0.join("sub");
    fs::create_dir_all(&sub).unwrap();
    fs::write(sub.join("Cargo.toml"), "[package]\nname = \"sub\"\n").unwrap();
    collect_all_workspace_manifests(&test_dir.0, &mut manifests);
    assert_eq!(manifests.len(), 1);

    // Empty target value in config returns None (lines 80-82 in cargo.rs)
    assert_eq!(extract_config_target("[build]\ntarget = \"\"\n"), None);
}
