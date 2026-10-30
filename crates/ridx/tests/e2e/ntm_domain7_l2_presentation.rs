/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Compiler-grounded L2 integration tests for Domain 7: Presentation & Tokens.
//! Tests semanticTokens/full, semanticTokens/full/delta, semanticTokens/range,
//! foldingRange, codeLens, documentColor, documentLink, and selectionRange.

use super::l2_harness::{fixture_dir, get_or_index_l2_fixture, pos_in_file};
use ridx::lsp::presentation::color::{handle_color_presentation, handle_document_color};
use ridx::lsp::presentation::folding::handle_folding_range;
use ridx::lsp::presentation::lens::{handle_code_lens, handle_code_lens_resolve};
use ridx::lsp::presentation::tokens::{
    handle_semantic_tokens_delta, handle_semantic_tokens_full, handle_semantic_tokens_range, TOKEN_TYPE_FUNCTION, TOKEN_TYPE_INTERFACE, TOKEN_TYPE_METHOD, TOKEN_TYPE_STRUCT,
};
use ridx::lsp::presentation::{handle_document_link, handle_selection_range};
use ridx::lsp::protocol::{path_to_uri, PositionConverter};
use ridx::overlay::OverlayManager;
use serde_json::json;

#[test]
fn test_l2_semantic_tokens_full_compiler_grounded() {
    let ws = get_or_index_l2_fixture("traits_and_generics");
    let lib_file = fixture_dir("traits_and_generics").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let full_req = json!({
        "params": {
            "textDocument": { "uri": &uri }
        }
    });

    let res = handle_semantic_tokens_full(&full_req, Some(ws), &overlay_mgr, &mut conv);
    assert!(res.is_object(), "semanticTokens/full must return an object");
    let result_id = res["resultId"].as_str().expect("resultId must be present");
    assert!(result_id.starts_with("ridx-st-"), "resultId must follow ridx-st- convention");

    let data = res["data"].as_array().expect("data array must be present");
    assert!(!data.is_empty(), "Tokens data must not be empty for traits_and_generics");
    assert_eq!(data.len() % 5, 0, "Tokens data length must be a multiple of 5 (LSP 5-tuple)");

    let mut token_types = Vec::new();
    for chunk in data.chunks(5) {
        let t_type = chunk[3].as_u64().unwrap() as u32;
        token_types.push(t_type);
    }

    assert!(token_types.contains(&TOKEN_TYPE_STRUCT) || token_types.contains(&TOKEN_TYPE_INTERFACE), "Tokens must contain structs or interfaces/traits from compiler index");
    assert!(token_types.contains(&TOKEN_TYPE_FUNCTION) || token_types.contains(&TOKEN_TYPE_METHOD), "Tokens must contain functions or methods from compiler index");
}

#[test]
fn test_l2_semantic_tokens_delta_caching_and_edits() {
    let ws = get_or_index_l2_fixture("traits_and_generics");
    let lib_file = fixture_dir("traits_and_generics").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let full_req = json!({
        "params": {
            "textDocument": { "uri": &uri }
        }
    });
    let full_res = handle_semantic_tokens_full(&full_req, Some(ws), &overlay_mgr, &mut conv);
    let prev_id = full_res["resultId"].as_str().unwrap();

    // 1. Delta on unchanged document must produce 0 edits
    let delta_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "previousResultId": prev_id
        }
    });
    let delta_res = handle_semantic_tokens_delta(&delta_req, Some(ws), &overlay_mgr, &mut conv);
    let edits = delta_res["edits"].as_array().expect("edits array");
    assert_eq!(edits.len(), 0, "Unchanged delta must contain 0 edits");
    let new_id = delta_res["resultId"].as_str().unwrap();

    // 2. Open dirty overlay with new code
    let modified_code = "pub fn new_helper() {}\n".to_string() + &std::fs::read_to_string(&lib_file).unwrap();
    overlay_mgr.open_document(&uri, lib_file.to_str().unwrap(), 1, modified_code);

    let delta_req_after = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "previousResultId": new_id
        }
    });
    let delta_res_after = handle_semantic_tokens_delta(&delta_req_after, Some(ws), &overlay_mgr, &mut conv);
    let edits_after = delta_res_after["edits"].as_array().expect("edits array");
    assert!(!edits_after.is_empty(), "Modified document must produce delta edits");
    assert!(edits_after[0]["start"].as_u64().is_some(), "Edit must specify start offset");
}

#[test]
fn test_l2_semantic_tokens_range_bounded() {
    let ws = get_or_index_l2_fixture("traits_and_generics");
    let lib_file = fixture_dir("traits_and_generics").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let (start_line, _) = pos_in_file(&lib_file, "pub trait Greeter {", "Greeter");
    let range_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "range": {
                "start": { "line": start_line, "character": 0 },
                "end": { "line": start_line + 5, "character": 0 }
            }
        }
    });

    let res = handle_semantic_tokens_range(&range_req, Some(ws), &overlay_mgr, &mut conv);
    let data = res["data"].as_array().expect("data array");
    assert!(!data.is_empty(), "Range request over trait definition must find tokens");
    assert_eq!(data.len() % 5, 0, "Range tokens must be 5-tuples");
}

#[test]
fn test_l2_folding_range_and_code_lens() {
    let ws = get_or_index_l2_fixture("traits_and_generics");
    let lib_file = fixture_dir("traits_and_generics").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let mut overlay_mgr = OverlayManager::new();

    // 1. Folding range on disk file (brace-matched)
    let fold_req = json!({
        "params": {
            "textDocument": { "uri": &uri }
        }
    });
    let fold_res = handle_folding_range(&fold_req, &overlay_mgr);
    let fold_ranges = fold_res.as_array().expect("folding ranges array");
    assert!(!fold_ranges.is_empty(), "Folding ranges must be generated for traits_and_generics");
    for r in fold_ranges {
        assert!(r["endLine"].as_u64().unwrap() > r["startLine"].as_u64().unwrap(), "End line must be strictly greater than start line");
    }

    // 2. Folding range on live overlay buffer
    let code = "fn block_test() {\n    let x = 1;\n    let y = 2;\n}\n";
    overlay_mgr.open_document(&uri, lib_file.to_str().unwrap(), 1, code.to_string());
    let fold_res_overlay = handle_folding_range(&fold_req, &overlay_mgr);
    assert!(fold_res_overlay.is_array(), "Overlay folding ranges must return array");

    // 3. Code lens on compiler-indexed workspace
    let lens_req = json!({
        "params": {
            "textDocument": { "uri": &uri }
        }
    });
    let mut conv = PositionConverter::new();
    let empty_overlay = OverlayManager::new();
    let lens_res = handle_code_lens(&lens_req, Some(ws), &empty_overlay, &mut conv);
    let lenses = lens_res.as_array().expect("code lens array");
    assert!(!lenses.is_empty(), "Code lenses must be generated for definitions");
    assert!(lenses[0]["data"].is_object(), "Unresolved code lens must carry data payload");

    let resolve_req = json!({ "params": &lenses[0] });
    let resolved = handle_code_lens_resolve(&resolve_req, Some(ws), &empty_overlay, &mut conv);
    assert!(!resolved.is_null(), "codeLens/resolve returns params");
    assert!(resolved["command"]["title"].as_str().map(|t| t.contains("reference") || t.contains("References")).unwrap_or(false), "Resolved code lens must carry reference title");
    assert_eq!(resolved["command"]["command"], "ridx.showReferences", "Code lens command must match registered ridx.showReferences");

    // 4. Code lens with a dirty overlay: the buffer is the indexed text with two lines inserted
    // at the top, opened as `didOpen` opens it. The lenses are the index's, two lines down, and
    // resolve maps them back onto the indexed text.
    let mut dirty_overlay = OverlayManager::new();
    let lib_path = lib_file.to_str().unwrap();
    let dirty_code = format!("// Header line\n\n{}", std::fs::read_to_string(&lib_file).unwrap());
    dirty_overlay.open_with_source_hash(&uri, lib_path, 1, dirty_code, ws.source_hash_for_file(lib_path));
    let dirty_lenses = handle_code_lens(&lens_req, Some(ws), &dirty_overlay, &mut conv);
    let dirty_arr = dirty_lenses.as_array().unwrap();
    assert_eq!(dirty_arr.len(), lenses.len(), "every lens of the closed file stands in the dirty buffer");
    for (closed, dirty) in lenses.iter().zip(dirty_arr) {
        assert_eq!(dirty["data"]["name"], closed["data"]["name"]);
        assert_eq!(dirty["range"]["start"]["character"], closed["range"]["start"]["character"]);
        assert_eq!(dirty["range"]["start"]["line"].as_u64().unwrap(), closed["range"]["start"]["line"].as_u64().unwrap() + 2);
        assert_eq!(dirty["data"]["line"].as_u64().unwrap(), closed["data"]["line"].as_u64().unwrap() + 2);
    }
    let dirty_resolved = handle_code_lens_resolve(&json!({ "params": &dirty_arr[0] }), Some(ws), &dirty_overlay, &mut conv);
    assert!(dirty_resolved["command"]["title"].as_str().is_some(), "Dirty overlay code lens must resolve cleanly");

    // 5. Folding range ignores braces in strings and comments
    let tricky_file = fixture_dir("traits_and_generics").join("src/tricky_fold.rs");
    let tricky_code = "fn dummy() {\n    let s = \"{ not a block }\";\n    // { also not a block }\n    let c = '{';\n}\n";
    let _ = std::fs::write(&tricky_file, tricky_code);
    let tricky_uri = path_to_uri(tricky_file.to_str().unwrap());
    let tricky_req = json!({ "params": { "textDocument": { "uri": &tricky_uri } } });
    let tricky_res = handle_folding_range(&tricky_req, &empty_overlay);
    let _ = std::fs::remove_file(&tricky_file);
    let tricky_folds = tricky_res.as_array().unwrap();
    assert_eq!(tricky_folds.len(), 1, "Only outer function brace should form a folding range, got: {tricky_folds:?}");
    assert_eq!(tricky_folds[0]["startLine"], 0);
    assert_eq!(tricky_folds[0]["endLine"], 4);
}

#[test]
fn test_l2_presentation_colors_and_links() {
    let lib_file = fixture_dir("traits_and_generics").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();

    let dummy_req = json!({
        "params": {
            "textDocument": { "uri": &uri }
        }
    });

    let colors = handle_document_color(&dummy_req);
    assert_eq!(colors, json!([]), "Document colors returns empty array");

    let color_pres = handle_color_presentation(&dummy_req);
    assert_eq!(color_pres, json!([]), "Color presentation returns empty array");

    let links = handle_document_link(&dummy_req, &overlay_mgr);
    assert!(links.is_array(), "Document links must return array");

    let sel_range = handle_selection_range(&dummy_req, &overlay_mgr);
    assert_eq!(sel_range, json!([]), "Selection range without overlay returns empty");
}

#[test]
fn test_l2_code_lens_command_declared_in_execute_command_provider() {
    let ws = get_or_index_l2_fixture("traits_and_generics");
    let lib_file = fixture_dir("traits_and_generics").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let mut conv = PositionConverter::new();
    let overlay_mgr = OverlayManager::new();

    let lens_req = json!({ "params": { "textDocument": { "uri": &uri } } });
    let lenses = handle_code_lens(&lens_req, Some(ws), &overlay_mgr, &mut conv);
    let lens_arr = lenses.as_array().expect("lenses array");
    assert!(!lens_arr.is_empty());

    let resolve_req = json!({ "params": &lens_arr[0] });
    let resolved = handle_code_lens_resolve(&resolve_req, Some(ws), &overlay_mgr, &mut conv);
    let cmd = resolved["command"]["command"].as_str().expect("lens command");

    let mut lc = ridx::lsp::lifecycle::LifecycleManager::new();
    let init_req = json!({ "params": { "capabilities": {} } });
    let (init_res, _) = lc.handle_initialize(&init_req).expect("initialize result");
    let exec_cmds = init_res["capabilities"]["executeCommandProvider"]["commands"].as_array().expect("executeCommandProvider.commands");
    let has_cmd = exec_cmds.iter().any(|c| c.as_str() == Some(cmd));
    assert!(has_cmd, "Code lens command '{cmd}' must be declared in executeCommandProvider.commands: {exec_cmds:?}");
}
