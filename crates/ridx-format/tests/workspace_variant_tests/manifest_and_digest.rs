/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use crate::common::unique_temp_dir;
use ridx_format::{DefKind, IndexWriter, RawDefinition, RawReference, WorkspaceIndex};
use std::fs::{self, File};
use std::io::BufWriter;

#[test]
fn test_active_manifest_with_empty_or_escaping_variant_dir_rejected() {
    let base_dir = unique_temp_dir("target_test_invalid_variant_dir_ws");

    // Put a legacy .ridx in base_dir
    let mut writer = IndexWriter::new("legacy_crate", "0.0.1");
    let f = File::create(base_dir.join("legacy.ridx")).unwrap();
    writer.write_to(&mut BufWriter::new(f)).unwrap();

    // 1. Empty variant_dir
    let manifest_empty = serde_json::json!({
        "version": "2.0",
        "active_variant_id": "var1",
        "variant_dir": "",
        "workspace_root": base_dir.display().to_string(),
        "manifest_path": base_dir.join("Cargo.toml").display().to_string(),
        "profile": "dev",
    });
    fs::write(
        base_dir.join("ridx_manifest.json"),
        serde_json::to_string_pretty(&manifest_empty).unwrap(),
    )
    .unwrap();

    let res = WorkspaceIndex::open(&base_dir);
    assert!(res.is_err(), "should reject empty variant_dir");
    assert_eq!(res.err().unwrap().kind(), std::io::ErrorKind::InvalidData);

    // 2. Dot variant_dir
    let manifest_dot = serde_json::json!({
        "version": "2.0",
        "active_variant_id": "var1",
        "variant_dir": ".",
        "workspace_root": base_dir.display().to_string(),
        "manifest_path": base_dir.join("Cargo.toml").display().to_string(),
        "profile": "dev",
    });
    fs::write(
        base_dir.join("ridx_manifest.json"),
        serde_json::to_string_pretty(&manifest_dot).unwrap(),
    )
    .unwrap();

    let res = WorkspaceIndex::open(&base_dir);
    assert!(res.is_err(), "should reject '.' variant_dir");
    assert_eq!(res.err().unwrap().kind(), std::io::ErrorKind::InvalidData);

    // 3. Escaping variant_dir
    let manifest_escape = serde_json::json!({
        "version": "2.0",
        "active_variant_id": "var1",
        "variant_dir": "../escaping",
        "workspace_root": base_dir.display().to_string(),
        "manifest_path": base_dir.join("Cargo.toml").display().to_string(),
        "profile": "dev",
    });
    fs::write(
        base_dir.join("ridx_manifest.json"),
        serde_json::to_string_pretty(&manifest_escape).unwrap(),
    )
    .unwrap();

    let res = WorkspaceIndex::open(&base_dir);
    assert!(res.is_err(), "should reject escaping variant_dir");
    assert_eq!(res.err().unwrap().kind(), std::io::ErrorKind::InvalidData);

    // 4. Mismatched variant_id and variant_dir
    let manifest_mismatch = serde_json::json!({
        "version": "2.0",
        "active_variant_id": "var1",
        "variant_dir": "variants/var2",
        "workspace_root": base_dir.display().to_string(),
        "manifest_path": base_dir.join("Cargo.toml").display().to_string(),
        "profile": "dev",
    });
    fs::write(
        base_dir.join("ridx_manifest.json"),
        serde_json::to_string_pretty(&manifest_mismatch).unwrap(),
    )
    .unwrap();

    let res = WorkspaceIndex::open(&base_dir);
    assert!(
        res.is_err(),
        "should reject mismatched variant_id and variant_dir"
    );
    assert_eq!(res.err().unwrap().kind(), std::io::ErrorKind::InvalidData);

    let _ = fs::remove_dir_all(&base_dir);
}

#[test]
fn test_local_metadata_not_loaded_as_second_shard() {
    let test_dir = unique_temp_dir("ridx_dedup_test");
    let var_dir = test_dir.join("variants/var1");
    let deps_dir = var_dir.join(".deps");
    fs::create_dir_all(&deps_dir).unwrap();

    let mut writer = IndexWriter::new("app", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "main".to_string(),
        path: "app::main".to_string(),
        signature: Some("fn()".to_string()),
        file: "src/main.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 3,
        end_col: 2,
    });
    let app_ridx = var_dir.join("app.ridx");
    writer
        .write_to(&mut BufWriter::new(File::create(&app_ridx).unwrap()))
        .unwrap();

    let app_meta = serde_json::json!({
        "name": "app",
        "version": "0.1.0",
        "unit_key": "unitApp",
        "object_path": app_ridx.display().to_string(),
        "is_dependency": false
    });
    fs::write(
        deps_dir.join("app.json"),
        serde_json::to_string_pretty(&app_meta).unwrap(),
    )
    .unwrap();

    // Manifest erroneously includes the local shard in dependency_shards
    let manifest = serde_json::json!({
        "version": "2.0",
        "format_version": ridx_format::CURRENT_FORMAT_VERSION,
        "extractor_version": ridx_format::EXTRACTOR_VERSION,
        "active_variant_id": "var1",
        "variant_dir": "variants/var1",
        "workspace_root": test_dir.display().to_string(),
        "manifest_path": test_dir.join("Cargo.toml").display().to_string(),
        "profile": "dev",
        "dependency_shards": [
            {
                "name": "app",
                "version": "0.1.0",
                "unit_key": "unitApp",
                "object_path": app_ridx.display().to_string(),
                "is_dependency": false
            }
        ]
    });
    fs::write(
        test_dir.join("ridx_manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let ws = WorkspaceIndex::open(&test_dir).expect("failed to open workspace");
    let symbols = ws.search_symbols("main");
    // Must be exactly 1 symbol, not duplicated!
    assert_eq!(
        symbols.len(),
        1,
        "expected 1 symbol for main, got {}",
        symbols.len()
    );
    assert_eq!(
        ws.stats().total_crates,
        1,
        "expected 1 total crate, got {}",
        ws.stats().total_crates
    );

    // Unit mismatch skips candidate references and returns empty
    let refs_mismatch = ws.find_references_for_unit("app::main", Some("different_unit_key"));
    assert!(refs_mismatch.is_empty());

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_references_target_file_filter_and_computed_snapshot_digest() {
    let ws_dir = unique_temp_dir("target_filter_test");

    // 1. Write crate_a with definition at coords: file = "crates/crate_a/src/types.rs", line = 10
    let mut writer_a = IndexWriter::new("crate_a", "0.1.0");
    writer_a.set_unit_key("unitA");
    writer_a.set_generation("genA");
    writer_a.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "TypeA".to_string(),
        path: "crate_a::types::TypeA".to_string(),
        signature: None,
        file: "crates/crate_a/src/types.rs".to_string(),
        start_line: 10,
        start_col: 1,
        end_line: 15,
        end_col: 2,
    });
    writer_a.add_reference(RawReference {
        file: "crates/crate_a/src/lib.rs".to_string(),
        start_line: 2,
        start_col: 1,
        end_line: 2,
        end_col: 10,
        target_def_id: Some(1),
        target_path: "crate_a::types::TypeA".to_string(),
        target_unit: None,
        target_file: Some("crates/crate_a/src/types.rs".to_string()),
        target_line: 10,
        target_col: 1,
        kind: DefKind::Struct,
        is_write: false,
        is_macro_expansion: false,
    });
    let path_a = ws_dir.join("crate_a.ridx");
    writer_a
        .write_to(&mut BufWriter::new(File::create(&path_a).unwrap()))
        .unwrap();

    // 2. Write crate_b with 2 references to crate_a::types::TypeA:
    // ref1 matches target_file
    // ref2 points to different file
    let mut writer_b = IndexWriter::new("crate_b", "0.1.0");
    writer_b.set_unit_key("unitB");
    writer_b.add_reference(RawReference {
        file: "crates/crate_b/src/lib.rs".to_string(),
        start_line: 5,
        start_col: 1,
        end_line: 5,
        end_col: 10,
        target_def_id: None,
        target_path: "crate_a::types::TypeA".to_string(),
        target_unit: None,
        target_file: Some("crates/crate_a/src/types.rs".to_string()),
        target_line: 10,
        target_col: 1,
        kind: DefKind::Struct,
        is_write: false,
        is_macro_expansion: false,
    });
    writer_b.add_reference(RawReference {
        file: "crates/crate_b/src/lib.rs".to_string(),
        start_line: 6,
        start_col: 1,
        end_line: 6,
        end_col: 10,
        target_def_id: None,
        target_path: "crate_a::types::TypeA".to_string(),
        target_unit: None,
        target_file: Some("crates/crate_a/src/other_diff.rs".to_string()),
        target_line: 10,
        target_col: 1,
        kind: DefKind::Struct,
        is_write: false,
        is_macro_expansion: false,
    });
    // Aliased reference with mismatched target unit (line 556 in navigation.rs)
    writer_b.add_reference(RawReference {
        file: "crates/crate_b/src/lib.rs".to_string(),
        start_line: 7,
        start_col: 1,
        end_line: 7,
        end_col: 10,
        target_def_id: None,
        target_path: "alias_a::types::TypeA".to_string(),
        target_unit: Some("other_unit".to_string()),
        target_file: Some("crates/crate_a/src/types.rs".to_string()),
        target_line: 10,
        target_col: 1,
        kind: DefKind::Struct,
        is_write: false,
        is_macro_expansion: false,
    });
    // Aliased reference with differing target file (lines 578-581 in navigation.rs)
    writer_b.add_reference(RawReference {
        file: "crates/crate_b/src/lib.rs".to_string(),
        start_line: 8,
        start_col: 1,
        end_line: 8,
        end_col: 10,
        target_def_id: None,
        target_path: "alias_a::types::TypeA".to_string(),
        target_unit: None,
        target_file: Some("crates/crate_a/src/unrelated.rs".to_string()),
        target_line: 10,
        target_col: 1,
        kind: DefKind::Struct,
        is_write: false,
        is_macro_expansion: false,
    });
    let path_b = ws_dir.join("crate_b.ridx");
    writer_b
        .write_to(&mut BufWriter::new(File::create(&path_b).unwrap()))
        .unwrap();

    let deps_dir = ws_dir.join(".deps");
    fs::create_dir_all(&deps_dir).unwrap();
    let meta_b = serde_json::json!({
        "name": "crate_b",
        "version": "0.1.0",
        "unit_key": "unitB",
        "object_path": path_b.display().to_string(),
        "dependencies": ["unitA"],
        "extern_units": { "alias_a": "unitA" }
    });
    fs::write(
        deps_dir.join("crate_b.json"),
        serde_json::to_string(&meta_b).unwrap(),
    )
    .unwrap();

    let ws = WorkspaceIndex::open(&ws_dir).expect("open workspace");
    // Unit mismatch on target crate unit (lines 457-460 and 491-495)
    let refs_mismatch =
        ws.find_references_for_unit("crate_a::types::TypeA", Some("different_unit_key"));
    assert!(refs_mismatch.is_empty());

    // References query filters out ref2 and aliased refs with differing target file (lines 505-520 and 578-581)
    let refs = ws.find_references_for_unit("crate_a::types::TypeA", Some("unitA"));
    assert_eq!(refs.len(), 2);

    // Test computed_digest when sidecar has file_hashes and no generation (lines 420-428)
    let mut file_hashes = std::collections::HashMap::new();
    file_hashes.insert("crates/crate_a/src/types.rs".to_string(), 12345u64);
    let meta_computed = serde_json::json!({
        "name": "crate_a",
        "version": "0.1.0",
        "unit_key": "unitA",
        "object_path": path_a.display().to_string(),
        "file_hashes": file_hashes
    });
    fs::write(
        deps_dir.join("crate_a.json"),
        serde_json::to_string(&meta_computed).unwrap(),
    )
    .unwrap();

    let manifest = serde_json::json!({
        "version": "2.0",
        "format_version": ridx_format::CURRENT_FORMAT_VERSION,
        "extractor_version": ridx_format::EXTRACTOR_VERSION,
        "active_variant_id": "",
        "variant_dir": ".",
        "workspace_root": ws_dir.display().to_string(),
        "manifest_path": ws_dir.join("Cargo.toml").display().to_string(),
        "profile": "dev"
    });
    fs::write(
        ws_dir.join("ridx_manifest.json"),
        serde_json::to_string(&manifest).unwrap(),
    )
    .unwrap();

    let ws_manifest = WorkspaceIndex::open(&ws_dir).expect("open workspace with manifest");
    assert!(ws_manifest.crate_count() >= 1);

    let _ = fs::remove_dir_all(&ws_dir);
}
