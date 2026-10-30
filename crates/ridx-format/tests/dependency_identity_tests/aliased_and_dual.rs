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
fn test_consumer_with_aliased_dependencies() {
    let test_dir = unique_temp_dir("ridx_aliased_dep");
    let var_dir = test_dir.join("variants/var1");
    let deps_dir = var_dir.join(".deps");
    fs::create_dir_all(&deps_dir).unwrap();

    let cache_dir = test_dir.join("cache/objects");
    fs::create_dir_all(&cache_dir).unwrap();
    let dep_a1_path = cache_dir.join("dep_a1.ridx");
    let dep_a2_path = cache_dir.join("dep_a2.ridx");

    // 1. Shard dep v1.0.0 (unitA1)
    let mut writer_a1 = IndexWriter::new("dep", "1.0.0");
    writer_a1.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "foo".to_string(),
        path: "dep::foo".to_string(),
        signature: Some("fn() -> i32".to_string()),
        file: "src/lib.rs".to_string(),
        start_line: 10,
        start_col: 1,
        end_line: 12,
        end_col: 2,
    });
    writer_a1.write_to(&mut BufWriter::new(File::create(&dep_a1_path).unwrap())).unwrap();

    // 2. Shard dep v2.0.0 (unitA2)
    let mut writer_a2 = IndexWriter::new("dep", "2.0.0");
    writer_a2.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "foo".to_string(),
        path: "dep::foo".to_string(),
        signature: Some("fn() -> String".to_string()),
        file: "src/lib.rs".to_string(),
        start_line: 20,
        start_col: 1,
        end_line: 22,
        end_col: 2,
    });
    writer_a2.write_to(&mut BufWriter::new(File::create(&dep_a2_path).unwrap())).unwrap();

    // 3. Workspace consumer crate with aliased extern names:
    // dep_old -> unitA1, dep_new -> unitA2
    let mut writer_consumer = IndexWriter::new("consumer", "0.1.0");
    writer_consumer.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 3,
        start_col: 5,
        end_line: 3,
        end_col: 17,
        target_def_id: None,
        target_path: "dep_old::foo".to_string(),
        target_unit: None,
        target_file: Some("src/lib.rs".to_string()),
        target_line: 10,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer_consumer.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 7,
        start_col: 5,
        end_line: 7,
        end_col: 17,
        target_def_id: None,
        target_path: "dep_new::foo".to_string(),
        target_unit: None,
        target_file: Some("src/lib.rs".to_string()),
        target_line: 20,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    let consumer_ridx = var_dir.join("consumer.ridx");
    writer_consumer.write_to(&mut BufWriter::new(File::create(&consumer_ridx).unwrap())).unwrap();

    let consumer_meta = serde_json::json!({
        "name": "consumer",
        "version": "0.1.0",
        "unit_key": "unitConsumer",
        "object_path": consumer_ridx.display().to_string(),
        "dependencies": ["unitA1", "unitA2"],
        "extern_units": {
            "dep_old": "unitA1",
            "dep_new": "unitA2"
        }
    });
    fs::write(deps_dir.join("consumer.json"), serde_json::to_string_pretty(&consumer_meta).unwrap()).unwrap();

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
                "unit_key": "unitA1",
                "object_path": dep_a1_path.display().to_string(),
                "source_root": "/registry/dep-1.0.0"
            },
            {
                "name": "dep",
                "version": "2.0.0",
                "unit_key": "unitA2",
                "object_path": dep_a2_path.display().to_string(),
                "source_root": "/registry/dep-2.0.0"
            }
        ]
    });
    fs::write(test_dir.join("ridx_manifest.json"), serde_json::to_string_pretty(&manifest).unwrap()).unwrap();

    let ws = WorkspaceIndex::open(&test_dir).expect("failed to open workspace with aliased dependencies");

    // Definition via dep_old alias must resolve to unitA1 (v1.0.0 at line 10)
    let def_old = ws.find_definition_at("src/main.rs", 3, 6).expect("should resolve dep_old::foo");
    assert_eq!(def_old.unit_key, Some("unitA1"));
    assert_eq!(def_old.start_line, 10);
    assert_eq!(def_old.signature, Some("fn() -> i32"));

    // Definition via dep_new alias must resolve to unitA2 (v2.0.0 at line 20)
    let def_new = ws.find_definition_at("src/main.rs", 7, 6).expect("should resolve dep_new::foo");
    assert_eq!(def_new.unit_key, Some("unitA2"));
    assert_eq!(def_new.start_line, 20);
    assert_eq!(def_new.signature, Some("fn() -> String"));

    // References for unitA1 must find the aliased reference at line 3
    let refs_a1 = ws.find_references_for_unit("dep::foo", Some("unitA1"));
    assert_eq!(refs_a1.len(), 1);
    assert_eq!(refs_a1[0].span.start_line(), 3);

    // References for unitA2 must find the aliased reference at line 7
    let refs_a2 = ws.find_references_for_unit("dep::foo", Some("unitA2"));
    assert_eq!(refs_a2.len(), 1);
    assert_eq!(refs_a2[0].span.start_line(), 7);

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_consumer_with_dual_version_canonical_references() {
    let test_dir = unique_temp_dir("ridx_canonical_dual");
    let var_dir = test_dir.join("variants/var1");
    let deps_dir = var_dir.join(".deps");
    fs::create_dir_all(&deps_dir).unwrap();

    let cache_dir = test_dir.join("cache/objects");
    fs::create_dir_all(&cache_dir).unwrap();
    let dep_a1_path = cache_dir.join("dep_a1.ridx");
    let dep_a2_path = cache_dir.join("dep_a2.ridx");

    // 1. Dependency shard A1: crate "a" v1.0.0, unitA1, defines a::Thing at line 10
    let mut writer_a1 = IndexWriter::new("a", "1.0.0");
    writer_a1.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "Thing".to_string(),
        path: "a::Thing".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 10,
        start_col: 1,
        end_line: 12,
        end_col: 2,
    });
    writer_a1.write_to(&mut BufWriter::new(File::create(&dep_a1_path).unwrap())).unwrap();

    // 2. Dependency shard A2: crate "a" v2.0.0, unitA2, defines a::Thing at line 20
    let mut writer_a2 = IndexWriter::new("a", "2.0.0");
    writer_a2.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "Thing".to_string(),
        path: "a::Thing".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 20,
        start_col: 1,
        end_line: 22,
        end_col: 2,
    });
    writer_a2.write_to(&mut BufWriter::new(File::create(&dep_a2_path).unwrap())).unwrap();

    // 3. Consumer crate C: references canonical a::Thing at line 4 (pointing to A1's line 10)
    // and canonical a::Thing at line 8 (pointing to A2's line 20)
    let mut writer_c = IndexWriter::new("c", "0.1.0");
    writer_c.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 4,
        start_col: 5,
        end_line: 4,
        end_col: 13,
        target_def_id: None,
        target_path: "a::Thing".to_string(),
        target_unit: None,
        target_file: Some("src/lib.rs".to_string()),
        target_line: 10,
        target_col: 1,
        kind: DefKind::Struct,
        is_write: false,
        is_macro_expansion: false,
    });
    writer_c.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 8,
        start_col: 5,
        end_line: 8,
        end_col: 13,
        target_def_id: None,
        target_path: "a::Thing".to_string(),
        target_unit: None,
        target_file: Some("src/lib.rs".to_string()),
        target_line: 20,
        target_col: 1,
        kind: DefKind::Struct,
        is_write: false,
        is_macro_expansion: false,
    });
    let c_ridx = var_dir.join("c.ridx");
    writer_c.write_to(&mut BufWriter::new(File::create(&c_ridx).unwrap())).unwrap();

    // Sidecar metadata for C with extern_units mapping a1 -> unitA1, a2 -> unitA2
    let c_meta = serde_json::json!({
        "name": "c",
        "version": "0.1.0",
        "unit_key": "unitC",
        "object_path": c_ridx.display().to_string(),
        "dependencies": ["unitA1", "unitA2"],
        "extern_units": {
            "a1": "unitA1",
            "a2": "unitA2"
        }
    });
    fs::write(deps_dir.join("c.json"), serde_json::to_string_pretty(&c_meta).unwrap()).unwrap();

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
                "name": "a",
                "version": "1.0.0",
                "unit_key": "unitA1",
                "object_path": dep_a1_path.display().to_string(),
                "source_root": "/registry/a-1.0.0",
                "is_dependency": true
            },
            {
                "name": "a",
                "version": "2.0.0",
                "unit_key": "unitA2",
                "object_path": dep_a2_path.display().to_string(),
                "source_root": "/registry/a-2.0.0",
                "is_dependency": true
            }
        ]
    });
    fs::write(test_dir.join("ridx_manifest.json"), serde_json::to_string_pretty(&manifest).unwrap()).unwrap();

    let ws = WorkspaceIndex::open(&test_dir).expect("failed to open workspace");

    // Definition resolution at line 4 (pointing to A1 coords) must resolve to unitA1
    let def_line4 = ws.find_definition_at("src/main.rs", 4, 6).expect("should resolve line 4");
    assert_eq!(def_line4.unit_key, Some("unitA1"));
    assert_eq!(def_line4.start_line, 10);

    // Definition resolution at line 8 (pointing to A2 coords) must resolve to unitA2
    let def_line8 = ws.find_definition_at("src/main.rs", 8, 6).expect("should resolve line 8");
    assert_eq!(def_line8.unit_key, Some("unitA2"));
    assert_eq!(def_line8.start_line, 20);

    // Querying references for unitA1 must only return line 4 (not line 8)
    let refs_a1 = ws.find_references_for_unit("a::Thing", Some("unitA1"));
    assert_eq!(refs_a1.len(), 1, "expected only line 4 for unitA1");
    assert_eq!(refs_a1[0].span.start_line(), 4);

    // Querying references for unitA2 must only return line 8 (not line 4)
    let refs_a2 = ws.find_references_for_unit("a::Thing", Some("unitA2"));
    assert_eq!(refs_a2.len(), 1, "expected only line 8 for unitA2");
    assert_eq!(refs_a2[0].span.start_line(), 8);

    let _ = fs::remove_dir_all(&test_dir);
}
