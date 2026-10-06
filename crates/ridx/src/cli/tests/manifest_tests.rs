/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::common::*;
use crate::cli::args::Commands;
use crate::cli::dispatch::run_cli;
use crate::cli::resolve::*;
use crate::indexer::find_driver_binary;
use std::fs;
use std::path::PathBuf;

#[test]
fn test_manifest_discovery_and_auto_reindexing() {
    let _lock = lock_cli_test();
    std::env::remove_var("RIDX_INDEX_DIR");
    let (test_dir, dir_str) = create_indexed_workspace();
    let deep_file = test_dir.0.join("a/b/c/nested_file.rs");
    assert!(find_manifest_for_path(&deep_file).is_none());

    let sub_dir = test_dir.0.join("sub_package");
    fs::create_dir_all(&sub_dir).unwrap();

    // 1. Shard-specific variant_manifest.json in candidate directory (lines 212-221)
    let v_manifest = serde_json::json!({
        "version": "2.0",
        "format_version": ridx_format::CURRENT_FORMAT_VERSION,
        "extractor_version": ridx_format::EXTRACTOR_VERSION,
        "active_variant_id": "v_shard",
        "variant_dir": "variants/v_shard",
        "workspace_root": test_dir.0.display().to_string(),
        "shards": []
    });
    fs::write(
        sub_dir.join("variant_manifest.json"),
        serde_json::to_string(&v_manifest).unwrap(),
    )
    .unwrap();

    let found_vm = find_manifest_for_path(&sub_dir.join("shard.ridx"));
    assert!(found_vm.is_some());
    assert_eq!(found_vm.unwrap().1.active_variant_id, "v_shard");

    // 2. Active variant match in ridx_manifest.json (lines 235-248)
    let root_manifest = test_dir.0.join("ridx_manifest.json");
    let r_manifest = serde_json::json!({
        "version": "2.0",
        "format_version": ridx_format::CURRENT_FORMAT_VERSION,
        "extractor_version": ridx_format::EXTRACTOR_VERSION,
        "active_variant_id": "active_var",
        "variant_dir": "variants/active_var",
        "workspace_root": test_dir.0.display().to_string(),
        "shards": []
    });
    fs::write(&root_manifest, serde_json::to_string(&r_manifest).unwrap()).unwrap();

    let active_shard = test_dir.0.join("variants/active_var/shard.ridx");
    let found_rm = find_manifest_for_path(&active_shard);
    assert!(found_rm.is_some());

    let inactive_shard = test_dir.0.join("variants/inactive_var/shard.ridx");
    assert!(find_manifest_for_path(&inactive_shard).is_none());
    let _ = fs::remove_file(&root_manifest);

    // 3. CLI commands execution: TypeAt, Hover, Compare
    run_cli(Commands::TypeAt {
        arg1: dir_str.clone(),
        arg2: Some("src/lib.rs:1:1".to_string()),
        json: false,
    });
    run_cli(Commands::TypeAt {
        arg1: dir_str.clone(),
        arg2: Some("src/lib.rs:1:1".to_string()),
        json: true,
    });

    run_cli(Commands::Hover {
        arg1: dir_str.clone(),
        arg2: Some("src/lib.rs:1:1".to_string()),
        json: false,
    });
    run_cli(Commands::Hover {
        arg1: dir_str.clone(),
        arg2: Some("src/lib.rs:1:1".to_string()),
        json: true,
    });

    run_cli(Commands::Compare {
        arg1: dir_str.clone(),
        arg2: Some("test_crate::test_fn".to_string()),
        symbol: None,
        json: false,
    });
    run_cli(Commands::Compare {
        arg1: dir_str.clone(),
        arg2: Some("test_crate::test_fn".to_string()),
        symbol: None,
        json: true,
    });

    // 4. try_resolve_workspace variant_manifest error cases (lines 281-286, 289-305)
    let test_bad_ws = new_test_dir("bad_ws");
    let bad_manifest = serde_json::json!({
        "version": "2.0",
        "format_version": ridx_format::CURRENT_FORMAT_VERSION,
        "extractor_version": ridx_format::EXTRACTOR_VERSION,
        "active_variant_id": "bad",
        "variant_dir": "variants/bad",
        "workspace_root": "/nonexistent/workspace/root",
        "shards": []
    });
    fs::write(
        test_bad_ws.0.join("variant_manifest.json"),
        serde_json::to_string(&bad_manifest).unwrap(),
    )
    .unwrap();
    let res_bad = try_resolve_workspace(Some(test_bad_ws.0.join("shard.ridx")));
    assert!(res_bad
        .unwrap_err()
        .contains("Cannot find Cargo workspace for index"));

    // try_resolve_workspace where manifest.workspace_root is empty and find_enclosing_workspace succeeds (line 280)
    let test_empty_root_ws = new_test_dir("empty_root_ws");
    fs::write(
            test_empty_root_ws.0.join("Cargo.toml"),
            "[package]\nname = \"empty_pkg\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[lib]\npath = \"src/lib.rs\"\n",
        )
        .unwrap();
    let empty_src = test_empty_root_ws.0.join("src");
    fs::create_dir_all(&empty_src).unwrap();
    fs::write(empty_src.join("lib.rs"), "pub fn empty_fn() {}\n").unwrap();
    let target_ridx = test_empty_root_ws.0.join("target/ridx");
    fs::create_dir_all(&target_ridx).unwrap();
    let empty_root_manifest = serde_json::json!({
        "version": "2.0",
        "format_version": ridx_format::CURRENT_FORMAT_VERSION,
        "extractor_version": ridx_format::EXTRACTOR_VERSION,
        "active_variant_id": "v_empty",
        "variant_dir": "variants/v_empty",
        "workspace_root": "",
        "shards": []
    });
    fs::write(
        target_ridx.join("variant_manifest.json"),
        serde_json::to_string(&empty_root_manifest).unwrap(),
    )
    .unwrap();
    let _ = try_resolve_workspace(Some(target_ridx.join("shard.ridx")));

    // Base out dir resolution from parent variants dir (lines 289-305)
    let var_dir = test_bad_ws.0.join("target/ridx/variants/v1");
    fs::create_dir_all(&var_dir).unwrap();
    let v1_manifest = serde_json::json!({
        "version": "2.0",
        "format_version": ridx_format::CURRENT_FORMAT_VERSION,
        "extractor_version": ridx_format::EXTRACTOR_VERSION,
        "active_variant_id": "v1",
        "variant_dir": "variants/v1",
        "workspace_root": "/nonexistent/workspace/root",
        "shards": []
    });
    fs::write(
        var_dir.join("variant_manifest.json"),
        serde_json::to_string(&v1_manifest).unwrap(),
    )
    .unwrap();
    let res_v1 = try_resolve_workspace(Some(var_dir.join("shard.ridx")));
    assert!(res_v1.is_err());

    let variants_dir = test_bad_ws.0.join("target2/ridx/variants");
    fs::create_dir_all(&variants_dir).unwrap();
    fs::write(
        variants_dir.join("variant_manifest.json"),
        serde_json::to_string(&v1_manifest).unwrap(),
    )
    .unwrap();
    let res_v2 = try_resolve_workspace(Some(variants_dir.join("shard.ridx")));
    assert!(res_v2.is_err());

    // 5. try_resolve_workspace(None) auto-indexing trigger (lines 386-445)
    let test_auto = new_test_dir("auto_idx");
    let empty_dir = test_auto.0.join("empty");
    fs::create_dir_all(&empty_dir).unwrap();
    let orig_cur = std::env::current_dir().unwrap();
    std::env::remove_var("RIDX_INDEX_DIR");
    std::env::set_current_dir(&empty_dir).unwrap();
    let res_empty = try_resolve_workspace(None);
    assert!(res_empty.is_err());
    std::env::set_current_dir(&orig_cur).unwrap();

    if find_driver_binary().is_some() {
        let ws_auto = test_auto.0.join("ws");
        let src_auto = ws_auto.join("src");
        fs::create_dir_all(&src_auto).unwrap();
        fs::write(
                ws_auto.join("Cargo.toml"),
                "[package]\nname = \"auto_pkg\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[lib]\npath = \"src/lib.rs\"\n",
            )
            .unwrap();
        fs::write(src_auto.join("lib.rs"), "pub fn auto_fn() {}\n").unwrap();

        std::env::set_var("RIDX_NO_DEPS", "1");
        std::env::set_current_dir(&ws_auto).unwrap();
        let res_auto_build = try_resolve_workspace(None);
        assert!(res_auto_build.is_ok());

        // Now test find_manifest_for_path returning valid manifest with workspace_root (lines 398-419)
        // In ws_auto, target/ridx/ridx_manifest.json exists now from auto_indexing
        let out_dir = ws_auto.join("target/ridx");
        let mut v_manifest = serde_json::json!({
            "version": "2.0",
            "format_version": 2,
            "extractor_version": "1",
            "active_variant_id": "test_var",
            "variant_dir": "variants/test_var",
            "base_out_dir": out_dir.display().to_string(),
            "workspace_root": ws_auto.display().to_string(),
            "manifest_path": ws_auto.join("Cargo.toml").display().to_string(),
            "profile": "release",
            "target": null,
            "features": null,
            "no_default_features": false,
            "all_features": false,
            "package_filter": null,
            "timestamp_epoch_secs": 0,
            "total_crates": 1,
            "total_defs": 0,
            "total_refs": 0,
            "total_types": 0,
            "dependency_shards": [],
            "crates": []
        });
        fs::write(
            out_dir.join("ridx_manifest.json"),
            serde_json::to_string(&v_manifest).unwrap(),
        )
        .unwrap();

        // Point current_dir to out_dir where ridx_manifest.json has workspace_root
        std::env::set_current_dir(&out_dir).unwrap();
        let res_from_manifest = try_resolve_workspace(None);
        assert!(res_from_manifest.is_ok());

        // Test manifest with non-existent workspace_root and no enclosing workspace (lines 405-408)
        v_manifest["workspace_root"] =
            serde_json::Value::String("/nonexistent/ws/root".to_string());
        let isolated_manifest_dir = test_auto.0.join("isolated_manifest");
        fs::create_dir_all(&isolated_manifest_dir).unwrap();
        fs::write(
            isolated_manifest_dir.join("ridx_manifest.json"),
            serde_json::to_string(&v_manifest).unwrap(),
        )
        .unwrap();
        std::env::set_current_dir(&isolated_manifest_dir).unwrap();
        let res_isolated = try_resolve_workspace(None);
        assert!(res_isolated
            .unwrap_err()
            .contains("Cannot find Cargo workspace"));

        // Test manifest with empty workspace_root where enclosing workspace exists (line 403)
        let cur_empty_ws = test_auto.0.join("cur_empty_ws");
        fs::create_dir_all(&cur_empty_ws).unwrap();
        let cur_src = cur_empty_ws.join("src");
        fs::create_dir_all(&cur_src).unwrap();
        fs::write(cur_src.join("lib.rs"), "pub fn cur_fn() {}\n").unwrap();
        fs::write(
                cur_empty_ws.join("Cargo.toml"),
                "[package]\nname = \"cur_empty\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[lib]\npath = \"src/lib.rs\"\n",
            )
            .unwrap();
        let cur_target_ridx = cur_empty_ws.join("target/ridx");
        fs::create_dir_all(&cur_target_ridx).unwrap();
        v_manifest["workspace_root"] = serde_json::Value::String("".to_string());
        fs::write(
            cur_target_ridx.join("ridx_manifest.json"),
            serde_json::to_string(&v_manifest).unwrap(),
        )
        .unwrap();
        std::env::set_current_dir(&cur_empty_ws).unwrap();
        let _ = try_resolve_workspace(None);

        std::env::set_current_dir(&orig_cur).unwrap();
        std::env::remove_var("RIDX_NO_DEPS");
    }

    // Test corrupted variant_manifest.json and ridx_manifest.json (lines 218-219, 247-248)
    let corrupt_dir = new_test_dir("corrupt_manifests");
    fs::write(corrupt_dir.0.join("variant_manifest.json"), b"invalid json").unwrap();
    fs::write(corrupt_dir.0.join("ridx_manifest.json"), b"invalid json").unwrap();
    assert!(find_manifest_for_path(&corrupt_dir.0).is_none());

    // Test corrupted index file returning io::ErrorKind::InvalidData (line 381)
    let corrupt_idx_file = corrupt_dir.0.join("bad.ridx");
    fs::write(&corrupt_idx_file, b"not a ridx file").unwrap();
    let res_corrupt = try_resolve_workspace(Some(corrupt_idx_file));
    assert!(res_corrupt.is_err());
    assert!(res_corrupt.unwrap_err().contains("Failed to open index"));

    // Test manifest_dir inside variants/var_id with base_out_dir None (lines 295-300)
    let var_base = new_test_dir("var_base_test");
    fs::write(
            var_base.0.join("Cargo.toml"),
            "[package]\nname = \"var_base\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[lib]\npath = \"src/lib.rs\"\n",
        )
        .unwrap();
    let var_src = var_base.0.join("src");
    fs::create_dir_all(&var_src).unwrap();
    fs::write(var_src.join("lib.rs"), "pub fn v() {}\n").unwrap();

    let variants_dir = var_base.0.join("variants");
    let dev_dir = variants_dir.join("dev");
    fs::create_dir_all(&dev_dir).unwrap();
    let no_base_mf = serde_json::json!({
        "version": "2.0",
        "format_version": ridx_format::CURRENT_FORMAT_VERSION,
        "extractor_version": ridx_format::EXTRACTOR_VERSION,
        "active_variant_id": "dev",
        "variant_dir": "variants/dev",
        "base_out_dir": null,
        "workspace_root": var_base.0.display().to_string(),
        "manifest_path": "",
        "profile": "dev",
        "timestamp_epoch_secs": 0,
        "total_crates": 0,
        "total_defs": 0,
        "total_refs": 0,
        "total_types": 0,
        "dependency_shards": [],
        "crates": []
    });
    fs::write(
        dev_dir.join("ridx_manifest.json"),
        serde_json::to_string(&no_base_mf).unwrap(),
    )
    .unwrap();
    let (m_dir, m_obj) = find_manifest_for_path(&dev_dir).unwrap();
    assert_eq!(m_dir, dev_dir);
    assert_eq!(m_obj.base_out_dir, None);
    let _ = try_resolve_workspace(Some(dev_dir));

    // Also test with manifest_dir ending in variants (line 304)
    fs::write(
        variants_dir.join("ridx_manifest.json"),
        serde_json::to_string(&no_base_mf).unwrap(),
    )
    .unwrap();
    let (v_dir, _) = find_manifest_for_path(&variants_dir).unwrap();
    assert_eq!(v_dir, variants_dir);
    let _ = try_resolve_workspace(Some(variants_dir));

    // Test try_resolve_index_and_target (lines 461-465)
    let _ = try_resolve_index_and_target(
        var_base.0.display().to_string(),
        Some("sample_target".to_string()),
    );
    let orig_c = std::env::current_dir().unwrap();
    std::env::set_current_dir(&var_base.0).unwrap();
    let _ = try_resolve_index_and_target("sample_target".to_string(), None);
    std::env::set_current_dir(&orig_c).unwrap();
}

#[test]
fn test_resolve_workspace_and_index_target_error_paths() {
    let _ = std::panic::catch_unwind(|| {
        resolve_workspace(Some(PathBuf::from("/nonexistent/ridx/path/12345")));
    });
    let _ = std::panic::catch_unwind(|| {
        resolve_index_and_target("/nonexistent/ridx/path/12345".to_string(), None);
    });
}
