/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

mod common;

use common::unique_temp_dir;
use ridx_format::{DefKind, IndexWriter, RawDefinition, RawReference, WorkspaceIndex};
use std::fs::File;
use std::io::BufWriter;

#[test]
fn test_disambiguation_and_unit_scoping_edges() {
    let ws_dir = unique_temp_dir("nav_edge_test");

    // 1. crate_c v1: defines Item at line 10
    let mut writer_c1 = IndexWriter::new("crate_c", "0.1.0");
    writer_c1.set_unit_key("crate_c-0.1.0");
    writer_c1.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "Item".to_string(),
        path: "crate_c::Item".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 10,
        start_col: 1,
        end_line: 10,
        end_col: 10,
    });
    let p_c1 = ws_dir.join("crate_c1.ridx");
    writer_c1
        .write_to(&mut BufWriter::new(File::create(&p_c1).unwrap()))
        .unwrap();

    // 2. crate_c v2: defines Item at line 20
    let mut writer_c2 = IndexWriter::new("crate_c", "0.2.0");
    writer_c2.set_unit_key("crate_c-0.2.0");
    writer_c2.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "Item".to_string(),
        path: "crate_c::Item".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 20,
        start_col: 1,
        end_line: 20,
        end_col: 10,
    });
    let p_c2 = ws_dir.join("crate_c2.ridx");
    writer_c2
        .write_to(&mut BufWriter::new(File::create(&p_c2).unwrap()))
        .unwrap();

    // 3. crate_ref: references crate_c::Item in various scenarios
    let mut writer_ref = IndexWriter::new("crate_ref", "0.1.0");
    writer_ref.set_unit_key("crate_ref-0.1.0");

    // Ref 1: matches line 10 (disambiguation chooses c1 by line)
    writer_ref.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 5,
        start_col: 1,
        end_line: 5,
        end_col: 5,
        target_def_id: None,
        target_path: "crate_c::Item".to_string(),
        target_unit: None,
        target_file: Some("src/lib.rs".to_string()),
        target_line: 10,
        target_col: 1,
        kind: DefKind::Struct,
        is_write: false,
        is_macro_expansion: false,
    });

    // Ref 2: matches neither line 10 nor line 20 (line 99) -> disambiguation fails (returns None)
    writer_ref.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 6,
        start_col: 1,
        end_line: 6,
        end_col: 5,
        target_def_id: None,
        target_path: "crate_c::Item".to_string(),
        target_unit: None,
        target_file: Some("src/other.rs".to_string()),
        target_line: 99,
        target_col: 1,
        kind: DefKind::Struct,
        is_write: false,
        is_macro_expansion: false,
    });

    // Ref 3: target_unit is specified but the unit does not contain NonExistent -> returns None
    writer_ref.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 7,
        start_col: 1,
        end_line: 7,
        end_col: 5,
        target_def_id: None,
        target_path: "crate_c::NonExistent".to_string(),
        target_unit: Some("crate_c-0.1.0".to_string()),
        target_file: None,
        target_line: 0,
        target_col: 0,
        kind: DefKind::Struct,
        is_write: false,
        is_macro_expansion: false,
    });

    // Ref 4: references external unknown crate (find_cross_definition returns None -> falls through to line 188)
    writer_ref.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 8,
        start_col: 1,
        end_line: 8,
        end_col: 5,
        target_def_id: None,
        target_path: "unknown_crate::Missing".to_string(),
        target_unit: None,
        target_file: None,
        target_line: 0,
        target_col: 0,
        kind: DefKind::Struct,
        is_write: false,
        is_macro_expansion: false,
    });

    let p_ref = ws_dir.join("crate_ref.ridx");
    writer_ref
        .write_to(&mut BufWriter::new(File::create(&p_ref).unwrap()))
        .unwrap();

    let ws = WorkspaceIndex::open(&ws_dir).unwrap();

    // Query Ref 1 (line 5): disambiguation matches candidate by line 10
    let def1 = ws.find_definition_at("src/main.rs", 5, 2);
    assert!(def1.is_some());
    assert_eq!(def1.unwrap().start_line, 10);

    // Query Ref 2 (line 6): ambiguous cross-crate candidate returns raw reference fallback (start_line: 99)
    let def2 = ws.find_definition_at("src/main.rs", 6, 2);
    assert!(def2.is_some());
    assert_eq!(def2.unwrap().start_line, 99);

    // Query Ref 3 (line 7): target_unit specified with no def in it falls back to raw reference
    let def3 = ws.find_definition_at("src/main.rs", 7, 2);
    assert!(def3.is_some());
    assert_eq!(def3.unwrap().path, "crate_c::NonExistent");

    // Query Ref 4 (line 8): unknown external dependency falls through
    let def4 = ws.find_definition_at("src/main.rs", 8, 2);
    assert!(def4.is_some());
    assert_eq!(def4.unwrap().path, "unknown_crate::Missing");

    // find_references_for_unit with target_unit: skips unit mismatch
    let refs = ws.find_references_for_unit("crate_c::Item", Some("crate_c-0.1.0"));
    assert_eq!(refs.len(), 1);
}
