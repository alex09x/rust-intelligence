/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::setup_test_intelligence;
use crate::lsp::intelligence::*;
use serde_json::json;
use std::fs;

#[test]
fn test_hover_clean_and_overlay() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_intelligence();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    // 1. Workspace hover hit (clean buffer)
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 10 }
        }
    });
    let h_res = handle_hover(&req, Some(&ws), &overlay_mgr, &mut conv);
    assert!(!h_res.is_null());
    assert!(h_res["contents"]["value"]
        .as_str()
        .unwrap()
        .contains("calculate"));

    // 2. Overlay hover hit (dirty buffer)
    overlay_mgr.open_document(
        &uri,
        file_path.to_str().unwrap(),
        1,
        "fn local_work() -> bool { true }\n".to_string(),
    );
    overlay_mgr.update(
        file_path.to_str().unwrap(),
        2,
        "// dirty\nfn local_work() -> bool { true }\n".to_string(),
    );
    let req_ov = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 4 }
        }
    });
    let h_ov = handle_hover(&req_ov, Some(&ws), &overlay_mgr, &mut conv);
    assert!(!h_ov.is_null());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_completion_scoping() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_intelligence();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    // Completion with nested function: should not leak nested item to sibling
    let code = "fn first() {\n    fn helper() {}\n    hel\n}\nfn second() {\n    hel\n}\n";
    overlay_mgr.open_document(&uri, file_path.to_str().unwrap(), 1, code.to_string());

    // In first(), helper() should be suggested
    let comp_in_first = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 2, "character": 7 }
        }
    });
    let res_first = handle_completion(&comp_in_first, Some(&ws), &overlay_mgr, &mut conv);
    assert!(res_first["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|i| i["label"] == "helper"));

    // In second(), helper() should NOT be suggested
    let comp_in_second = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 5, "character": 7 }
        }
    });
    let res_second = handle_completion(&comp_in_second, Some(&ws), &overlay_mgr, &mut conv);
    assert!(!res_second["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|i| i["label"] == "helper"));

    // Completion with empty overlay manager (disk fallback)
    let empty_ov = crate::overlay::OverlayManager::new();
    let res_disk = handle_completion(&comp_in_first, Some(&ws), &empty_ov, &mut conv);
    assert!(!res_disk["items"].as_array().unwrap().is_empty());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_signature_help_multiline() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_intelligence();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    let call_code = "fn test() {\n    calculate(\n        10,\n        20,\n    );\n}\n";
    overlay_mgr.open_document(&uri, file_path.to_str().unwrap(), 1, call_code.to_string());

    // Cursor at line 3, character 10 (on second argument `20`)
    let sig_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 3, "character": 10 }
        }
    });
    let sig_res = handle_signature_help(&sig_req, Some(&ws), &overlay_mgr, &mut conv);
    assert!(!sig_res.is_null());
    assert_eq!(sig_res["activeParameter"], 1);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_inlay_hints_and_inline() {
    let (temp_dir, ws, mut overlay_mgr, _conv) = setup_test_intelligence();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    let hint_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 2, "character": 0 }
            }
        }
    });
    let hints = handle_inlay_hint(&hint_req, Some(&ws), &overlay_mgr);
    assert!(hints.is_array());

    // Non-existent file
    let missing_hint = json!({
        "params": {
            "textDocument": { "uri": "file:///nonexistent/file.rs" },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 1, "character": 0 } }
        }
    });
    assert_eq!(
        handle_inlay_hint(&missing_hint, Some(&ws), &overlay_mgr),
        json!([])
    );

    // Inlay hints with inferred type on `let mut x = ...`
    let code_with_let = "   let calculate = 100;\n";
    overlay_mgr.open_document(
        &uri,
        file_path.to_str().unwrap(),
        1,
        code_with_let.to_string(),
    );
    let req_let = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 20 } }
        }
    });
    let res_let = handle_inlay_hint(&req_let, Some(&ws), &overlay_mgr);
    assert!(res_let.is_array());

    // Inlay hints with `let mut` and range filtering (lines 46, 52 in hints.rs)
    let code_with_mut = "fn test() {\n   let mut val = 100;\n}\n";
    overlay_mgr.open_document(
        &uri,
        file_path.to_str().unwrap(),
        2,
        code_with_mut.to_string(),
    );
    let req_mut = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "range": { "start": { "line": 1, "character": 0 }, "end": { "line": 1, "character": 20 } }
        }
    });
    let res_mut = handle_inlay_hint(&req_mut, Some(&ws), &overlay_mgr);
    assert!(res_mut.is_array());

    let inline_vals = handle_inline_value(&hint_req);
    assert!(inline_vals.is_array());
    assert_eq!(handle_inline_completion(&hint_req), json!({ "items": [] }));

    let _ = fs::remove_dir_all(&temp_dir);
}
