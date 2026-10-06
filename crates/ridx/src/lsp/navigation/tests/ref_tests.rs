/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::setup_test_navigation;
use crate::lsp::navigation::references::*;
use crate::overlay::OverlayManager;
use serde_json::json;
use std::fs;

#[test]
fn test_references_and_highlights() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    // 1. References from workspace
    let ref_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 8 },
            "context": { "includeDeclaration": true }
        }
    });
    let ref_res = handle_references(&ref_req, Some(&ws), &overlay_mgr, &mut conv);
    assert!(!ref_res.as_array().unwrap().is_empty());

    // 2. Document highlight from workspace
    let hl_res = handle_document_highlight(&ref_req, Some(&ws), &overlay_mgr, &mut conv);
    assert!(!hl_res.as_array().unwrap().is_empty());

    // 3. Local binding references and highlights in overlay
    let mut local_ov_mgr = OverlayManager::new();
    let local_code = "fn test() {\n    let my_var = 1;\n    let _ = my_var;\n}\n";
    local_ov_mgr.open_document(
        &uri,
        file_path.to_str().unwrap(),
        10,
        local_code.to_string(),
    );
    let local_ref_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 9 }
        }
    });
    let local_refs = handle_references(&local_ref_req, Some(&ws), &local_ov_mgr, &mut conv);
    assert_eq!(local_refs.as_array().unwrap().len(), 2);

    let local_hl = handle_document_highlight(&local_ref_req, Some(&ws), &local_ov_mgr, &mut conv);
    assert_eq!(local_hl.as_array().unwrap().len(), 2);
    assert_eq!(local_hl[0]["kind"], 3);
    assert_eq!(local_hl[1]["kind"], 1);

    // 4. Query references and highlight from a reference callsite (where target_end_line == 0)
    let callsite_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 4, "character": 19 },
            "context": { "includeDeclaration": true }
        }
    });
    let cs_refs = handle_references(&callsite_req, Some(&ws), &overlay_mgr, &mut conv);
    assert!(!cs_refs.as_array().unwrap().is_empty());

    let cs_hl = handle_document_highlight(&callsite_req, Some(&ws), &overlay_mgr, &mut conv);
    assert!(!cs_hl.as_array().unwrap().is_empty());

    // 5. Query references and highlight on a definition where target_end_line == 0
    let zero_end_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 9, "character": 8 },
            "context": { "includeDeclaration": true }
        }
    });
    let ze_refs = handle_references(&zero_end_req, Some(&ws), &overlay_mgr, &mut conv);
    assert!(!ze_refs.as_array().unwrap().is_empty());

    let ze_hl = handle_document_highlight(&zero_end_req, Some(&ws), &overlay_mgr, &mut conv);
    assert!(!ze_hl.as_array().unwrap().is_empty());

    let _ = fs::remove_dir_all(&temp_dir);
}
