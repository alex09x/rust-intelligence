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
fn test_signature_help_extended_and_disk_fallback() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_intelligence();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    // 1. Generic callee: calculate::<i32>(10, 20)
    let call_generic = "fn test() { calculate::<i32>(10, 20); }\n";
    overlay_mgr.open_document(
        &uri,
        file_path.to_str().unwrap(),
        1,
        call_generic.to_string(),
    );
    let req_generic = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 30 }
        }
    });
    let sig_generic =
        handle_signature_help(&req_generic, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!sig_generic.is_null());

    // 2. Complex call with nested brackets and overlay signature extraction
    let call_complex = "fn complex_fn(a: (i32, i32), b: [u8; 4], c: Vec<String>) -> bool { true }\nfn test() { complex_fn((1, 2), [0; 4], <Vec<String>>::new()); }\n";
    overlay_mgr.open_document(
        &uri,
        file_path.to_str().unwrap(),
        2,
        call_complex.to_string(),
    );
    let req_complex = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 38 }
        }
    });
    let sig_complex =
        handle_signature_help(&req_complex, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!sig_complex.is_null());

    // 3. Signature help on disk file (not in overlay)
    let disk_file = temp_dir.join("src/disk_call.rs");
    let _ = fs::write(&disk_file, "fn run() {\n    calculate(5, 10);\n}\n");
    let disk_uri = format!("file://{}", disk_file.to_str().unwrap());
    let req_disk = json!({
        "params": {
            "textDocument": { "uri": &disk_uri },
            "position": { "line": 1, "character": 16 }
        }
    });
    let sig_disk = handle_signature_help(&req_disk, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!sig_disk.is_null());

    // 4. Signature help outside calls or on unknown function
    let req_outside = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 2 }
        }
    });
    assert!(
        handle_signature_help(&req_outside, Some(&ws), &overlay_mgr, &mut conv)
            .unwrap()
            .is_null()
    );

    // 5. Signature help with unknown callee returns null
    let unknown_call = "fn test() { no_such_func(1, 2); }\n";
    overlay_mgr.open_document(
        &uri,
        file_path.to_str().unwrap(),
        3,
        unknown_call.to_string(),
    );
    let req_unknown = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 26 }
        }
    });
    assert!(
        handle_signature_help(&req_unknown, Some(&ws), &overlay_mgr, &mut conv)
            .unwrap()
            .is_null()
    );

    // 6. Signature help with whitespace, turbofish, brackets, and angle brackets
    let ws_call =
        "fn test() { calculate::<i32>  ([1, 2], <Option<i32> as Default>::default()); }\n";
    overlay_mgr.open_document(&uri, file_path.to_str().unwrap(), 4, ws_call.to_string());
    let req_ws = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 44 }
        }
    });
    let sig_ws = handle_signature_help(&req_ws, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!sig_ws.is_null());

    // 7. Signature help with braces, nested turbofish, and fallback to workspace symbols
    let complex_call = "fn test() { let _b = { 42 }; calculate::<Option<Vec<i32>>>(1, 2); }\n";
    overlay_mgr.open_document(
        &uri,
        file_path.to_str().unwrap(),
        5,
        complex_call.to_string(),
    );
    if let Some(ov) = overlay_mgr.get_mut(file_path.to_str().unwrap()) {
        ov.symbols.clear(); // forces fallback to ws symbols in signature.rs
    }
    let req_complex = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 63 }
        }
    });
    let sig_complex =
        handle_signature_help(&req_complex, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!sig_complex.is_null());

    // 8. Signature help at expression without callee name (e.g. `(1 + 2)`)
    let paren_call = "fn test() { (1 + 2); }\n";
    overlay_mgr.open_document(&uri, file_path.to_str().unwrap(), 6, paren_call.to_string());
    let req_paren = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 16 }
        }
    });
    assert!(
        handle_signature_help(&req_paren, Some(&ws), &overlay_mgr, &mut conv)
            .unwrap()
            .is_null()
    );

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_hover_dirty_fallback() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_intelligence();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    // Open clean baseline
    let clean_code =
        "pub fn calculate(a: i32, b: i32) -> i32 { a + b }\npub struct User { pub id: u64 }\n";
    overlay_mgr.open_document(&uri, file_path.to_str().unwrap(), 1, clean_code.to_string());

    // 1. Clean hover on User (kind and name without signature)
    let req_user = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 12 }
        }
    });
    let h_user = handle_hover(&req_user, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!h_user.is_null());

    // 2. Dirty overlay that maps back to line 0 (calculate with signature)
    overlay_mgr.update(
        file_path.to_str().unwrap(),
        2,
        format!("// comment\n{}", clean_code),
    );
    let req_calc = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 10 }
        }
    });
    let h_calc = handle_hover(&req_calc, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!h_calc.is_null());

    // 3. Dirty overlay mapping back to User (definition without signature)
    let req_user_dirty = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 2, "character": 12 }
        }
    });
    let h_user_dirty = handle_hover(&req_user_dirty, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!h_user_dirty.is_null());

    // 4. Dirty overlay mapping back to line 0 col 30 (inferred type without overlay definition)
    let req_at_30 = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 30 }
        }
    });
    let h_type = handle_hover(&req_at_30, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!h_type.is_null());
    assert!(h_type["contents"]["value"]
        .as_str()
        .unwrap()
        .contains("Type: i32"));

    // 5. Dirty overlay fallback to compiled definitions when overlay symbols are absent (lines 90-95 in hover.rs)
    overlay_mgr.update(
        file_path.to_str().unwrap(),
        3,
        format!("// prepended\n{}", clean_code),
    );
    if let Some(ov) = overlay_mgr.get_mut(file_path.to_str().unwrap()) {
        ov.symbols.clear(); // forces fallback to ws symbols in signature.rs
    }
    let req_calc_fallback = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 10 }
        }
    });
    let h_calc_fb = handle_hover(&req_calc_fallback, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!h_calc_fb.is_null());
    assert!(h_calc_fb["contents"]["value"]
        .as_str()
        .unwrap()
        .contains("calculate"));

    let req_user_fallback = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 2, "character": 12 }
        }
    });
    let h_user_fb = handle_hover(&req_user_fallback, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!h_user_fb.is_null());
    assert!(h_user_fb["contents"]["value"]
        .as_str()
        .unwrap()
        .contains("struct User"));

    // Signature help for nonexistent file (lines 60-61 in signature.rs)
    let req_nonexist_sig = json!({
        "params": {
            "textDocument": { "uri": "file:///nonexistent_file_for_sig.rs" },
            "position": { "line": 0, "character": 0 }
        }
    });
    assert!(
        handle_signature_help(&req_nonexist_sig, Some(&ws), &overlay_mgr, &mut conv)
            .unwrap()
            .is_null()
    );

    let _ = fs::remove_dir_all(&temp_dir);
}
