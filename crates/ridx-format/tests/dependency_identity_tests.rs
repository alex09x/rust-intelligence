/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

#[path = "dependency_identity_tests/aliased_and_dual.rs"]
mod aliased_and_dual;
mod common;
#[path = "dependency_identity_tests/cross_unit.rs"]
mod cross_unit;
#[path = "dependency_identity_tests/version_and_consumer.rs"]
mod version_and_consumer;

use common::unique_temp_dir;
use ridx_format::{DefKind, IndexWriter, RawDefinition, WorkspaceIndex};
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::PathBuf;

#[test]
fn test_dependency_shard_loading() {
    let test_dir = unique_temp_dir("ridx_dep_test");
    let var_dir = test_dir.join("variants/var1");
    fs::create_dir_all(&var_dir).unwrap();

    // 1. Local crate in workspace
    let mut writer_local = IndexWriter::new("local_app", "0.1.0");
    writer_local.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "main".to_string(),
        path: "local_app::main".to_string(),
        signature: Some("fn()".to_string()),
        file: "src/main.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 5,
        end_col: 2,
    });
    let f_local = File::create(var_dir.join("local_app.ridx")).unwrap();
    writer_local.write_to(&mut BufWriter::new(f_local)).unwrap();

    // 2. Dependency crate in separate cache location
    let cache_dir = test_dir.join("cache/objects/ab");
    fs::create_dir_all(&cache_dir).unwrap();
    let dep_obj_path = cache_dir.join("ab1234567890.ridx");

    let mut writer_dep = IndexWriter::new("dep_lib", "1.2.0");
    writer_dep.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "DepConfig".to_string(),
        path: "dep_lib::DepConfig".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 10,
        start_col: 1,
        end_line: 20,
        end_col: 2,
    });
    let f_dep = File::create(&dep_obj_path).unwrap();
    writer_dep.write_to(&mut BufWriter::new(f_dep)).unwrap();

    // 3. Manifest linking dependency shard
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
                "name": "dep_lib",
                "version": "1.2.0",
                "unit_key": "ab1234567890",
                "object_path": dep_obj_path.display().to_string(),
                "source_root": "/cargo/registry/dep_lib-1.2.0"
            }
        ]
    });
    fs::write(test_dir.join("ridx_manifest.json"), serde_json::to_string_pretty(&manifest).unwrap()).unwrap();

    // 4. Open workspace and verify both local and dependency crates are accessible
    let ws = WorkspaceIndex::open(&test_dir).expect("failed to open workspace with dependency shards");
    assert_eq!(ws.crate_count(), 2);

    let stats = ws.stats();
    assert_eq!(stats.total_crates, 2);
    let dep_summary = stats.crate_summaries.iter().find(|c| c.name == "dep_lib").unwrap();
    assert!(dep_summary.is_dependency);

    // Cross-crate query reaches dependency definition
    let def = ws.find_definition_by_path("dep_lib::DepConfig").expect("should find dep definition");
    assert_eq!(def.name, "DepConfig");

    // Path resolution for dependency crate
    let resolved = ws.resolve_path_for_crate("dep_lib", def.file);
    assert_eq!(resolved, PathBuf::from("/cargo/registry/dep_lib-1.2.0/src/lib.rs"));

    // Find definition at cursor inside dependency file (given absolute path)
    let def_at = ws.find_definition_at("/cargo/registry/dep_lib-1.2.0/src/lib.rs", 12, 5);
    assert!(def_at.is_some());
    assert_eq!(def_at.unwrap().name, "DepConfig");

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_reviewer_cursor_keeps_owning_dependency_unit() {
    let test_dir = unique_temp_dir("ridx_dep_unit_test");
    let var_dir = test_dir.join("variants/var1");
    fs::create_dir_all(&var_dir).unwrap();

    let cache_dir = test_dir.join("cache/objects/ab");
    fs::create_dir_all(&cache_dir).unwrap();
    let dep_a_path = cache_dir.join("unitA.ridx");
    let dep_b_path = cache_dir.join("unitB.ridx");

    // Local workspace crate
    let mut writer_local = IndexWriter::new("local_app", "0.1.0");
    writer_local.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "main".to_string(),
        path: "local_app::main".to_string(),
        signature: Some("fn()".to_string()),
        file: "src/main.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 5,
        end_col: 2,
    });
    writer_local.write_to(&mut BufWriter::new(File::create(var_dir.join("local_app.ridx")).unwrap())).unwrap();

    // Both shard A and shard B define `dep::Thing`
    let mut writer_a = IndexWriter::new("dep", "1.0.0");
    writer_a.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "Thing".to_string(),
        path: "dep::Thing".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 10,
        start_col: 1,
        end_line: 15,
        end_col: 2,
    });
    writer_a.write_to(&mut BufWriter::new(File::create(&dep_a_path).unwrap())).unwrap();

    let mut writer_b = IndexWriter::new("dep", "2.0.0");
    writer_b.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "Thing".to_string(),
        path: "dep::Thing".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 20,
        start_col: 1,
        end_line: 25,
        end_col: 2,
    });
    writer_b.write_to(&mut BufWriter::new(File::create(&dep_b_path).unwrap())).unwrap();

    // Manifest with both dependency shards
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
                "name": "dep",
                "version": "1.0.0",
                "unit_key": "unitA",
                "object_path": dep_a_path.display().to_string(),
                "source_root": "/registry/dep-a"
            },
            {
                "name": "dep",
                "version": "2.0.0",
                "unit_key": "unitB",
                "object_path": dep_b_path.display().to_string(),
                "source_root": "/registry/dep-b"
            }
        ]
    });
    fs::write(test_dir.join("ridx_manifest.json"), serde_json::to_string_pretty(&manifest).unwrap()).unwrap();

    let ws = WorkspaceIndex::open(&test_dir).expect("failed to open workspace with multi-version dependency shards");

    // Querying cursor inside dep-b (/registry/dep-b/src/lib.rs at line 20) MUST resolve to unitB, NOT unitA!
    let def_b = ws.find_definition_at("/registry/dep-b/src/lib.rs", 20, 1).expect("should resolve in unit B");
    assert_eq!(def_b.name, "Thing");
    assert_eq!(def_b.unit_key, Some("unitB"));
    assert_eq!(def_b.start_line, 20);

    // Querying cursor inside dep-a (/registry/dep-a/src/lib.rs at line 10) MUST resolve to unitA!
    let def_a = ws.find_definition_at("/registry/dep-a/src/lib.rs", 10, 1).expect("should resolve in unit A");
    assert_eq!(def_a.name, "Thing");
    assert_eq!(def_a.unit_key, Some("unitA"));
    assert_eq!(def_a.start_line, 10);

    let _ = fs::remove_dir_all(&test_dir);
}
