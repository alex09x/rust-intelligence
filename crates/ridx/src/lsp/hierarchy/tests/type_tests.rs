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
use crate::lsp::hierarchy::types::*;
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter, RawDefinition, RawReference, WorkspaceIndex};
use serde_json::json;
use std::fs;
use std::path::Path;

#[test]
fn test_type_hierarchy_full() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_hierarchy();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    // 1. Prepare type hierarchy on struct Foo
    let type_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 13 }
        }
    });
    let type_res = handle_prepare_type_hierarchy(&type_req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!type_res.as_array().unwrap().is_empty());

    // 2. Supertypes with and without workspace
    let super_req = json!({
        "params": {
            "item": { "name": "Foo" }
        }
    });
    let super_res = handle_type_hierarchy_supertypes(&super_req, Some(&ws));
    assert_eq!(super_res.as_array().unwrap().len(), 1);
    assert_eq!(handle_type_hierarchy_supertypes(&super_req, None), json!([]));

    // 3. Subtypes with and without workspace
    let sub_req = json!({
        "params": {
            "item": { "name": "Bar" }
        }
    });
    let sub_res = handle_type_hierarchy_subtypes(&sub_req, Some(&ws));
    assert_eq!(sub_res.as_array().unwrap().len(), 1);
    assert_eq!(handle_type_hierarchy_subtypes(&sub_req, None), json!([]));

    // 4. Prepare type hierarchy with overlay on a struct
    let mut ov_mgr = OverlayManager::new();
    ov_mgr.open_document(&uri, file_path.to_str().unwrap(), 1, "pub struct MyType;\n".to_string());
    let type_prep_ov = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 12 }
        }
    });
    let type_ov_res = handle_prepare_type_hierarchy(&type_prep_ov, Some(&ws), &ov_mgr, &mut conv).unwrap();
    assert!(!type_ov_res.as_array().unwrap().is_empty());

    // 5. Prepare type hierarchy on a function in workspace: returns empty
    let type_prep_fn = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 8 }
        }
    });
    let fn_type_res = handle_prepare_type_hierarchy(&type_prep_fn, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(fn_type_res.as_array().unwrap().is_empty());

    // 6. Prepare type hierarchy with overlay on a function: falls through to workspace
    ov_mgr.open_document(&uri, file_path.to_str().unwrap(), 2, "pub fn not_a_type() {}\n".to_string());
    let fn_type_ov_res = handle_prepare_type_hierarchy(&type_prep_fn, Some(&ws), &ov_mgr, &mut conv).unwrap();
    assert!(fn_type_ov_res.as_array().unwrap().is_empty());

    // 7. Prepare type hierarchy on whitespace: returns empty
    let type_prep_ws = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 0 }
        }
    });
    let ws_type_res = handle_prepare_type_hierarchy(&type_prep_ws, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(ws_type_res.as_array().unwrap().is_empty());

    // 8. Prepare type hierarchy without workspace
    assert_eq!(handle_prepare_type_hierarchy(&type_prep_fn, None, &overlay_mgr, &mut conv).unwrap(), json!([]));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_prepare_type_hierarchy_missing_params_rejected_code_32602() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_hierarchy();

    // 1. Missing params object entirely
    let req_no_params = json!({});
    let err1 = handle_prepare_type_hierarchy(&req_no_params, Some(&ws), &overlay_mgr, &mut conv).unwrap_err();
    assert_eq!(err1.code, -32602);

    // 2. Empty params object
    let req_empty_params = json!({ "params": {} });
    let err2 = handle_prepare_type_hierarchy(&req_empty_params, Some(&ws), &overlay_mgr, &mut conv).unwrap_err();
    assert_eq!(err2.code, -32602);

    let _ = fs::remove_dir_all(&temp_dir);
}

fn make_test_workspace_with_ref(temp_dir: &Path, crate_name: &str, src_code: &str, defs: Vec<RawDefinition>, refs: Vec<RawReference>) -> WorkspaceIndex {
    let src_file = temp_dir.join("src/lib.rs");
    if let Some(parent) = src_file.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(&src_file, src_code);

    let mut writer = IndexWriter::new(crate_name, "0.1.0");
    for def in defs {
        writer.add_definition(def);
    }
    for r in refs {
        writer.add_reference(r);
    }
    let ridx_file = temp_dir.join(format!("{}.ridx", crate_name));
    let mut out_file = fs::File::create(&ridx_file).unwrap();
    writer.write_to(&mut out_file).unwrap();
    WorkspaceIndex::open(&ridx_file).unwrap()
}

fn make_def(id: u32, kind: DefKind, name: &str, file: &str, line: u32) -> RawDefinition {
    RawDefinition {
        def_id: id,
        kind,
        visibility: 0,
        is_macro_expansion: false,
        file: file.to_string(),
        name: name.to_string(),
        start_line: line,
        start_col: 0,
        end_line: line,
        end_col: name.len() as u32,
        path: format!("test_pkg::{}", name),
        signature: None,
    }
}

fn make_ref(file: &str, line: u32, target_def_id: u32, target_path: &str) -> RawReference {
    RawReference {
        file: file.to_string(),
        start_line: line,
        start_col: 0,
        end_line: line,
        end_col: 10,
        target_def_id: Some(target_def_id),
        target_path: target_path.to_string(),
        target_unit: None,
        target_file: Some(file.to_string()),
        target_line: 1,
        target_col: 0,
        kind: DefKind::Struct,
        is_write: false,
        is_macro_expansion: false,
    }
}

#[test]
fn test_supertypes_impl_outside_window_matches_oracle() {
    let temp_dir =
        std::env::temp_dir().join(format!("ridx_test_window_{}_{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    let _ = fs::create_dir_all(&temp_dir);

    // Line 1: trait Display
    // Line 2: struct Foo
    // Line 3: struct Bar
    // Line 4: impl Clone for Bar (indexed impl -> src/lib.rs in file_ids)
    // Line 5: blank
    // Line 6: impl Display for Foo (unindexed text trait impl)
    // Line 7: fn fmt(&self) {}
    // Line 8: }
    // Line 9: blank
    // Line 10: // pad 1
    // Line 11: // pad 2
    // Line 12: // pad 3
    // Line 13: fn test_ref(x: Foo) {} (ref to Foo at line 13, distance 7 to line 6)
    let code = "\
pub trait Display { fn fmt(&self); }
pub struct Foo;
pub struct Bar;
impl Clone for Bar { fn clone(&self) -> Self { Bar } }

impl Display for Foo {
    fn fmt(&self) {}
}

// pad 1
// pad 2
// pad 3
fn test_ref(x: Foo) {}
";
    let src_path = temp_dir.join("src/lib.rs");
    let src_str = src_path.to_str().unwrap().to_string();

    let defs = vec![
        make_def(1, DefKind::Trait, "Display", &src_str, 1),
        make_def(2, DefKind::Struct, "Foo", &src_str, 2),
        make_def(3, DefKind::Struct, "Bar", &src_str, 3),
        make_def(4, DefKind::Impl, "<Bar as Clone>", &src_str, 4),
    ];

    let refs = vec![make_ref(&src_str, 13, 2, "test_pkg::Foo")];

    let ws = make_test_workspace_with_ref(&temp_dir, "test_pkg", code, defs, refs);

    let req = json!({
        "params": {
            "item": { "name": "Foo" }
        }
    });

    let handle_res = handle_type_hierarchy_supertypes(&req, Some(&ws));
    let baseline_res = baseline_handle_type_hierarchy_supertypes(&req, Some(&ws));

    assert_eq!(handle_res, baseline_res);
    assert_eq!(handle_res, json!([]));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_supertypes_inherent_impl_blocks_trait_impl_matches_oracle() {
    let temp_dir = std::env::temp_dir().join(format!(
        "ridx_test_inherent_block_{}_{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    let _ = fs::create_dir_all(&temp_dir);

    // Line 1: trait Display
    // Line 2: struct Foo
    // Line 3: struct Bar
    // Line 4: impl Clone for Bar (indexed impl -> src/lib.rs in file_ids)
    // Line 5: blank
    // Line 6: impl Display for Foo { fn fmt(&self) {} } (unindexed trait impl)
    // Line 7: blank
    // Line 8: impl Foo { (inherent impl, blocks backward scan at line 9)
    // Line 9:     fn new() -> Foo { Foo }
    // Line 10: }
    let code = "\
pub trait Display { fn fmt(&self); }
pub struct Foo;
pub struct Bar;
impl Clone for Bar { fn clone(&self) -> Self { Bar } }

impl Display for Foo { fn fmt(&self) {} }

impl Foo {
    fn new() -> Foo { Foo }
}
";
    let src_path = temp_dir.join("src/lib.rs");
    let src_str = src_path.to_str().unwrap().to_string();

    let defs = vec![
        make_def(1, DefKind::Trait, "Display", &src_str, 1),
        make_def(2, DefKind::Struct, "Foo", &src_str, 2),
        make_def(3, DefKind::Struct, "Bar", &src_str, 3),
        make_def(4, DefKind::Impl, "<Bar as Clone>", &src_str, 4),
    ];

    let refs = vec![make_ref(&src_str, 9, 2, "test_pkg::Foo")];

    let ws = make_test_workspace_with_ref(&temp_dir, "test_pkg", code, defs, refs);

    let req = json!({
        "params": {
            "item": { "name": "Foo" }
        }
    });

    let handle_res = handle_type_hierarchy_supertypes(&req, Some(&ws));
    let baseline_res = baseline_handle_type_hierarchy_supertypes(&req, Some(&ws));

    assert_eq!(handle_res, baseline_res);
    assert_eq!(handle_res, json!([]));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_supertypes_derive_impl_on_non_impl_line_does_not_block_oracle() {
    let temp_dir = std::env::temp_dir().join(format!(
        "ridx_test_derive_block_{}_{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    let _ = fs::create_dir_all(&temp_dir);

    // Line 1: trait TraitA
    // Line 2: struct Foo
    // Line 3: blank
    // Line 4: impl TraitA for Foo { fn foo(&self) {} } (unindexed trait impl)
    // Line 5: blank
    // Line 6: #[derive(Clone)] (indexed DefKind::Impl with span at line 6, but text does NOT start with "impl")
    // Line 7: pub struct Bar;
    // Line 8: fn test_ref(x: Foo) {} (ref to Foo at line 8, distance 4 to line 4)
    let code = "\
pub trait TraitA { fn foo(&self); }
pub struct Foo;

impl TraitA for Foo { fn foo(&self) {} }

#[derive(Clone)]
pub struct Bar;
fn test_ref(x: Foo) {}
";
    let src_path = temp_dir.join("src/lib.rs");
    let src_str = src_path.to_str().unwrap().to_string();

    let defs = vec![
        make_def(1, DefKind::Trait, "TraitA", &src_str, 1),
        make_def(2, DefKind::Struct, "Foo", &src_str, 2),
        make_def(3, DefKind::Struct, "Bar", &src_str, 7),
        make_def(4, DefKind::Impl, "<Bar as Clone>", &src_str, 6),
    ];

    let refs = vec![make_ref(&src_str, 8, 2, "test_pkg::Foo")];

    let ws = make_test_workspace_with_ref(&temp_dir, "test_pkg", code, defs, refs);

    let req = json!({
        "params": {
            "item": { "name": "Foo" }
        }
    });

    let handle_res = handle_type_hierarchy_supertypes(&req, Some(&ws));
    let baseline_res = baseline_handle_type_hierarchy_supertypes(&req, Some(&ws));

    assert_eq!(handle_res, baseline_res);
    assert_eq!(handle_res.as_array().unwrap().len(), 1);
    assert_eq!(handle_res[0]["name"], "TraitA");

    let _ = fs::remove_dir_all(&temp_dir);
}
