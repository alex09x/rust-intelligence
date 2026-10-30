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
use crate::lsp::error::INVALID_PARAMS;
use crate::lsp::intelligence::handle_completion;
use ridx_format::DefKind;
use serde_json::json;

#[test]
fn test_completion_missing_params() {
    let (_temp, ws, overlay_mgr, mut conv) = setup_test_intelligence();
    let req = json!({});
    let err = handle_completion(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap_err();
    assert_eq!(err.code, INVALID_PARAMS);
}

#[test]
fn test_completion_missing_or_empty_uri() {
    let (_temp, ws, overlay_mgr, mut conv) = setup_test_intelligence();
    let req_missing = json!({
        "params": {
            "position": { "line": 0, "character": 0 }
        }
    });
    assert_eq!(handle_completion(&req_missing, Some(&ws), &overlay_mgr, &mut conv).unwrap_err().code, INVALID_PARAMS);

    let req_empty = json!({
        "params": {
            "textDocument": { "uri": "" },
            "position": { "line": 0, "character": 0 }
        }
    });
    assert_eq!(handle_completion(&req_empty, Some(&ws), &overlay_mgr, &mut conv).unwrap_err().code, INVALID_PARAMS);
}

#[test]
fn test_completion_missing_position() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_intelligence();
    let file = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file.to_str().unwrap());

    let req_no_pos = json!({
        "params": {
            "textDocument": { "uri": &uri }
        }
    });
    assert_eq!(handle_completion(&req_no_pos, Some(&ws), &overlay_mgr, &mut conv).unwrap_err().code, INVALID_PARAMS);

    let req_no_char = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0 }
        }
    });
    assert_eq!(handle_completion(&req_no_char, Some(&ws), &overlay_mgr, &mut conv).unwrap_err().code, INVALID_PARAMS);
}

#[test]
fn test_completion_null_workspace() {
    let (temp_dir, _ws, overlay_mgr, mut conv) = setup_test_intelligence();
    let file = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file.to_str().unwrap());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 0 }
        }
    });
    let res = handle_completion(&req, None, &overlay_mgr, &mut conv).unwrap();
    assert!(res["items"].as_array().unwrap().is_empty());
}

#[test]
fn test_completion_empty_prefix_returns_all() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_intelligence();
    let file = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file.to_str().unwrap());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 0 }
        }
    });
    let res = handle_completion(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!res.is_null());
    let items = res["items"].as_array().unwrap();
    assert!(items.len() >= 2);
    let labels: Vec<&str> = items.iter().map(|i| i["label"].as_str().unwrap()).collect();
    assert!(labels.contains(&"calculate"));
    assert!(labels.contains(&"User"));
}

#[test]
fn test_completion_prefix_filtering() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_intelligence();
    let file = temp_dir.join("src/main.rs");
    let uri = format!("file://{}", file.to_str().unwrap());

    let doc = "fn main() {\n    calc\n}\n";
    overlay_mgr.open_document(&uri, file.to_str().unwrap(), 1, doc.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 8 }
        }
    });
    let res = handle_completion(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!res.is_null());
    let items = res["items"].as_array().unwrap();
    assert!(items.iter().any(|i| i["label"].as_str().unwrap() == "calculate"));
    assert!(!items.iter().any(|i| i["label"].as_str().unwrap() == "User"));
}

#[test]
fn test_completion_non_matching_prefix() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_intelligence();
    let file = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file.to_str().unwrap());

    let doc = "fn main() {\n    xyznonexistent\n}\n";
    overlay_mgr.open_document(&uri, file.to_str().unwrap(), 1, doc.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 18 }
        }
    });
    let res = handle_completion(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!res.is_null());
    let items = res["items"].as_array().unwrap();
    assert!(items.is_empty());
}

#[test]
fn test_completion_overlay_symbols_priority() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_intelligence();
    let file = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file.to_str().unwrap());

    let doc = "fn custom_helper() {}\nfn main() { cust }\n";
    overlay_mgr.open_document(&uri, file.to_str().unwrap(), 1, doc.to_string());
    if let Some(ov) = overlay_mgr.get_mut(file.to_str().unwrap()) {
        ov.symbols.push(crate::overlay::OverlaySymbol {
            name: "custom_helper".to_string(),
            kind: DefKind::Fn,
            signature: Some("fn custom_helper()".to_string()),
            start_line: 0,
            start_col: 3,
            end_line: 0,
            end_col: 16,
            scope_id: 0,
            decl_start_byte: 0,
            decl_end_byte: 16,
            scope_start_byte: 0,
        });
    }

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 16 }
        }
    });
    let res = handle_completion(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!res.is_null());
    let items = res["items"].as_array().unwrap();
    assert!(items.iter().any(|i| i["label"].as_str().unwrap() == "custom_helper"));
}

#[test]
fn test_completion_symbol_kinds_mapping() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_intelligence();
    let file = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file.to_str().unwrap());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 0 }
        }
    });
    let res = handle_completion(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    let items = res["items"].as_array().unwrap();

    let calc_item = items.iter().find(|i| i["label"] == "calculate").unwrap();
    assert_eq!(calc_item["kind"], 3);

    let user_item = items.iter().find(|i| i["label"] == "User").unwrap();
    assert_eq!(user_item["kind"], 22);
}

#[test]
fn test_completion_compiler_internals_filtered() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_intelligence();
    let file = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file.to_str().unwrap());

    let doc = "fn main() {}\n";
    overlay_mgr.open_document(&uri, file.to_str().unwrap(), 1, doc.to_string());
    if let Some(ov) = overlay_mgr.get_mut(file.to_str().unwrap()) {
        ov.symbols.push(crate::overlay::OverlaySymbol {
            name: "{closure#0}".to_string(),
            kind: DefKind::Fn,
            signature: None,
            start_line: 0,
            start_col: 0,
            end_line: 0,
            end_col: 10,
            scope_id: 0,
            decl_start_byte: 0,
            decl_end_byte: 10,
            scope_start_byte: 0,
        });
    }

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 0 }
        }
    });
    let res = handle_completion(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    let items = res["items"].as_array().unwrap();
    assert!(!items.iter().any(|i| i["label"].as_str().unwrap().contains("{closure")));
}

#[test]
fn test_completion_empty_document() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_intelligence();
    let file = temp_dir.join("src/empty.rs");
    let uri = format!("file://{}", file.to_str().unwrap());

    overlay_mgr.open_document(&uri, file.to_str().unwrap(), 1, String::new());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 0 }
        }
    });
    let res = handle_completion(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!res.is_null());
}

#[test]
fn test_completion_at_line_end_and_eof() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_intelligence();
    let file = temp_dir.join("src/main.rs");
    let uri = format!("file://{}", file.to_str().unwrap());

    let doc = "pub fn calc";
    overlay_mgr.open_document(&uri, file.to_str().unwrap(), 1, doc.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 11 }
        }
    });
    let res = handle_completion(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!res.is_null());
    let items = res["items"].as_array().unwrap();
    assert!(items.iter().any(|i| i["label"] == "calculate"));
}
