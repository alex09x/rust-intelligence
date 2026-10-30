/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

mod candidates;
mod disambiguation;
mod references;
mod trait_impl_refs_tests;

use super::*;
use crate::format::DefKind;
use crate::writer::{IndexWriter, RawDefinition, RawReference, RawType};
use std::collections::HashMap;
use std::fs::{self, File};

struct TestDir(PathBuf);
impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn new_test_dir(prefix: &str) -> TestDir {
    static CNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let id = CNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!("{}_{}_{}", prefix, std::process::id(), id));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    TestDir(path)
}

#[test]
fn test_navigation_comprehensive() {
    let test_dir = new_test_dir("ridx_nav_test");
    let index_path = test_dir.0.join("sample.ridx");

    let mut writer = IndexWriter::new("sample", "0.1.0");
    writer.set_unit_key("unit1");

    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "sample_fn".to_string(),
        path: "sample::sample_fn".to_string(),
        signature: Some("fn()".to_string()),
        file: "src/lib.rs".to_string(),
        start_line: 10,
        start_col: 1,
        end_line: 12,
        end_col: 2,
    });

    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Local,
        visibility: 0,
        is_macro_expansion: false,
        name: "local_var".to_string(),
        path: "sample::sample_fn::local_var".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 11,
        start_col: 5,
        end_line: 11,
        end_col: 14,
    });

    writer.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 20,
        start_col: 5,
        end_line: 20,
        end_col: 14,
        target_def_id: Some(1),
        target_path: "sample::sample_fn".to_string(),
        target_unit: Some("unit1".to_string()),
        target_file: Some("src/lib.rs".to_string()),
        target_line: 10,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });

    writer.add_reference(RawReference {
        file: "src/lib.rs".to_string(),
        start_line: 11,
        start_col: 20,
        end_line: 11,
        end_col: 29,
        target_def_id: Some(2),
        target_path: "local_var".to_string(),
        target_unit: None,
        target_file: Some("src/lib.rs".to_string()),
        target_line: 11,
        target_col: 5,
        kind: DefKind::Local,
        is_write: false,
        is_macro_expansion: false,
    });

    writer.add_type(RawType {
        file: "src/lib.rs".to_string(),
        start_line: 10,
        start_col: 1,
        end_line: 10,
        end_col: 9,
        type_name: "fn()".to_string(),
    });

    let mut file = File::create(&index_path).unwrap();
    writer.write_to(&mut file).unwrap();

    let ws = WorkspaceIndex::open(&test_dir.0).unwrap();

    // 1. find_definition_by_path
    let def_prefixed = ws.find_definition_by_path("sample::sample_fn");
    assert!(def_prefixed.is_some());
    assert_eq!(def_prefixed.unwrap().name, "sample_fn");

    let def_unprefixed = ws.find_definition_by_path("sample_fn");
    assert!(def_unprefixed.is_none());

    // 2. crate_source_root
    assert!(ws.crate_source_root("sample").is_none());
    assert!(ws.crate_source_root("non_existent").is_none());

    // 3. resolve_path_for_crate
    let abs = ws.resolve_path_for_crate("sample", "/abs/path.rs");
    assert_eq!(abs, PathBuf::from("/abs/path.rs"));

    let rel = ws.resolve_path_for_crate("sample", "src/lib.rs");
    assert!(rel.ends_with("src/lib.rs"));

    // 4. find_definition_at
    let def_at_def = ws.find_definition_at("src/lib.rs", 10, 2);
    assert!(def_at_def.is_some());
    assert_eq!(def_at_def.unwrap().name, "sample_fn");

    let def_at_ref = ws.find_definition_at("src/main.rs", 20, 6);
    assert!(def_at_ref.is_some());
    assert_eq!(def_at_ref.unwrap().name, "sample_fn");

    let def_none = ws.find_definition_at("src/lib.rs", 99, 99);
    assert!(def_none.is_none());

    // 5. find_references and find_references_for_unit
    let refs = ws.find_references("sample::sample_fn");
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].file, "src/main.rs");

    // 6. type_at
    assert_eq!(ws.type_at("src/lib.rs", 10, 2), Some("fn()"));
    assert_eq!(ws.type_at("src/lib.rs", 99, 99), None);

    // 7. search_symbols
    let syms = ws.search_symbols("sample_fn");
    assert_eq!(syms.len(), 2);
    assert_eq!(syms[0].name, "sample_fn");

    // 8. find_references_at
    let ref_at = ws.find_references_at("src/lib.rs", 10, 2);
    assert!(ref_at.is_some());
    assert_eq!(ref_at.unwrap().references.len(), 1);

    let local_ref_at = ws.find_references_at("src/lib.rs", 11, 6);
    assert!(local_ref_at.is_some());
    assert_eq!(local_ref_at.unwrap().references.len(), 1);

    assert!(ws.find_references_at("src/lib.rs", 99, 99).is_none());

    // 9. hover_at
    let hover = ws.hover_at("src/lib.rs", 10, 2);
    assert!(hover.is_some());
    let h = hover.unwrap();
    assert!(h.definition.is_some());
    assert_eq!(h.inferred_type, Some("fn()"));

    assert!(ws.hover_at("src/lib.rs", 99, 99).is_none());

    // 10. find_references_for_unit
    let refs_scoped = ws.find_references_for_unit("sample::sample_fn", Some("unit1"));
    assert_eq!(refs_scoped.len(), 1);

    let refs_scoped_none = ws.find_references_for_unit("sample::sample_fn", Some("other_unit"));
    assert_eq!(refs_scoped_none.len(), 0);

    // 11. find_definition_by_path
    assert!(ws.find_definition_by_path("sample::sample_fn").is_some());
    assert!(ws.find_definition_by_path("non_existent").is_none());
}

#[test]
fn test_cross_crate_navigation() {
    let test_dir = new_test_dir("ridx_cross_nav");

    // Crate 1: dep_crate
    let mut dep_writer = IndexWriter::new("dep_crate", "0.1.0");
    dep_writer.set_unit_key("dep_unit");
    dep_writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "dep_func".to_string(),
        path: "dep_crate::dep_func".to_string(),
        signature: Some("pub fn dep_func()".to_string()),
        file: "src/dep.rs".to_string(),
        start_line: 5,
        start_col: 1,
        end_line: 7,
        end_col: 2,
    });
    let mut f1 = File::create(test_dir.0.join("dep_crate.ridx")).unwrap();
    dep_writer.write_to(&mut f1).unwrap();

    // Crate 2: app_crate
    let mut app_writer = IndexWriter::new("app_crate", "0.1.0");
    app_writer.set_unit_key("app_unit");
    app_writer.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 15,
        start_col: 5,
        end_line: 15,
        end_col: 13,
        target_def_id: None,
        target_path: "dep_crate::dep_func".to_string(),
        target_unit: Some("dep_unit".to_string()),
        target_file: None,
        target_line: 0,
        target_col: 0,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    let mut f2 = File::create(test_dir.0.join("app_crate.ridx")).unwrap();
    app_writer.write_to(&mut f2).unwrap();

    let ws = WorkspaceIndex::open(&test_dir.0).unwrap();
    let def = ws.find_definition_at("src/main.rs", 15, 6);
    assert!(def.is_some());
    let d = def.unwrap();
    assert_eq!(d.name, "dep_func");
    assert_eq!(d.file, "src/dep.rs");
}

#[test]
fn test_cross_crate_navigation_candidate_disambiguation() {
    let test_dir = new_test_dir("ridx_cross_disambig");

    let mut dep1 = IndexWriter::new("multi_crate", "0.1.0");
    dep1.set_unit_key("multi_unit_1");
    dep1.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "target_func".to_string(),
        path: "multi_crate::target_func".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 10,
        start_col: 1,
        end_line: 10,
        end_col: 20,
    });
    let mut f1 = File::create(test_dir.0.join("multi_crate_1.ridx")).unwrap();
    dep1.write_to(&mut f1).unwrap();

    let mut dep2 = IndexWriter::new("multi_crate", "0.2.0");
    dep2.set_unit_key("multi_unit_2");
    dep2.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "target_func".to_string(),
        path: "multi_crate::target_func".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 50,
        start_col: 1,
        end_line: 50,
        end_col: 20,
    });
    let mut f2 = File::create(test_dir.0.join("multi_crate_2.ridx")).unwrap();
    dep2.write_to(&mut f2).unwrap();

    let mut caller = IndexWriter::new("caller_crate", "0.1.0");
    caller.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 5,
        start_col: 5,
        end_line: 5,
        end_col: 16,
        target_def_id: None,
        target_path: "multi_crate::target_func".to_string(),
        target_unit: None,
        target_file: Some("src/lib.rs".to_string()),
        target_line: 50,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    let mut f3 = File::create(test_dir.0.join("caller_crate.ridx")).unwrap();
    caller.write_to(&mut f3).unwrap();

    let ws = WorkspaceIndex::open(&test_dir.0).unwrap();
    let def = ws.find_definition_at("src/main.rs", 5, 6);
    assert!(def.is_some());
    let d = def.unwrap();
    assert_eq!(d.start_line, 50);
}
