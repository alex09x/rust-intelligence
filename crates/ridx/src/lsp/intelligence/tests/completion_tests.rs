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

/// Where a type is typed, the built-in types lead the list and no function is offered; where a value is, neither the
/// types nor anything changes. They need no index: a crate that did not build gets them too.
#[test]
fn test_completion_builtin_types_where_a_type_is_typed() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_intelligence();
    let file = temp_dir.join("src/main.rs");
    let uri = format!("file://{}", file.to_str().unwrap());
    let doc = "type uint = u32;\nfn used() {}\nfn main() {\n    let a: u\n    let b = u\n    let c = 5 as i\n    let d = Rec { f: us\n    let e: Result<\n        u\n    let f: (u8, i\n}\n";
    overlay_mgr.open_document(&uri, file.to_str().unwrap(), 1, doc.to_string());
    let labels_at = |ws, line: u32, character: u32, conv: &mut _| -> Vec<String> {
        let req = json!({ "params": { "textDocument": { "uri": &uri }, "position": { "line": line, "character": character } } });
        let mut items = handle_completion(&req, ws, &overlay_mgr, conv).unwrap()["items"].as_array().unwrap().clone();
        // The order an editor shows: by `sortText`, else by label.
        items.sort_by_key(|i| i["sortText"].as_str().or(i["label"].as_str()).unwrap_or_default().to_string());
        items.iter().map(|i| i["label"].as_str().unwrap().to_string()).collect()
    };

    for ws in [Some(&ws), None] {
        let typed = labels_at(ws, 3, 12, &mut conv);
        assert_eq!(typed[..6], ["u8", "u16", "u32", "u64", "u128", "usize"], "{typed:?}");
        assert!(typed.contains(&"uint".to_string()), "a type alias stays: {typed:?}");
        assert!(!typed.contains(&"used".to_string()), "no function where a type is typed: {typed:?}");

        let cast = labels_at(ws, 5, 18, &mut conv);
        assert_eq!(cast[..5], ["i8", "i16", "i32", "i64", "i128"], "{cast:?}");

        let value = labels_at(ws, 4, 13, &mut conv);
        assert!(value.contains(&"used".to_string()), "{value:?}");
        assert!(!value.contains(&"u8".to_string()), "no built-in type where a value is typed: {value:?}");

        // A struct literal's field takes a value: the types lead, the function stays (review of b789093b).
        let field = labels_at(ws, 6, 23, &mut conv);
        assert_eq!(field.first().map(String::as_str), Some("usize"), "{field:?}");
        assert!(field.contains(&"used".to_string()), "{field:?}");

        // A tuple's element (review of c697ee37).
        let tuple = labels_at(ws, 9, 17, &mut conv);
        assert_eq!(tuple[..3], ["i8", "i16", "i32"], "{tuple:?}");

        // A type argument on the line after its `<`.
        let next_line = labels_at(ws, 8, 9, &mut conv);
        assert_eq!(next_line[..6], ["u8", "u16", "u32", "u64", "u128", "usize"], "{next_line:?}");
    }
}

/// A long comment with a quote in it before the annotation: the comment is still read as one (review of c697ee37).
#[test]
fn test_completion_builtin_types_after_a_long_comment() {
    let (temp_dir, _ws, mut overlay_mgr, mut conv) = setup_test_intelligence();
    let file = temp_dir.join("src/main.rs");
    let uri = format!("file://{}", file.to_str().unwrap());
    let doc = format!("/* {} \" */\nfn main() {{\n    let value: u\n}}\n", "a".repeat(5000));
    overlay_mgr.open_document(&uri, file.to_str().unwrap(), 1, doc);
    let req = json!({ "params": { "textDocument": { "uri": &uri }, "position": { "line": 2, "character": 16 } } });
    let res = handle_completion(&req, None, &overlay_mgr, &mut conv).unwrap();
    let labels: Vec<&str> = res["items"].as_array().unwrap().iter().filter_map(|i| i["label"].as_str()).collect();
    assert!(labels.contains(&"u8") && labels.contains(&"usize"), "{labels:?}");
}
