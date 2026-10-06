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
fn test_signature_help_disk_file_and_delimiters() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_intelligence();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    // 1. Signature help from on-disk file when file is not open in overlay_mgr (lines 42-60 in signature.rs)
    let disk_code = "fn run() {\n    calculate   ([1, 2], { let x = 1; x }, 42);\n}\n";
    fs::write(&file_path, disk_code).unwrap();
    let req_disk = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 43 }
        }
    });
    let sig_disk = handle_signature_help(&req_disk, Some(&ws), &overlay_mgr, &mut conv);
    assert!(!sig_disk.is_null());

    // 2. Delimiters handling: angle brackets in arguments and whitespace before :: (lines 83-100, 136-137 in signature.rs)
    let delim_code =
        "fn run() {\n    calculate  ::<Option<i32>>(std::sync::Arc::<i32>::new(1), 30);\n}\n";
    overlay_mgr.open_document(&uri, file_path.to_str().unwrap(), 1, delim_code.to_string());
    let req_delim = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 62 }
        }
    });
    let sig_delim = handle_signature_help(&req_delim, Some(&ws), &overlay_mgr, &mut conv);
    assert!(!sig_delim.is_null());

    // 3. Callee signature with no parens and unclosed parens (lines 216, 219 in signature.rs)
    let no_paren_code =
        "fn dummy_call() { no_paren(1); unclosed(2); empty_sig_fn(3); missing_ov(4); }\n";
    overlay_mgr.open_document(
        &uri,
        file_path.to_str().unwrap(),
        2,
        no_paren_code.to_string(),
    );
    if let Some(ov) = overlay_mgr.get_mut(file_path.to_str().unwrap()) {
        ov.symbols.push(crate::overlay::OverlaySymbol {
            name: "no_paren".to_string(),
            kind: ridx_format::DefKind::Fn,
            signature: Some("fn no_paren".to_string()),
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 10,
            scope_id: 0,
            decl_start_byte: 0,
            decl_end_byte: 10,
            scope_start_byte: 0,
        });
        ov.symbols.push(crate::overlay::OverlaySymbol {
            name: "unclosed".to_string(),
            kind: ridx_format::DefKind::Fn,
            signature: Some("fn unclosed(".to_string()),
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 10,
            scope_id: 0,
            decl_start_byte: 0,
            decl_end_byte: 10,
            scope_start_byte: 0,
        });
        ov.symbols.push(crate::overlay::OverlaySymbol {
            name: "missing_ov".to_string(),
            kind: ridx_format::DefKind::Fn,
            signature: None,
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 10,
            scope_id: 0,
            decl_start_byte: 0,
            decl_end_byte: 10,
            scope_start_byte: 0,
        });
    }
    let req_no_paren = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 27 }
        }
    });
    let sig_no_paren = handle_signature_help(&req_no_paren, Some(&ws), &overlay_mgr, &mut conv);
    assert!(!sig_no_paren.is_null());

    let req_unclosed = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 40 }
        }
    });
    let sig_unclosed = handle_signature_help(&req_unclosed, Some(&ws), &overlay_mgr, &mut conv);
    assert!(!sig_unclosed.is_null());

    let req_empty_sig = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 57 }
        }
    });
    let sig_empty = handle_signature_help(&req_empty_sig, Some(&ws), &overlay_mgr, &mut conv);
    assert!(sig_empty.is_null());

    let req_missing_ov = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 73 }
        }
    });
    let sig_missing_ov = handle_signature_help(&req_missing_ov, Some(&ws), &overlay_mgr, &mut conv);
    assert!(sig_missing_ov.is_null());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_hover_empty_parts_and_unmapped_dirty() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_intelligence();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    // 1. Hover on position with no symbol in workspace (lines 63-64 in hover.rs)
    let req_empty = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 0 }
        }
    });
    let h_empty = handle_hover(&req_empty, Some(&ws), &overlay_mgr, &mut conv);
    assert!(h_empty.is_null());

    // 2. Dirty overlay where mapping fails or points to empty line (lines 110-114 in hover.rs)
    overlay_mgr.open_document(&uri, file_path.to_str().unwrap(), 1, "\n\n\n".to_string());
    overlay_mgr.update(file_path.to_str().unwrap(), 2, "// dirty\n\n\n".to_string());
    let req_unmapped = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 0 }
        }
    });
    let h_unmapped = handle_hover(&req_unmapped, Some(&ws), &overlay_mgr, &mut conv);
    assert!(h_unmapped.is_null());

    let _ = fs::remove_dir_all(&temp_dir);
}
