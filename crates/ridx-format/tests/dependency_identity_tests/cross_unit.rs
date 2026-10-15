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
fn test_exact_unit_cross_definition_and_references() {
    let test_dir = unique_temp_dir("ridx_exact_unit");
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
    writer_a1
        .write_to(&mut BufWriter::new(File::create(&dep_a1_path).unwrap()))
        .unwrap();

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
    writer_a2
        .write_to(&mut BufWriter::new(File::create(&dep_a2_path).unwrap()))
        .unwrap();

    // 3. Workspace crate b1 (depends on unitA1)
    let mut writer_b1 = IndexWriter::new("b1", "0.1.0");
    writer_b1.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "call_a1".to_string(),
        path: "b1::call_a1".to_string(),
        signature: Some("fn()".to_string()),
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 3,
        end_col: 2,
    });
    writer_b1.add_reference(RawReference {
        file: "src/lib.rs".to_string(),
        start_line: 2,
        start_col: 5,
        end_line: 2,
        end_col: 13,
        target_def_id: None,
        target_path: "dep::foo".to_string(),
        target_unit: None,
        target_file: Some("src/lib.rs".to_string()),
        target_line: 10,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    let b1_ridx = var_dir.join("b1.ridx");
    writer_b1
        .write_to(&mut BufWriter::new(File::create(&b1_ridx).unwrap()))
        .unwrap();

    // Write .deps/b1.json metadata specifying exact unit dependency
    let b1_meta = serde_json::json!({
        "name": "b1",
        "version": "0.1.0",
        "unit_key": "unitB1",
        "object_path": b1_ridx.display().to_string(),
        "dependencies": ["unitA1"],
        "extern_units": {
            "dep": "unitA1"
        }
    });
    fs::write(
        deps_dir.join("b1.json"),
        serde_json::to_string_pretty(&b1_meta).unwrap(),
    )
    .unwrap();

    // 4. Workspace crate b2 (depends on unitA2)
    let mut writer_b2 = IndexWriter::new("b2", "0.1.0");
    writer_b2.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "call_a2".to_string(),
        path: "b2::call_a2".to_string(),
        signature: Some("fn()".to_string()),
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 3,
        end_col: 2,
    });
    writer_b2.add_reference(RawReference {
        file: "src/lib.rs".to_string(),
        start_line: 2,
        start_col: 5,
        end_line: 2,
        end_col: 13,
        target_def_id: None,
        target_path: "dep::foo".to_string(),
        target_unit: None,
        target_file: Some("src/lib.rs".to_string()),
        target_line: 20,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    let b2_ridx = var_dir.join("b2.ridx");
    writer_b2
        .write_to(&mut BufWriter::new(File::create(&b2_ridx).unwrap()))
        .unwrap();

    // Write .deps/b2.json metadata specifying exact unit dependency
    let b2_meta = serde_json::json!({
        "name": "b2",
        "version": "0.1.0",
        "unit_key": "unitB2",
        "object_path": b2_ridx.display().to_string(),
        "dependencies": ["unitA2"],
        "extern_units": {
            "dep": "unitA2"
        }
    });
    fs::write(
        deps_dir.join("b2.json"),
        serde_json::to_string_pretty(&b2_meta).unwrap(),
    )
    .unwrap();

    // 5. Manifest linking both dependency shards
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
    fs::write(
        test_dir.join("ridx_manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let ws = WorkspaceIndex::open(&test_dir)
        .expect("failed to open workspace with multi-version dependency shards");

    // Cross-definition from b1 must resolve strictly to unitA1
    let def_from_b1 = ws
        .find_definition_at("src/lib.rs", 2, 6)
        .expect("should resolve dep::foo from b1");
    assert_eq!(def_from_b1.unit_key, Some("unitA1"));
    assert_eq!(def_from_b1.start_line, 10);
    assert_eq!(def_from_b1.signature, Some("fn() -> i32"));

    // Cross-references for unitA1 must ONLY include b1, excluding b2!
    let refs_a1 = ws.find_references_for_unit("dep::foo", Some("unitA1"));
    assert_eq!(
        refs_a1.len(),
        1,
        "references to unitA1 must only include b1"
    );
    assert_eq!(refs_a1[0].crate_name, "b1");

    // Cross-references for unitA2 must ONLY include b2, excluding b1!
    let refs_a2 = ws.find_references_for_unit("dep::foo", Some("unitA2"));
    assert_eq!(
        refs_a2.len(),
        1,
        "references to unitA2 must only include b2"
    );
    assert_eq!(refs_a2[0].crate_name, "b2");

    let _ = fs::remove_dir_all(&test_dir);
}
