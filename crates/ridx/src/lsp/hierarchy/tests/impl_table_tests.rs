/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::setup_test_hierarchy;
use crate::lsp::hierarchy::fallback_cache::FallbackCache;
use crate::lsp::hierarchy::impl_table::*;
use crate::lsp::hierarchy::types::*;
use ridx_format::{DefKind, IndexWriter, RawDefinition, WorkspaceIndex};
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

fn make_reader_file(temp_dir: &PathBuf, crate_name: &str, src_rel: &str, src_code: &str, defs: Vec<RawDefinition>) -> PathBuf {
    let src_file = temp_dir.join(src_rel);
    if let Some(parent) = src_file.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(&src_file, src_code);

    let mut writer = IndexWriter::new(crate_name, "0.1.0");
    for def in defs {
        writer.add_definition(def);
    }
    let ridx_file = temp_dir.join(format!("{}.ridx", crate_name));
    let mut out_file = fs::File::create(&ridx_file).unwrap();
    writer.write_to(&mut out_file).unwrap();
    ridx_file
}

#[test]
fn test_multiline_and_generics_impl() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_test_multiline_impl_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    let _ = fs::create_dir_all(&temp_dir);

    let code = r#"
pub trait Component<T> { fn get(&self) -> T; }
pub struct Position<T> { pub x: T }

impl<T: Clone + Send>
    Component<T>
    for
    Position<T>
{
    fn get(&self) -> T { self.x.clone() }
}
"#;
    let src_path = temp_dir.join("src/lib.rs");
    let src_str = src_path.to_str().unwrap().to_string();

    let defs = vec![
        RawDefinition {
            def_id: 1,
            kind: DefKind::Trait,
            visibility: 0,
            is_macro_expansion: false,
            file: src_str.clone(),
            name: "Component".to_string(),
            start_line: 2,
            start_col: 11,
            end_line: 2,
            end_col: 20,
            path: "test_pkg::Component".to_string(),
            signature: None,
        },
        RawDefinition {
            def_id: 2,
            kind: DefKind::Struct,
            visibility: 0,
            is_macro_expansion: false,
            file: src_str.clone(),
            name: "Position".to_string(),
            start_line: 3,
            start_col: 12,
            end_line: 3,
            end_col: 20,
            path: "test_pkg::Position".to_string(),
            signature: None,
        },
        RawDefinition {
            def_id: 3,
            kind: DefKind::Impl,
            visibility: 0,
            is_macro_expansion: false,
            file: src_str.clone(),
            name: "<Position as Component>".to_string(),
            start_line: 5,
            start_col: 1,
            end_line: 11,
            end_col: 2,
            path: "<Position as Component>".to_string(),
            signature: None,
        },
    ];

    make_reader_file(&temp_dir, "test_pkg", "src/lib.rs", code, defs);
    let ws = WorkspaceIndex::open(&temp_dir).unwrap();

    let table = get_or_build_impl_table(&ws);
    assert_eq!(table.impls.len(), 1);

    // 1. Supertypes of Position
    let mut super_seen = std::collections::HashSet::new();
    let mut super_results = Vec::new();
    table.find_supertypes(&ws, "Position", &mut super_seen, &mut super_results);
    assert_eq!(super_results.len(), 1);
    assert_eq!(super_results[0]["name"], "Component");

    // 2. Subtypes of Component
    let mut sub_seen = std::collections::HashSet::new();
    let mut sub_results = Vec::new();
    table.find_subtypes(&ws, "Component", &mut sub_seen, &mut sub_results);
    assert_eq!(sub_results.len(), 1);
    assert_eq!(sub_results[0]["name"], "Position");
}

#[test]
fn test_same_named_types_reader_order_and_baseline_equivalence() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_test_same_named_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    let _ = fs::create_dir_all(&temp_dir);

    // Crate 1: bevy_render
    let code_1 = "pub trait Render { fn draw(&self); }\npub struct Mesh;\nimpl Render for Mesh { fn draw(&self) {} }\n";
    let src_1 = temp_dir.join("crate1/src/lib.rs");
    let src_1_str = src_1.to_str().unwrap().to_string();
    let defs_1 = vec![
        RawDefinition {
            def_id: 1,
            kind: DefKind::Trait,
            visibility: 0,
            is_macro_expansion: false,
            file: src_1_str.clone(),
            name: "Render".to_string(),
            start_line: 1,
            start_col: 11,
            end_line: 1,
            end_col: 17,
            path: "bevy_render::Render".to_string(),
            signature: None,
        },
        RawDefinition {
            def_id: 2,
            kind: DefKind::Struct,
            visibility: 0,
            is_macro_expansion: false,
            file: src_1_str.clone(),
            name: "Mesh".to_string(),
            start_line: 2,
            start_col: 12,
            end_line: 2,
            end_col: 16,
            path: "bevy_render::Mesh".to_string(),
            signature: None,
        },
        RawDefinition {
            def_id: 3,
            kind: DefKind::Impl,
            visibility: 0,
            is_macro_expansion: false,
            file: src_1_str.clone(),
            name: "<Mesh as Render>".to_string(),
            start_line: 3,
            start_col: 1,
            end_line: 3,
            end_col: 42,
            path: "<Mesh as Render>".to_string(),
            signature: None,
        },
    ];
    make_reader_file(&temp_dir, "bevy_render", "crate1/src/lib.rs", code_1, defs_1);

    // Crate 2: bevy_ui
    let code_2 = "pub trait Render { fn draw_ui(&self); }\npub struct Mesh;\nimpl Render for Mesh { fn draw_ui(&self) {} }\n";
    let src_2 = temp_dir.join("crate2/src/lib.rs");
    let src_2_str = src_2.to_str().unwrap().to_string();
    let defs_2 = vec![
        RawDefinition {
            def_id: 1,
            kind: DefKind::Trait,
            visibility: 0,
            is_macro_expansion: false,
            file: src_2_str.clone(),
            name: "Render".to_string(),
            start_line: 1,
            start_col: 11,
            end_line: 1,
            end_col: 17,
            path: "bevy_ui::Render".to_string(),
            signature: None,
        },
        RawDefinition {
            def_id: 2,
            kind: DefKind::Struct,
            visibility: 0,
            is_macro_expansion: false,
            file: src_2_str.clone(),
            name: "Mesh".to_string(),
            start_line: 2,
            start_col: 12,
            end_line: 2,
            end_col: 16,
            path: "bevy_ui::Mesh".to_string(),
            signature: None,
        },
        RawDefinition {
            def_id: 3,
            kind: DefKind::Impl,
            visibility: 0,
            is_macro_expansion: false,
            file: src_2_str.clone(),
            name: "<Mesh as Render>".to_string(),
            start_line: 3,
            start_col: 1,
            end_line: 3,
            end_col: 45,
            path: "<Mesh as Render>".to_string(),
            signature: None,
        },
    ];
    make_reader_file(&temp_dir, "bevy_ui", "crate2/src/lib.rs", code_2, defs_2);

    let ws = WorkspaceIndex::open(&temp_dir).unwrap();
    assert_eq!(ws.readers().len(), 2);

    let sub_req = json!({
        "params": { "item": { "name": "Render" } }
    });
    let super_req = json!({
        "params": { "item": { "name": "Mesh" } }
    });

    // Case 1: Oracle is called before handler
    let expected_sub_1 = baseline_handle_type_hierarchy_subtypes(&sub_req, Some(&ws));
    let actual_sub_1 = handle_type_hierarchy_subtypes(&sub_req, Some(&ws));
    assert_eq!(actual_sub_1, expected_sub_1, "Subtypes output must match baseline oracle when oracle runs first");

    let expected_super_1 = baseline_handle_type_hierarchy_supertypes(&super_req, Some(&ws));
    let actual_super_1 = handle_type_hierarchy_supertypes(&super_req, Some(&ws));
    assert_eq!(actual_super_1, expected_super_1, "Supertypes output must match baseline oracle when oracle runs first");

    // Case 2: On fresh workspace, handler is called before oracle
    let ws2 = WorkspaceIndex::open(&temp_dir).unwrap();
    let actual_sub_2 = handle_type_hierarchy_subtypes(&sub_req, Some(&ws2));
    let expected_sub_2 = baseline_handle_type_hierarchy_subtypes(&sub_req, Some(&ws2));
    assert_eq!(actual_sub_2, expected_sub_2, "Subtypes output must match baseline oracle when handler runs first");

    let actual_super_2 = handle_type_hierarchy_supertypes(&super_req, Some(&ws2));
    let expected_super_2 = baseline_handle_type_hierarchy_supertypes(&super_req, Some(&ws2));
    assert_eq!(actual_super_2, expected_super_2, "Supertypes output must match baseline oracle when handler runs first");
}

#[test]
fn test_oracle_does_not_populate_cache() {
    let (_temp_dir, ws, _, _) = setup_test_hierarchy();
    let sub_req = json!({
        "params": { "item": { "name": "Render" } }
    });
    let super_req = json!({
        "params": { "item": { "name": "Mesh" } }
    });

    let _ = baseline_handle_type_hierarchy_subtypes(&sub_req, Some(&ws));
    let _ = baseline_handle_type_hierarchy_supertypes(&super_req, Some(&ws));

    let table = get_or_build_impl_table(&ws);
    assert_eq!(table.get_fallback_results("Render", false), None);
    assert_eq!(table.get_fallback_results("Mesh", true), None);
}

#[test]
fn test_cache_invalidation_on_generation_bump() {
    let (_temp_dir, mut ws, _, _) = setup_test_hierarchy();

    let table1 = get_or_build_impl_table(&ws);
    let table2 = get_or_build_impl_table(&ws);
    assert!(Arc::ptr_eq(&table1, &table2), "Same generation must return cached Arc");

    // Bump generation
    let new_gen = ws.bump_generation();
    assert_eq!(new_gen, 2);

    let table3 = get_or_build_impl_table(&ws);
    assert!(!Arc::ptr_eq(&table1, &table3), "Bumped generation must re-build and return new Arc");

    // Clear cache
    clear_impl_table_cache();
    let table4 = get_or_build_impl_table(&ws);
    assert!(!Arc::ptr_eq(&table3, &table4), "Cleared cache must re-build and return new Arc");
}

#[test]
fn test_fallback_cache_memoization() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file = temp_dir.path().join("test.rs");
    fs::write(&file, "line 1\nimpl Foo for Bar {}\nline 3\n").unwrap();
    let file_str = file.to_str().unwrap();

    let cache = FallbackCache::new();

    // First query for line 2 (impl line) -> miss, parses file
    let res1 = cache.get_or_parse(file_str, 2, (None, None));
    assert_eq!(res1, (Some("Foo".to_string()), Some("Bar".to_string())));

    // Second query for line 2 -> hit from cache
    let res2 = cache.get_or_parse(file_str, 2, (None, None));
    assert_eq!(res2, res1);

    // Query for line 1 (not an impl) -> negative cache hit
    let res_neg1 = cache.get_or_parse(file_str, 1, (None, None));
    assert_eq!(res_neg1, (None, None));

    // Second query for line 1 -> negative cache hit without re-parsing
    let res_neg2 = cache.get_or_parse(file_str, 1, (None, None));
    assert_eq!(res_neg2, (None, None));
}
