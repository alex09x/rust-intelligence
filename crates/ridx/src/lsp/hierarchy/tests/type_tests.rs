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
use serde_json::json;
use std::fs;

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
    let type_res =
        handle_prepare_type_hierarchy(&type_req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!type_res.as_array().unwrap().is_empty());

    // 2. Supertypes with and without workspace
    let super_req = json!({
        "params": {
            "item": { "name": "Foo" }
        }
    });
    let super_res = handle_type_hierarchy_supertypes(&super_req, Some(&ws));
    assert_eq!(super_res.as_array().unwrap().len(), 1);
    assert_eq!(
        handle_type_hierarchy_supertypes(&super_req, None),
        json!([])
    );

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
    ov_mgr.open_document(
        &uri,
        file_path.to_str().unwrap(),
        1,
        "pub struct MyType;\n".to_string(),
    );
    let type_prep_ov = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 12 }
        }
    });
    let type_ov_res =
        handle_prepare_type_hierarchy(&type_prep_ov, Some(&ws), &ov_mgr, &mut conv).unwrap();
    assert!(!type_ov_res.as_array().unwrap().is_empty());

    // 5. Prepare type hierarchy on a function in workspace: returns empty
    let type_prep_fn = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 8 }
        }
    });
    let fn_type_res =
        handle_prepare_type_hierarchy(&type_prep_fn, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(fn_type_res.as_array().unwrap().is_empty());

    // 6. Prepare type hierarchy with overlay on a function: falls through to workspace
    ov_mgr.open_document(
        &uri,
        file_path.to_str().unwrap(),
        2,
        "pub fn not_a_type() {}\n".to_string(),
    );
    let fn_type_ov_res =
        handle_prepare_type_hierarchy(&type_prep_fn, Some(&ws), &ov_mgr, &mut conv).unwrap();
    assert!(fn_type_ov_res.as_array().unwrap().is_empty());

    // 7. Prepare type hierarchy on whitespace: returns empty
    let type_prep_ws = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 0 }
        }
    });
    let ws_type_res =
        handle_prepare_type_hierarchy(&type_prep_ws, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(ws_type_res.as_array().unwrap().is_empty());

    // 8. Prepare type hierarchy without workspace
    assert_eq!(
        handle_prepare_type_hierarchy(&type_prep_fn, None, &overlay_mgr, &mut conv).unwrap(),
        json!([])
    );

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_prepare_type_hierarchy_missing_params_rejected_code_32602() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_hierarchy();

    // 1. Missing params object entirely
    let req_no_params = json!({});
    let err1 = handle_prepare_type_hierarchy(&req_no_params, Some(&ws), &overlay_mgr, &mut conv)
        .unwrap_err();
    assert_eq!(err1.code, -32602);

    // 2. Empty params object
    let req_empty_params = json!({ "params": {} });
    let err2 = handle_prepare_type_hierarchy(&req_empty_params, Some(&ws), &overlay_mgr, &mut conv)
        .unwrap_err();
    assert_eq!(err2.code, -32602);

    let _ = fs::remove_dir_all(&temp_dir);
}
