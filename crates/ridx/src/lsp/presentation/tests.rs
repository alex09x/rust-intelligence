/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::*;
use crate::lsp::protocol::PositionConverter;
use crate::overlay::OverlayManager;
use ridx_format::{IndexWriter, RawDefinition, WorkspaceIndex};
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

fn setup_test_presentation() -> (PathBuf, WorkspaceIndex, OverlayManager, PositionConverter) {
    let id = TEST_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_dir =
        std::env::temp_dir().join(format!("ridx_test_pres_{}_{}", std::process::id(), id));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    let code =
        "// link https://example.com/test\npub fn hello() {\n    let a = 1;\n    let b = 2;\n}\n";
    let _ = fs::write(&src_file, code);

    let mut writer = IndexWriter::new("test_pres", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: ridx_format::DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        file: "src/lib.rs".to_string(),
        name: "hello".to_string(),
        start_line: 2,
        start_col: 8,
        end_line: 2,
        end_col: 13,
        path: "test_pres::hello".to_string(),
        signature: Some("fn hello()".to_string()),
    });

    let ridx_file = temp_dir.join("index.ridx");
    let mut f = fs::File::create(&ridx_file).unwrap();
    writer.write_to(&mut f).unwrap();

    let ws = WorkspaceIndex::open(&ridx_file).unwrap();
    let overlay_mgr = OverlayManager::new();
    let conv = PositionConverter::new();

    (temp_dir, ws, overlay_mgr, conv)
}

#[test]
fn test_presentation_handlers_comprehensive() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_presentation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    let code = "// link https://example.com/test\npub fn hello() {\n    let a = 1;\n    let b = 2;\n}\nmod my_mod {\n    struct MyStruct; enum MyEnum { A }\n    trait MyTrait {}\n    const MY_CONST: i32 = 1;\n}\n";
    overlay_mgr.open_document(&uri, file_path.to_str().unwrap(), 1, code.to_string());
    if let Some(doc) = overlay_mgr.get_mut(file_path.to_str().unwrap()) {
        doc.symbols.push(crate::overlay::OverlaySymbol {
            name: "multiline_symbol".to_string(),
            kind: ridx_format::DefKind::Struct,
            signature: None,
            start_line: 12,
            start_col: 1,
            end_line: 15,
            end_col: 1,
            scope_id: 0,
            decl_start_byte: 0,
            decl_end_byte: 10,
            scope_start_byte: 0,
        });
    }

    // 1. Selection range
    let sel_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "positions": [{ "line": 2, "character": 8 }]
        }
    });
    let sel_res = handle_selection_range(&sel_req, &overlay_mgr);
    assert!(!sel_res.as_array().unwrap().is_empty());

    // 2. Document link
    let link_req = json!({
        "params": {
            "textDocument": { "uri": &uri }
        }
    });
    let link_res = handle_document_link(&link_req, &overlay_mgr);
    assert!(!link_res.as_array().unwrap().is_empty());
    let resolve_res = handle_document_link_resolve(&link_req);
    assert!(!resolve_res.is_null());

    // 3. Semantic tokens
    let sem_res = handle_semantic_tokens_full(&link_req, Some(&ws), &overlay_mgr, &mut conv);
    assert!(sem_res["data"].as_array().is_some());
    let prev_result_id = sem_res["resultId"].as_str().unwrap().to_string();

    let delta_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "previousResultId": prev_result_id
        }
    });
    let delta_res = handle_semantic_tokens_delta(&delta_req, Some(&ws), &overlay_mgr, &mut conv);
    assert_eq!(delta_res["edits"].as_array().unwrap().len(), 0);

    let range_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 3, "character": 0 }
            }
        }
    });
    let range_res = handle_semantic_tokens_range(&range_req, Some(&ws), &overlay_mgr, &mut conv);
    assert!(range_res["data"].as_array().is_some());

    // 4. Folding range
    let fold_res = handle_folding_range(&link_req, &overlay_mgr);
    assert!(fold_res.as_array().is_some());

    // 5. Code lens
    let lens_res = handle_code_lens(&link_req, Some(&ws), &overlay_mgr, &mut conv);
    assert!(lens_res.as_array().is_some());
    let first_lens = &lens_res.as_array().unwrap()[0];
    let lens_resolve = handle_code_lens_resolve(&json!({ "params": first_lens }), Some(&ws), &overlay_mgr, &mut conv);
    assert!(!lens_resolve.is_null());
    assert_eq!(lens_resolve["command"]["command"], "ridx.showReferences");

    // 6. Moniker
    let mon_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 8 }
        }
    });
    let mon_res = handle_moniker(&mon_req, Some(&ws), &overlay_mgr, &mut conv);
    assert!(mon_res.as_array().is_some());

    // 7. Text document content
    let content_req = json!({ "params": { "uri": &uri } });
    let content_res = handle_text_document_content(&content_req, &overlay_mgr);
    assert!(content_res["text"].as_str().unwrap().contains("hello"));

    // 8. Colors
    assert_eq!(handle_document_color(&link_req), json!([]));
    assert_eq!(handle_color_presentation(&link_req), json!([]));

    // 9. Diagnostics: valid syntax returns empty items
    let diag_res = handle_diagnostic(&link_req, &overlay_mgr);
    assert_eq!(diag_res["kind"], "full");
    assert_eq!(diag_res["items"].as_array().unwrap().len(), 0);

    // Diagnostics: invalid syntax reports error
    let invalid_file = temp_dir.join("src/invalid.rs");
    let invalid_uri = format!("file://{}", invalid_file.to_str().unwrap());
    overlay_mgr.open_document(
        &invalid_uri,
        invalid_file.to_str().unwrap(),
        1,
        "fn broken() {\n".to_string(),
    );
    let invalid_req = json!({
        "params": { "textDocument": { "uri": &invalid_uri } }
    });
    let diag_invalid = handle_diagnostic(&invalid_req, &overlay_mgr);
    assert!(!diag_invalid["items"].as_array().unwrap().is_empty());
    assert_eq!(diag_invalid["items"][0]["severity"], 1);

    // Diagnostics: delimiter edge cases
    let delim_tests = [
        ("fn f() { let x = [1, 2); }", "mismatched closing delimiter"),
        ("let x = 1);", "unmatched closing delimiter ')'"),
        ("let x = 1];", "unmatched closing delimiter ']'"),
        ("let x = 1};", "unmatched closing delimiter '}'"),
        ("let s = \"unclosed string;\n", "unclosed string literal"),
    ];
    for (code_sample, expected_msg) in delim_tests {
        let f = temp_dir.join("src/delim.rs");
        let u = format!("file://{}", f.to_str().unwrap());
        overlay_mgr.open_document(&u, f.to_str().unwrap(), 1, code_sample.to_string());
        let res = handle_diagnostic(
            &json!({ "params": { "textDocument": { "uri": &u } } }),
            &overlay_mgr,
        );
        let items = res["items"].as_array().unwrap();
        assert!(
            items
                .iter()
                .any(|item| item["message"].as_str().unwrap().contains(expected_msg)),
            "Expected '{}' in diagnostics for '{}'",
            expected_msg,
            code_sample
        );
    }

    // Diagnostics and links for missing file
    let missing_uri = "file:///nonexistent/file.rs";
    let missing_req = json!({ "params": { "textDocument": { "uri": missing_uri } } });
    assert_eq!(
        handle_diagnostic(&missing_req, &overlay_mgr)["items"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(handle_document_link(&missing_req, &overlay_mgr), json!([]));
    assert_eq!(
        handle_selection_range(&missing_req, &overlay_mgr),
        json!([])
    );
    assert_eq!(
        handle_moniker(&missing_req, Some(&ws), &overlay_mgr, &mut conv),
        json!([])
    );

    // Document link, diagnostic, and content from disk file (not in overlay)
    let disk_file = temp_dir.join("src/disk.rs");
    let _ = fs::write(
        &disk_file,
        "// see https://example.com/disk\nfn disk() { let _arr = [1, 2]; }\n",
    );
    let disk_uri = format!("file://{}", disk_file.to_str().unwrap());
    let disk_req = json!({ "params": { "textDocument": { "uri": &disk_uri }, "uri": &disk_uri } });
    let disk_diag = handle_diagnostic(&disk_req, &overlay_mgr);
    assert_eq!(disk_diag["items"].as_array().unwrap().len(), 0);
    let disk_links = handle_document_link(&disk_req, &overlay_mgr);
    assert_eq!(disk_links.as_array().unwrap().len(), 1);
    let disk_content = handle_text_document_content(&disk_req, &overlay_mgr);
    assert!(disk_content["text"].as_str().unwrap().contains("disk"));

    // Semantic tokens, folding range, and code lens when document is not in overlay
    assert_eq!(
        handle_semantic_tokens_full(&missing_req, Some(&ws), &overlay_mgr, &mut conv)["data"],
        json!([])
    );
    assert_eq!(handle_folding_range(&missing_req, &overlay_mgr), json!([]));
    assert_eq!(handle_code_lens(&missing_req, Some(&ws), &overlay_mgr, &mut conv), json!([]));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_workspace_diagnostic_discovers_closed_files() {
    let (temp_dir, _ws, overlay_mgr, _) = setup_test_presentation();
    let src_file = temp_dir.join("src/lib.rs");
    fs::write(&src_file, "fn broken( {").unwrap();

    let mut multi_root = crate::lsp::workspace::MultiRootWorkspace::new();
    let ridx_file = temp_dir.join("index.ridx");
    multi_root.add_root(ridx_file);

    let req = json!({ "params": {} });
    let res = handle_workspace_diagnostic(&req, &overlay_mgr, &multi_root);
    let items = res["items"].as_array().expect("items array");
    assert!(
        !items.is_empty(),
        "Must discover closed file in workspace index"
    );
    let err_item = items
        .iter()
        .find(|it| it["uri"].as_str().unwrap().contains("lib.rs"));
    assert!(
        err_item.is_some(),
        "Must report diagnostic for closed lib.rs"
    );
    let report = err_item.unwrap();
    assert!(
        !report["items"].as_array().unwrap().is_empty(),
        "Must contain syntax error for broken closed file"
    );

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_workspace_diagnostic_filters_external_dependencies() {
    use crate::lsp::protocol::path_to_uri;

    let id = TEST_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_dir =
        std::env::temp_dir().join(format!("ridx_test_ws_diag_ext_{}_{}", std::process::id(), id));

    // 1. Workspace member crate
    let ws_crate_dir = temp_dir.join("my_crate");
    fs::create_dir_all(ws_crate_dir.join("src")).unwrap();
    let ws_file = ws_crate_dir.join("src/lib.rs");
    fs::write(&ws_file, "pub fn ws_fn() {}\n").unwrap();
    let ws_ridx = ws_crate_dir.join("index.ridx");
    let mut ws_writer = IndexWriter::new("my_crate", "0.1.0");
    ws_writer.add_definition(RawDefinition {
        def_id: 1, kind: ridx_format::DefKind::Fn, visibility: 0, is_macro_expansion: false,
        file: "src/lib.rs".to_string(), name: "ws_fn".to_string(),
        start_line: 1, start_col: 8, end_line: 1, end_col: 13,
        path: "my_crate::ws_fn".to_string(), signature: Some("fn ws_fn()".to_string()),
    });
    let mut f = fs::File::create(&ws_ridx).unwrap();
    ws_writer.write_to(&mut f).unwrap();

    // 2. External dependency crate (simulated cargo registry)
    let dep_crate_dir = temp_dir.join(".cargo/registry/src/index.crates.io-1234/dep_crate-0.1.0");
    fs::create_dir_all(dep_crate_dir.join("src")).unwrap();
    let dep_file = dep_crate_dir.join("src/lib.rs");
    fs::write(&dep_file, "pub fn dep_fn() {}\n").unwrap();
    let dep_ridx = dep_crate_dir.join("index.ridx");
    let mut dep_writer = IndexWriter::new("dep_crate", "0.1.0");
    dep_writer.add_definition(RawDefinition {
        def_id: 2, kind: ridx_format::DefKind::Fn, visibility: 0, is_macro_expansion: false,
        file: "src/lib.rs".to_string(), name: "dep_fn".to_string(),
        start_line: 1, start_col: 8, end_line: 1, end_col: 14,
        path: "dep_crate::dep_fn".to_string(), signature: Some("fn dep_fn()".to_string()),
    });
    let mut f = fs::File::create(&dep_ridx).unwrap();
    dep_writer.write_to(&mut f).unwrap();

    let mut multi_root = crate::lsp::workspace::MultiRootWorkspace::new();
    multi_root.add_root(ws_ridx);
    multi_root.add_root(dep_ridx);

    let overlay_mgr = OverlayManager::new();
    let req = json!({ "params": {} });
    let res = handle_workspace_diagnostic(&req, &overlay_mgr, &multi_root);
    let items = res["items"].as_array().expect("items array");
    let mut uris: Vec<_> = items.iter().map(|it| it["uri"].as_str().unwrap().to_string()).collect();
    uris.sort();

    let expected_ws_uri = path_to_uri(&ws_file.to_string_lossy());
    assert_eq!(uris, vec![expected_ws_uri]);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_workspace_diagnostic_excludes_external_files_in_workspace_reader() {
    use crate::lsp::protocol::path_to_uri;

    let id = TEST_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_dir = std::env::temp_dir()
        .join(format!("ridx_test_ws_diag_macro_{}_{}", std::process::id(), id));

    let ws_crate_dir = temp_dir.join("ws_crate");
    fs::create_dir_all(ws_crate_dir.join("src")).unwrap();
    let ws_file = ws_crate_dir.join("src/lib.rs");
    fs::write(&ws_file, "pub fn a() {}\n").unwrap();

    let ext_dir = temp_dir.join(".cargo/registry/src/index.crates.io-1234/dep-1.0.0/src");
    fs::create_dir_all(&ext_dir).unwrap();
    let ext_file = ext_dir.join("lib.rs");
    fs::write(&ext_file, "pub fn b() {}\n").unwrap();

    let ws_ridx = ws_crate_dir.join("index.ridx");
    let mut writer = IndexWriter::new("ws_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: ridx_format::DefKind::Fn, visibility: 0, is_macro_expansion: false,
        file: "src/lib.rs".to_string(), name: "a".to_string(),
        start_line: 1, start_col: 8, end_line: 1, end_col: 9,
        path: "ws_crate::a".to_string(), signature: None,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: ridx_format::DefKind::Fn, visibility: 0, is_macro_expansion: true,
        file: ext_file.to_string_lossy().to_string(), name: "b".to_string(),
        start_line: 1, start_col: 8, end_line: 1, end_col: 9,
        path: "ws_crate::b".to_string(), signature: None,
    });
    let mut f = fs::File::create(&ws_ridx).unwrap();
    writer.write_to(&mut f).unwrap();

    let mut multi_root = crate::lsp::workspace::MultiRootWorkspace::new();
    multi_root.add_root(ws_ridx);

    let overlay_mgr = OverlayManager::new();
    let req = json!({ "params": {} });
    let res = handle_workspace_diagnostic(&req, &overlay_mgr, &multi_root);
    let items = res["items"].as_array().expect("items array");
    let uris: Vec<_> = items.iter().map(|it| it["uri"].as_str().unwrap().to_string()).collect();

    let expected_uri = path_to_uri(&ws_file.to_string_lossy());
    assert_eq!(uris, vec![expected_uri]);

    let _ = fs::remove_dir_all(&temp_dir);
}
