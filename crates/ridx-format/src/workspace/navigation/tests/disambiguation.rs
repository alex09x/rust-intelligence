/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::super::*;
use super::*;

// Unix: the fixture's paths are Unix absolute paths; on Windows they gain a drive.
#[cfg(unix)]
#[test]
fn test_navigation_edge_cases() {
    let test_dir = new_test_dir("ridx_nav_edges");

    let mut w = IndexWriter::new("edge_crate", "0.1.0");
    w.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "bare_fn".to_string(),
        path: "bare_fn".to_string(),
        signature: Some("fn bare_fn()".to_string()),
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 15,
    });
    let mut f = File::create(test_dir.0.join("edge.ridx")).unwrap();
    w.write_to(&mut f).unwrap();

    let ws = WorkspaceIndex::open(&test_dir.0).unwrap();

    // 1. find_definition_by_path with bare path
    let def = ws.find_definition_by_path("bare_fn");
    assert!(def.is_some());
    assert_eq!(def.unwrap().name, "bare_fn");

    // 2. resolve_path_for_crate
    let abs = Path::new("/abs/path/to/file.rs");
    assert_eq!(ws.resolve_path_for_crate("edge_crate", abs.to_str().unwrap()), abs);
    let rel_res = ws.resolve_path_for_crate("edge_crate", "src/lib.rs");
    assert!(rel_res.ends_with("src/lib.rs"));

    // 3. crate_source_root
    assert!(ws.crate_source_root("nonexistent").is_none());

    // 4. find_references_for_unit
    let refs = ws.find_references_for_unit("bare_fn", Some("nonexistent_unit"));
    assert!(refs.is_empty());
}

#[test]
fn test_cross_crate_navigation_disambiguation_and_coords() {
    let test_dir = new_test_dir("ridx_nav_disambig");

    // Crate 1: caller
    let mut w1 = IndexWriter::new("caller", "0.1.0");
    w1.set_unit_key("caller:0.1.0");
    w1.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 10,
        start_col: 5,
        end_line: 10,
        end_col: 15,
        target_def_id: None,
        target_path: "dep::target_fn".to_string(),
        target_unit: Some("dep:0.1.0".to_string()),
        target_file: Some("src/lib.rs".to_string()),
        target_line: 20,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });

    // Crate 2: dep (two versions or candidates)
    let mut w2 = IndexWriter::new("dep", "0.1.0");
    w2.set_unit_key("dep:0.1.0");
    w2.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "target_fn".to_string(),
        path: "dep::target_fn".to_string(),
        signature: Some("fn target_fn()".to_string()),
        file: "src/lib.rs".to_string(),
        start_line: 20,
        start_col: 1,
        end_line: 20,
        end_col: 15,
    });

    let mut w3 = IndexWriter::new("dep", "0.2.0");
    w3.set_unit_key("dep:0.2.0");
    w3.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "target_fn".to_string(),
        path: "dep::target_fn".to_string(),
        signature: Some("fn target_fn_v2()".to_string()),
        file: "src/lib.rs".to_string(),
        start_line: 25,
        start_col: 1,
        end_line: 25,
        end_col: 15,
    });

    let mut f1 = File::create(test_dir.0.join("caller.ridx")).unwrap();
    w1.write_to(&mut f1).unwrap();
    drop(f1);

    let mut f2 = File::create(test_dir.0.join("dep1.ridx")).unwrap();
    w2.write_to(&mut f2).unwrap();
    drop(f2);

    let mut f3 = File::create(test_dir.0.join("dep2.ridx")).unwrap();
    w3.write_to(&mut f3).unwrap();
    drop(f3);

    let ws = WorkspaceIndex::open(&test_dir.0).unwrap();

    // 1. Definition at cursor in caller resolving to exact dep unit
    let def = ws.find_definition_at("src/main.rs", 10, 8);
    assert!(def.is_some());
    let d = def.unwrap();
    assert_eq!(d.name, "target_fn");
    assert_eq!(d.signature, Some("fn target_fn()"));

    // 2. find_references for dep::target_fn
    let refs = ws.find_references("dep::target_fn");
    assert_eq!(refs.len(), 1);

    // 3. find_references_for_unit with exact matching unit
    let refs_exact = ws.find_references_for_unit("dep::target_fn", Some("dep:0.1.0"));
    assert_eq!(refs_exact.len(), 1);

    // 4. find_references_for_unit with non-matching unit
    let refs_none = ws.find_references_for_unit("dep::target_fn", Some("dep:0.2.0"));
    assert_eq!(refs_none.len(), 0);

    // 5. type_at
    let ty = ws.type_at("src/main.rs", 10, 8);
    assert_eq!(ty, None);
}
