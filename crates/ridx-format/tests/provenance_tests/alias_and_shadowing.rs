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
use ridx_format::{DefKind, IndexWriter, RawDefinition, WorkspaceIndex};
use std::fs::{self, File};
use std::io::BufWriter;

#[test]
fn test_absolute_source_alias_cannot_shadow_authoritative_embedded_hash() {
    let test_dir = unique_temp_dir("ridx_alias_shadow_test");
    let _ = fs::remove_dir_all(&test_dir);

    let var_dir = test_dir.join("variants/var1");
    fs::create_dir_all(&var_dir).unwrap();

    // 1. Write workspace crate
    let mut writer_app = IndexWriter::new("app", "0.1.0");
    writer_app.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "main".to_string(),
        path: "app::main".to_string(),
        signature: None,
        file: "src/main.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 10,
    });
    writer_app
        .write_to(&mut BufWriter::new(
            File::create(var_dir.join("app.ridx")).unwrap(),
        ))
        .unwrap();

    // 2. Write dependency crate A1 with generation computed from embedded hash 111
    let dep_dir = test_dir.join("registry/a1");
    fs::create_dir_all(&dep_dir).unwrap();
    let mut writer_a1 = IndexWriter::new("a1", "0.1.0");
    writer_a1.set_unit_key("unitA1");
    let mut embedded_hashes = std::collections::HashMap::new();
    embedded_hashes.insert("src/lib.rs".to_string(), 111u64);
    let digest = ridx_format::compute_source_snapshot_digest(&embedded_hashes);
    let bin_gen = format!("{:016x}", digest);
    writer_a1.set_generation(&bin_gen);
    writer_a1.set_file_hashes(embedded_hashes);
    writer_a1.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "dep_fn".to_string(),
        path: "a1::dep_fn".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 10,
    });
    let a1_ridx = dep_dir.join("a1.ridx");
    writer_a1
        .write_to(&mut BufWriter::new(File::create(&a1_ridx).unwrap()))
        .unwrap();

    // 3. Write ridx_manifest.json with matching binary generation, but containing a stale absolute alias 222
    let abs_lib_rs = dep_dir.join("src/lib.rs").display().to_string();
    let manifest_json = serde_json::json!({
        "version": "2.0",
        "format_version": ridx_format::CURRENT_FORMAT_VERSION,
        "extractor_version": ridx_format::EXTRACTOR_VERSION,
        "active_variant_id": "var1",
        "variant_dir": "variants/var1",
        "workspace_root": test_dir.display().to_string(),
        "manifest_path": test_dir.join("Cargo.toml").display().to_string(),
        "profile": "dev",
        "timestamp_epoch_secs": 1000,
        "total_crates": 1,
        "total_defs": 1,
        "total_refs": 0,
        "total_types": 0,
        "dependency_shards": [{
            "name": "a1",
            "version": "0.1.0",
            "unit_key": "unitA1",
            "generation": bin_gen,
            "object_path": a1_ridx.display().to_string(),
            "source_root": dep_dir.display().to_string(),
            "is_dependency": true,
            "dependencies": [],
            "extern_units": {},
            "file_hashes": {
                "src/lib.rs": 111,
                abs_lib_rs: 222
            }
        }]
    });
    fs::write(
        test_dir.join("ridx_manifest.json"),
        serde_json::to_string_pretty(&manifest_json).unwrap(),
    )
    .unwrap();

    let ws = WorkspaceIndex::open(&test_dir).expect("failed to open workspace");

    // The embedded hash 111 MUST NOT be shadowed by the stale absolute alias 222!
    let a1_file = dep_dir.join("src/lib.rs");
    let res = ws.source_hash_for_file(&a1_file.to_string_lossy());
    assert_eq!(
        res,
        Some(111),
        "authoritative embedded hash 111 must take precedence over stale absolute alias 222"
    );

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_current_workspace_exact_source_beats_suffix_match() {
    let test_dir = unique_temp_dir("ridx_exact_source_test");
    let _ = fs::remove_dir_all(&test_dir);
    let ws_root = test_dir.join("workspace");
    let var_dir = ws_root.join("target/ridx/variants/var1");
    fs::create_dir_all(&var_dir).unwrap();

    // 1. Shard 00_member.ridx embeds /workspace/crates/member/src/lib.rs -> 111
    let member_file = ws_root.join("crates/member/src/lib.rs");
    let mut writer_member = IndexWriter::new("member", "0.1.0");
    writer_member.set_unit_key("unitMember");
    let mut member_hashes = std::collections::HashMap::new();
    member_hashes.insert(member_file.display().to_string(), 111u64);
    writer_member.set_file_hashes(member_hashes);
    writer_member.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "member_fn".to_string(),
        path: "member::member_fn".to_string(),
        signature: None,
        file: member_file.display().to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 10,
    });
    let member_ridx = var_dir.join("00_member.ridx");
    writer_member
        .write_to(&mut BufWriter::new(File::create(&member_ridx).unwrap()))
        .unwrap();

    // 2. Shard app.ridx embeds /workspace/src/lib.rs -> 222
    let root_file = ws_root.join("src/lib.rs");
    let mut writer_app = IndexWriter::new("app", "0.1.0");
    writer_app.set_unit_key("unitApp");
    let mut app_hashes = std::collections::HashMap::new();
    app_hashes.insert(root_file.display().to_string(), 222u64);
    writer_app.set_file_hashes(app_hashes);
    writer_app.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "app_fn".to_string(),
        path: "app::app_fn".to_string(),
        signature: None,
        file: root_file.display().to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 10,
    });
    let app_ridx = var_dir.join("app.ridx");
    writer_app
        .write_to(&mut BufWriter::new(File::create(&app_ridx).unwrap()))
        .unwrap();

    // 3. Manifest recording workspace_root
    let manifest_json = serde_json::json!({
        "version": "2.0",
        "format_version": ridx_format::CURRENT_FORMAT_VERSION,
        "extractor_version": ridx_format::EXTRACTOR_VERSION,
        "active_variant_id": "var1",
        "variant_dir": "variants/var1",
        "workspace_root": ws_root.display().to_string(),
        "manifest_path": ws_root.join("Cargo.toml").display().to_string(),
        "profile": "dev",
        "timestamp_epoch_secs": 1000,
        "total_crates": 2,
        "total_defs": 2,
        "total_refs": 0,
        "total_types": 0,
        "dependency_shards": []
    });
    fs::write(
        ws_root.join("target/ridx/ridx_manifest.json"),
        serde_json::to_string_pretty(&manifest_json).unwrap(),
    )
    .unwrap();

    let ws = WorkspaceIndex::open(&ws_root).expect("failed to open workspace");

    // Root file must return 222 (exact match), not 111 (suffix match on member's lib.rs)
    let res_root = ws.source_hash_for_file(&root_file.display().to_string());
    assert_eq!(
        res_root,
        Some(222),
        "exact root file query must not match member's lib.rs via suffix"
    );

    // Member file must return 111
    let res_member = ws.source_hash_for_file(&member_file.display().to_string());
    assert_eq!(res_member, Some(111));

    // Relative root file query must also match root file 222
    let res_rel_root = ws.source_hash_for_file("src/lib.rs");
    assert_eq!(res_rel_root, Some(222));

    // Relative member file query must match member file 111
    let res_rel_member = ws.source_hash_for_file("crates/member/src/lib.rs");
    assert_eq!(res_rel_member, Some(111));

    let _ = fs::remove_dir_all(&test_dir);
}
