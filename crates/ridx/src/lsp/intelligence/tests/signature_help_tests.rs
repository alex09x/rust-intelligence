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
use crate::lsp::intelligence::handle_signature_help;
use ridx_format::DefKind;
use serde_json::json;

#[test]
fn test_signature_missing_params() {
    let (_temp, ws, overlay_mgr, mut conv) = setup_test_intelligence();
    let req = json!({});
    let err = handle_signature_help(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap_err();
    assert_eq!(err.code, INVALID_PARAMS);
}

#[test]
fn test_signature_missing_or_empty_uri() {
    let (_temp, ws, overlay_mgr, mut conv) = setup_test_intelligence();
    let req_missing = json!({
        "params": {
            "position": { "line": 0, "character": 0 }
        }
    });
    assert_eq!(handle_signature_help(&req_missing, Some(&ws), &overlay_mgr, &mut conv).unwrap_err().code, INVALID_PARAMS);

    let req_empty = json!({
        "params": {
            "textDocument": { "uri": "" },
            "position": { "line": 0, "character": 0 }
        }
    });
    assert_eq!(handle_signature_help(&req_empty, Some(&ws), &overlay_mgr, &mut conv).unwrap_err().code, INVALID_PARAMS);
}

#[test]
fn test_signature_missing_position() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_intelligence();
    let file = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file.to_str().unwrap());

    let req_no_pos = json!({
        "params": {
            "textDocument": { "uri": &uri }
        }
    });
    assert_eq!(handle_signature_help(&req_no_pos, Some(&ws), &overlay_mgr, &mut conv).unwrap_err().code, INVALID_PARAMS);
}

#[test]
fn test_signature_null_workspace() {
    let (temp_dir, _ws, overlay_mgr, mut conv) = setup_test_intelligence();
    let file = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file.to_str().unwrap());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 0 }
        }
    });
    let res = handle_signature_help(&req, None, &overlay_mgr, &mut conv).unwrap();
    assert!(res.is_null());
}

#[test]
fn test_signature_outside_call() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_intelligence();
    let file = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file.to_str().unwrap());

    let code = "fn main() {\n    let x = 42;\n}\n";
    overlay_mgr.open_document(&uri, file.to_str().unwrap(), 1, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 10 }
        }
    });
    let res = handle_signature_help(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(res.is_null());
}

#[test]
fn test_signature_unknown_callee() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_intelligence();
    let file = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file.to_str().unwrap());

    let code = "fn main() {\n    unknown_func(1, 2);\n}\n";
    overlay_mgr.open_document(&uri, file.to_str().unwrap(), 1, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 18 }
        }
    });
    let res = handle_signature_help(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(res.is_null());
}

#[test]
fn test_signature_single_param_active_index() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_intelligence();
    let file = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file.to_str().unwrap());

    let code = "fn main() {\n    calculate(10\n}\n";
    overlay_mgr.open_document(&uri, file.to_str().unwrap(), 1, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 16 }
        }
    });
    let res = handle_signature_help(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!res.is_null());
    assert_eq!(res["activeParameter"], 0);
    let sigs = res["signatures"].as_array().unwrap();
    assert_eq!(sigs.len(), 1);
    assert!(sigs[0]["label"].as_str().unwrap().contains("calculate"));
}

#[test]
fn test_signature_multi_param_advancement() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_intelligence();
    let file = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file.to_str().unwrap());

    let code = "fn main() {\n    calculate(10, 20);\n}\n";
    overlay_mgr.open_document(&uri, file.to_str().unwrap(), 1, code.to_string());

    // Position after comma
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 19 }
        }
    });
    let res = handle_signature_help(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!res.is_null());
    assert_eq!(res["activeParameter"], 1);
}

#[test]
fn test_signature_nested_call_isolation() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_intelligence();
    let file = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file.to_str().unwrap());

    let code = "fn main() {\n    calculate(calculate(1, 2), 30);\n}\n";
    overlay_mgr.open_document(&uri, file.to_str().unwrap(), 1, code.to_string());

    // Cursor inside inner call at second argument: calculate(1, 2)
    let req_inner = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 27 }
        }
    });
    let res_inner = handle_signature_help(&req_inner, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!res_inner.is_null());
    assert_eq!(res_inner["activeParameter"], 1);

    // Cursor after inner call in outer call: calculate(..., 30)
    let req_outer = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 32 }
        }
    });
    let res_outer = handle_signature_help(&req_outer, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!res_outer.is_null());
    assert_eq!(res_outer["activeParameter"], 1);
}

#[test]
fn test_signature_generics_turbofish_angle_brackets() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_intelligence();
    let file = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file.to_str().unwrap());

    // Turbofish with multiple generic type parameters separated by comma
    let code = "fn main() {\n    calculate::<Option<i32>, Result<u64, String>>(42, 99);\n}\n";
    overlay_mgr.open_document(&uri, file.to_str().unwrap(), 1, code.to_string());

    // Inside first argument (42) -> activeParameter should be 0 despite commas inside <...>
    let req_first = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 52 }
        }
    });
    let res_first = handle_signature_help(&req_first, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!res_first.is_null());
    assert_eq!(res_first["activeParameter"], 0);

    // Inside second argument (99) -> activeParameter should be 1
    let req_second = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 56 }
        }
    });
    let res_second = handle_signature_help(&req_second, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!res_second.is_null());
    assert_eq!(res_second["activeParameter"], 1);
}

#[test]
fn test_signature_nested_arrays_and_tuples() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_intelligence();
    let file = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file.to_str().unwrap());

    let code = "fn main() {\n    calculate([1, 2, 3], 42);\n}\n";
    overlay_mgr.open_document(&uri, file.to_str().unwrap(), 1, code.to_string());

    // Argument 2 after array literal: commas inside [1, 2, 3] do not advance activeParameter
    let req_after_arr = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 26 }
        }
    });
    let res_arr = handle_signature_help(&req_after_arr, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!res_arr.is_null());
    assert_eq!(res_arr["activeParameter"], 1);
}

#[test]
fn test_signature_nested_blocks_and_closures() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_intelligence();
    let file = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file.to_str().unwrap());

    let code = "fn main() {\n    calculate({ let a = 1; let b = 2; a + b }, 100);\n}\n";
    overlay_mgr.open_document(&uri, file.to_str().unwrap(), 1, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 47 }
        }
    });
    let res = handle_signature_help(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!res.is_null());
    assert_eq!(res["activeParameter"], 1);
}

#[test]
fn test_signature_trailing_commas() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_intelligence();
    let file = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file.to_str().unwrap());

    let code = "fn main() {\n    calculate(1, 2, );\n}\n";
    overlay_mgr.open_document(&uri, file.to_str().unwrap(), 1, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 19 }
        }
    });
    let res = handle_signature_help(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!res.is_null());
    assert_eq!(res["activeParameter"], 2);
}

#[test]
fn test_signature_multiline_call() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_intelligence();
    let file = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file.to_str().unwrap());

    let code = "fn main() {\n    calculate(\n        10,\n        20,\n    );\n}\n";
    overlay_mgr.open_document(&uri, file.to_str().unwrap(), 1, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 3, "character": 9 }
        }
    });
    let res = handle_signature_help(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!res.is_null());
    assert_eq!(res["activeParameter"], 1);
}

#[test]
fn test_signature_overlay_callee() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_intelligence();
    let file = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file.to_str().unwrap());

    let code = "fn helper(x: u32, y: u32) -> u32 { x + y }\nfn main() { helper(5, 10); }\n";
    overlay_mgr.open_document(&uri, file.to_str().unwrap(), 1, code.to_string());
    if let Some(ov) = overlay_mgr.get_mut(file.to_str().unwrap()) {
        ov.symbols.push(crate::overlay::OverlaySymbol {
            name: "helper".to_string(),
            kind: DefKind::Fn,
            signature: Some("fn helper(x: u32, y: u32) -> u32".to_string()),
            start_line: 0,
            start_col: 3,
            end_line: 0,
            end_col: 9,
            scope_id: 0,
            decl_start_byte: 0,
            decl_end_byte: 9,
            scope_start_byte: 0,
        });
    }

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 23 }
        }
    });
    let res = handle_signature_help(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!res.is_null());
    assert_eq!(res["activeParameter"], 1);
    let sigs = res["signatures"].as_array().unwrap();
    assert!(sigs[0]["label"].as_str().unwrap().contains("fn helper"));
}
