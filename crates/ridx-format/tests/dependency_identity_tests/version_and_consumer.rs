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
fn test_references_scoped_to_exact_dependency_version_with_same_coordinates() {
    let test_dir = unique_temp_dir("ridx_dep_coords_test");
    let var_dir = test_dir.join("variants/var1");
    let deps_dir = var_dir.join(".deps");
    fs::create_dir_all(&deps_dir).unwrap();

    let cache_dir = test_dir.join("cache");
    let dep_a1_path = cache_dir.join("a1.ridx");
    let dep_a2_path = cache_dir.join("a2.ridx");
    fs::create_dir_all(&cache_dir).unwrap();

    // 1. Dependency shard A1: crate "a" v1.0.0, unitA1, defines a::Thing at line 10
    let mut writer_a1 = IndexWriter::new("a", "1.0.0");
    writer_a1.set_unit_key("unitA1");
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

    // 2. Dependency shard A2: crate "a" v2.0.0, unitA2, ALSO defines a::Thing at line 10 in src/lib.rs!
    let mut writer_a2 = IndexWriter::new("a", "2.0.0");
    writer_a2.set_unit_key("unitA2");
    writer_a2.add_definition(RawDefinition {
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
    writer_a2.write_to(&mut BufWriter::new(File::create(&dep_a2_path).unwrap())).unwrap();

    // 3. Consumer crate C: references a::Thing at line 4 (target_unit = unitA1)
    // and a::Thing at line 8 (target_unit = unitA2)
    let mut writer_c = IndexWriter::new("c", "0.1.0");
    writer_c.set_unit_key("unitC");
    writer_c.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 4,
        start_col: 5,
        end_line: 4,
        end_col: 13,
        target_def_id: None,
        target_path: "a::Thing".to_string(),
        target_unit: Some("unitA1".to_string()),
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
        target_unit: Some("unitA2".to_string()),
        target_file: Some("src/lib.rs".to_string()),
        target_line: 10,
        target_col: 1,
        kind: DefKind::Struct,
        is_write: false,
        is_macro_expansion: false,
    });
    let c_ridx = var_dir.join("c.ridx");
    writer_c.write_to(&mut BufWriter::new(File::create(&c_ridx).unwrap())).unwrap();

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

    // Querying references for unitA1 must ONLY return line 4 (not line 8)
    let refs_a1 = ws.find_references_for_unit("a::Thing", Some("unitA1"));
    assert_eq!(refs_a1.len(), 1, "expected only line 4 for unitA1");
    assert_eq!(refs_a1[0].span.start_line(), 4);

    // Querying references for unitA2 must ONLY return line 8 (not line 4)
    let refs_a2 = ws.find_references_for_unit("a::Thing", Some("unitA2"));
    assert_eq!(refs_a2.len(), 1, "expected only line 8 for unitA2");
    assert_eq!(refs_a2[0].span.start_line(), 8);

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_same_named_consumer_definition_and_references() {
    let test_dir = unique_temp_dir("ridx_same_name");

    // Crate A1: unit_key = "A1", crate_name = "a"
    let mut writer_a1 = IndexWriter::new("a", "1.0.0");
    writer_a1.set_unit_key("A1");
    writer_a1.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "Item".to_string(),
        path: "a::Item".to_string(),
        signature: None,
        file: "crates/a1/src/lib.rs".to_string(),
        start_line: 10,
        start_col: 1,
        end_line: 12,
        end_col: 2,
    });
    writer_a1.write_to(&mut BufWriter::new(File::create(test_dir.join("a1.ridx")).unwrap())).unwrap();

    // Crate A2: unit_key = "A2", crate_name = "a", depends on A1!
    let mut writer_a2 = IndexWriter::new("a", "2.0.0");
    writer_a2.set_unit_key("A2");
    writer_a2.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "use_item".to_string(),
        path: "a::use_item".to_string(),
        signature: None,
        file: "crates/a2/src/lib.rs".to_string(),
        start_line: 5,
        start_col: 1,
        end_line: 8,
        end_col: 2,
    });
    writer_a2.add_reference(RawReference {
        target_def_id: None,
        target_path: "a::Item".to_string(),
        kind: DefKind::Struct,
        is_write: false,
        is_macro_expansion: false,
        file: "crates/a2/src/lib.rs".to_string(),
        start_line: 6,
        start_col: 10,
        end_line: 6,
        end_col: 14,
        target_unit: Some("A1".to_string()),
        target_file: Some("crates/a1/src/lib.rs".to_string()),
        target_line: 10,
        target_col: 0,
    });
    writer_a2.write_to(&mut BufWriter::new(File::create(test_dir.join("a2.ridx")).unwrap())).unwrap();

    // Publish metadata for both
    let deps_dir = test_dir.join(".deps");
    fs::create_dir_all(&deps_dir).unwrap();
    let meta_a1 = serde_json::json!({
        "name": "a",
        "version": "1.0.0",
        "unit_key": "A1",
        "object_path": test_dir.join("a1.ridx").display().to_string(),
        "dependencies": [],
        "extern_units": {},
        "file_hashes": {},
    });
    fs::write(deps_dir.join("a1.json"), serde_json::to_string_pretty(&meta_a1).unwrap()).unwrap();

    let meta_a2 = serde_json::json!({
        "name": "a",
        "version": "2.0.0",
        "unit_key": "A2",
        "object_path": test_dir.join("a2.ridx").display().to_string(),
        "dependencies": ["A1"],
        "extern_units": {},
        "file_hashes": {},
    });
    fs::write(deps_dir.join("a2.json"), serde_json::to_string_pretty(&meta_a2).unwrap()).unwrap();

    let ws = WorkspaceIndex::open(&test_dir).expect("should open workspace");
    assert_eq!(ws.crate_count(), 2);

    // 1. Definition lookup at reference in A2 should resolve to A1, NOT be treated as local to A2
    let def = ws.find_definition_at("crates/a2/src/lib.rs", 6, 11).expect("should resolve definition");
    assert_eq!(def.name, "Item");
    assert_eq!(def.unit_key, Some("A1"));
    assert_eq!(def.file, "crates/a1/src/lib.rs");

    // 2. References lookup for a::Item in A1 should find reference in consumer A2
    let refs = ws.find_references_for_unit("a::Item", Some("A1"));
    assert_eq!(refs.len(), 1, "A2 should not be skipped as a same-named consumer");
    assert_eq!(refs[0].file, "crates/a2/src/lib.rs");
    assert_eq!(refs[0].unit_key, Some("A2"));

    let _ = fs::remove_dir_all(&test_dir);
}
