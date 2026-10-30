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

#[test]
fn test_navigation_references_and_candidate_resolution() {
    let test_dir = new_test_dir("ws_nav_refs_test");
    let target_file1 = test_dir.0.join("t1.ridx");
    let target_file2 = test_dir.0.join("t2.ridx");
    let consumer_file = test_dir.0.join("cons.ridx");

    // Target unit 1
    let mut w1 = IndexWriter::new("target_krate", "0.1.0");
    w1.set_unit_key("unit_1");
    w1.set_generation("gen_1");
    w1.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "target_fn".to_string(),
        path: "target_krate::target_fn".to_string(),
        signature: Some("fn target_fn()".to_string()),
        file: "src/target.rs".to_string(),
        start_line: 10,
        start_col: 1,
        end_line: 10,
        end_col: 20,
    });
    let mut f1 = File::create(&target_file1).unwrap();
    w1.write_to(&mut f1).unwrap();
    drop(f1);

    // Target unit 2 (same crate, different unit)
    let mut w2 = IndexWriter::new("target_krate", "0.2.0");
    w2.set_unit_key("unit_2");
    w2.set_generation("gen_2");
    w2.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "target_fn".to_string(),
        path: "target_krate::target_fn".to_string(),
        signature: Some("fn target_fn()".to_string()),
        file: "src/target_alt.rs".to_string(),
        start_line: 10,
        start_col: 1,
        end_line: 10,
        end_col: 20,
    });
    // Same crate also has an internal reference to target_fn
    w2.add_reference(RawReference {
        file: "src/target_alt.rs".to_string(),
        start_line: 25,
        start_col: 5,
        end_line: 25,
        end_col: 15,
        target_def_id: Some(1),
        target_path: "target_krate::target_fn".to_string(),
        target_unit: None,
        target_file: Some("src/target_alt.rs".to_string()),
        target_line: 10,
        target_col: 0,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    let mut f2 = File::create(&target_file2).unwrap();
    w2.write_to(&mut f2).unwrap();
    drop(f2);

    // Consumer crate referencing target_krate directly and via alias
    let mut wc = IndexWriter::new("consumer_krate", "0.1.0");
    wc.set_unit_key("unit_consumer");
    wc.set_generation("gen_consumer");
    // Ref with mismatched file (lines 519-525)
    wc.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 30,
        start_col: 1,
        end_line: 30,
        end_col: 10,
        target_def_id: None,
        target_path: "target_krate::target_fn".to_string(),
        target_unit: None,
        target_file: Some("mismatched_file.rs".to_string()),
        target_line: 10,
        target_col: 0,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    // Ref via alias with mismatched target unit (lines 562-564)
    wc.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 31,
        start_col: 1,
        end_line: 31,
        end_col: 10,
        target_def_id: None,
        target_path: "t_alias::target_fn".to_string(),
        target_unit: Some("wrong_unit".to_string()),
        target_file: Some("src/target.rs".to_string()),
        target_line: 10,
        target_col: 0,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    // Ref via alias with mismatched line (lines 571-573)
    wc.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 32,
        start_col: 1,
        end_line: 32,
        end_col: 10,
        target_def_id: None,
        target_path: "t_alias::target_fn".to_string(),
        target_unit: None,
        target_file: Some("src/target.rs".to_string()),
        target_line: 99,
        target_col: 0,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    // Ref via alias with mismatched file (lines 581-585)
    wc.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 33,
        start_col: 1,
        end_line: 33,
        end_col: 10,
        target_def_id: None,
        target_path: "t_alias::target_fn".to_string(),
        target_unit: None,
        target_file: Some("unrelated.rs".to_string()),
        target_line: 10,
        target_col: 0,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    // Ref via alias that matches! (lines 587-603)
    wc.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 34,
        start_col: 1,
        end_line: 34,
        end_col: 10,
        target_def_id: None,
        target_path: "t_alias::target_fn".to_string(),
        target_unit: None,
        target_file: Some("src/target.rs".to_string()),
        target_line: 10,
        target_col: 0,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });

    // Ambiguous ref: matching line and file ends_with target_alt.rs (lines 288-291)
    wc.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 35,
        start_col: 1,
        end_line: 35,
        end_col: 10,
        target_def_id: None,
        target_path: "target_krate::target_fn".to_string(),
        target_unit: None,
        target_file: Some("src/target_alt.rs".to_string()),
        target_line: 10,
        target_col: 0,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    // Ambiguous ref: matching line only (lines 293-296)
    wc.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 36,
        start_col: 1,
        end_line: 36,
        end_col: 10,
        target_def_id: None,
        target_path: "target_krate::target_fn".to_string(),
        target_unit: None,
        target_file: Some("unmatched.rs".to_string()),
        target_line: 10,
        target_col: 0,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    // Ambiguous ref: neither line nor file matching, first candidate fallback (line 298)
    wc.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 37,
        start_col: 1,
        end_line: 37,
        end_col: 10,
        target_def_id: None,
        target_path: "target_krate::target_fn".to_string(),
        target_unit: None,
        target_file: Some("unmatched.rs".to_string()),
        target_line: 999,
        target_col: 0,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    // Target unit found but symbol does not exist -> target unit fallback (lines 300-302)
    wc.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 38,
        start_col: 1,
        end_line: 38,
        end_col: 10,
        target_def_id: None,
        target_path: "t_alias::nonexistent_symbol".to_string(),
        target_unit: Some("unit_1".to_string()),
        target_file: Some("src/target.rs".to_string()),
        target_line: 10,
        target_col: 0,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    // Target def file mismatch in direct references loop (lines 520-527)
    wc.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 45,
        start_col: 1,
        end_line: 45,
        end_col: 10,
        target_def_id: None,
        target_path: "target_krate::target_fn".to_string(),
        target_unit: None,
        target_file: Some("src/mismatched_file.rs".to_string()),
        target_line: 10,
        target_col: 0,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    // Target def file mismatch in alias references loop (lines 580-590)
    wc.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 46,
        start_col: 1,
        end_line: 46,
        end_col: 10,
        target_def_id: None,
        target_path: "t_alias::target_fn".to_string(),
        target_unit: None,
        target_file: Some("src/mismatched_file.rs".to_string()),
        target_line: 10,
        target_col: 0,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    // Definition with signature: None (line 65)
    wc.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Mod,
        visibility: 0,
        is_macro_expansion: false,
        name: "no_sig_mod".to_string(),
        path: "consumer_krate::no_sig_mod".to_string(),
        signature: None,
        file: "src/main.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 5,
        end_col: 1,
    });

    let mut fc = File::create(&consumer_file).unwrap();
    wc.write_to(&mut fc).unwrap();
    drop(fc);

    let mut ws = WorkspaceIndex::open(&test_dir.0).unwrap();
    // Setup extern mapping for consumer: t_alias -> unit_1
    let cons_idx = ws.readers.iter().position(|r| r.crate_name() == "consumer_krate").unwrap();
    let mut ext_map = HashMap::new();
    ext_map.insert("t_alias".to_string(), "unit_1".to_string());
    ws.reader_extern_units[cons_idx] = ext_map;
    ws.reader_dependencies[cons_idx] = vec!["unit_1".to_string(), "unit_2".to_string()]; // hits lines 224-235

    let t2_idx = ws.readers.iter().position(|r| r.unit_key() == "unit_2").unwrap();
    ws.reader_dependencies[t2_idx] = vec!["unit_1".to_string()]; // hits lines 499-503

    // find_definition_by_path with signature None (line 65)
    let def_no_sig = ws.find_definition_by_path("consumer_krate::no_sig_mod").unwrap();
    assert_eq!(def_no_sig.signature, None);

    // Query references to target_krate::target_fn in unit_1
    // Reader for target unit 2 has reader_crate == target_crate but ru != target_unit (lines 499-503)
    let refs = ws.find_references_for_unit("target_krate::target_fn", Some("unit_1"));
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].span.start_line(), 34);

    // Hover with source_root set (lines 407-409)
    let t1_idx = ws.readers.iter().position(|r| r.unit_key() == "unit_1").unwrap();
    ws.source_roots[t1_idx] = Some(test_dir.0.clone());
    let hover = ws.hover_at("src/target.rs", 10, 5);
    assert!(hover.is_some());
    assert!(hover.unwrap().definition.is_some());

    // Target path starting with target_crate:: and mapping to r.crate_name() (lines 407-419)
    let mapped_refs = ws.find_references_for_unit("t_alias::target_fn", Some("unit_1"));
    assert!(!mapped_refs.is_empty());

    // Target def coords with no source root and no workspace root (line 396, 415)
    ws.source_roots[t1_idx] = None;
    let orig_ws_root = ws.workspace_root.take();
    let no_root_refs = ws.find_references_for_unit("target_krate::target_fn", Some("unit_1"));
    assert_eq!(no_root_refs.len(), 1);
    ws.workspace_root = orig_ws_root;

    // Query ambiguous references (lines 282-298)
    let def_alt = ws.find_definition_at("src/main.rs", 35, 5).unwrap();
    assert_eq!(def_alt.file, "src/target_alt.rs");

    let def_line_match = ws.find_definition_at("src/main.rs", 36, 5).unwrap();
    assert_eq!(def_line_match.start_line, 10);

    let def_fallback = ws.find_definition_at("src/main.rs", 37, 5).unwrap();
    assert_eq!(def_fallback.name, "target_fn");

    // Target unit not found fallback (lines 300-302)
    let def_unresolved = ws.find_definition_at("src/main.rs", 38, 5).unwrap();
    assert_eq!(def_unresolved.name, "nonexistent_symbol");

    // Definition with signature None (line 65)
    let d_no_sig = ws.find_definition_by_path("consumer_krate::no_sig_mod").unwrap();
    assert_eq!(d_no_sig.signature, None);

    // find_references_for_unit with target_unit_key None to trigger target_def_coords filtering (lines 504-528, 565-590)
    let coord_refs = ws.find_references_for_unit("target_krate::target_fn", None);
    assert!(!coord_refs.is_empty());

    // Target unit filtering in extern_map and reader_unit != target_unit (lines 500-502, 555)
    let refs_wrong_unit = ws.find_references_for_unit("target_krate::target_fn", Some("nonexistent_unit"));
    assert!(refs_wrong_unit.is_empty());
}
