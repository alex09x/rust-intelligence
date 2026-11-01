/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::common::*;
use crate::lsp::*;
use ridx_format::writer::{IndexWriter, RawDefinition, RawType};
use ridx_format::DefKind;
use serde_json::json;
use std::io::Cursor;

#[test]
fn test_sync_reloaded_index_advances_compiled_baseline() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_sync_reload_baseline_{}", std::process::id()));
    let _ = std::fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    let initial_content = "pub fn original() {}\n";
    let _ = std::fs::write(&src_file, initial_content);

    let ridx_file = temp_dir.join("index.ridx");
    let initial_hash = ridx_format::normalized_source_hash(initial_content.as_bytes());

    let mut writer = IndexWriter::new("sync_crate", "0.1.0");
    writer.get_or_insert_file(src_file.to_str().unwrap());
    writer.add_file_hash(src_file.to_str().unwrap(), initial_hash);
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "original".to_string(),
        path: "sync_crate::original".to_string(),
        signature: Some("fn()".to_string()),
        file: src_file.to_str().unwrap().to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 20,
    });
    {
        let mut f = std::fs::File::create(&ridx_file).unwrap();
        writer.write_to(&mut f).unwrap();
    }

    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let mut overlay_mgr = crate::overlay::OverlayManager::new();
    let uri = format!("file://{}", src_file.display());
    overlay_mgr.open_document(&uri, src_file.to_str().unwrap(), 1, initial_content.to_string());
    assert!(!overlay_mgr.get(src_file.to_str().unwrap()).unwrap().is_dirty());

    // User edits document: now dirty
    let updated_content = "pub fn original() {}\npub fn updated() {}\n";
    overlay_mgr.update(src_file.to_str().unwrap(), 2, updated_content.to_string());
    assert!(overlay_mgr.get(src_file.to_str().unwrap()).unwrap().is_dirty());

    // Save edited content to disk
    let _ = std::fs::write(&src_file, updated_content);
    let updated_hash = ridx_format::normalized_source_hash(updated_content.as_bytes());

    // Rebuild and update the .ridx file with the new hash
    std::thread::sleep(std::time::Duration::from_millis(50));
    let mut writer2 = IndexWriter::new("sync_crate", "0.1.0");
    writer2.get_or_insert_file(src_file.to_str().unwrap());
    writer2.add_file_hash(src_file.to_str().unwrap(), updated_hash);
    writer2.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "updated".to_string(),
        path: "sync_crate::updated".to_string(),
        signature: Some("fn()".to_string()),
        file: src_file.to_str().unwrap().to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 2,
        end_col: 20,
    });
    {
        let mut f = std::fs::File::create(&ridx_file).unwrap();
        writer2.write_to(&mut f).unwrap();
    }

    // Now call sync_reloaded_index!
    sync_reloaded_index(&mut ws, &mut overlay_mgr);

    // Overlay compiled baseline should now be advanced, so is_dirty() is false!
    assert!(!overlay_mgr.get(src_file.to_str().unwrap()).unwrap().is_dirty());

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_lsp_server_stream_cancellation_and_shutdown() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_lsp_stream_test_{}", std::process::id()));
    let _ = std::fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    let _ = std::fs::write(&src_file, "pub fn stream_fn() {}\n");
    let src_uri = format!("file://{}", src_file.display());

    let mut input = Vec::new();

    // 1. Request before initialize: rejected with ServerNotInitialized (-32002)
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": src_uri },
            "position": { "line": 0, "character": 0 }
        }
    })));

    // 2. Initialize request
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "initialize",
        "params": {
            "capabilities": {},
            "rootUri": format!("file://{}", temp_dir.display())
        }
    })));

    // 3. Initialized notification
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "initialized",
        "params": {}
    })));

    // 4. didChange on document not opened yet (falls back to reading from disk)
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": src_uri, "version": 2 },
            "contentChanges": [{ "text": "pub fn changed_fn() {}\n" }]
        }
    })));

    // 5. Cancel request: send request with id 3, followed by $/cancelRequest with id 3
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "textDocument/definition",
        "params": {
            "textDocument": { "uri": src_uri },
            "position": { "line": 0, "character": 7 }
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "$/cancelRequest",
        "params": { "id": 3 }
    })));

    // 6. workspace/didRenameFiles
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "workspace/didRenameFiles",
        "params": {
            "files": [{
                "oldUri": format!("file://{}/old.rs", temp_dir.display()),
                "newUri": format!("file://{}/new.rs", temp_dir.display())
            }]
        }
    })));

    // 7. workspace/didDeleteFiles
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "workspace/didDeleteFiles",
        "params": {
            "files": [{
                "uri": format!("file://{}/new.rs", temp_dir.display())
            }]
        }
    })));

    // 8. shutdown request
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "shutdown",
        "params": null
    })));

    // 9. Request after shutdown: rejected with Server is shutting down (-32600)
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 5,
        "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": src_uri },
            "position": { "line": 0, "character": 0 }
        }
    })));

    // 10. exit notification
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "exit"
    })));

    let mut output = Vec::new();
    let res = run_lsp_server_stream(Cursor::new(input), &mut output, None);
    assert!(res.is_ok());

    let responses = parse_lsp_responses(&output);
    assert_eq!(responses[0]["id"], 1);
    assert_eq!(responses[0]["error"]["code"], -32002);
    assert_eq!(responses[1]["id"], 2);
    assert!(responses[1]["result"]["capabilities"].is_object());
    assert_eq!(responses[2]["id"], 3);
    assert_eq!(responses[2]["error"]["code"], -32800);
    assert_eq!(responses[3]["id"], 4);
    assert!(responses[3]["result"].is_null());
    assert_eq!(responses[4]["id"], 5);
    assert_eq!(responses[4]["error"]["code"], -32600);

    // Also test exit notification before shutdown returns Error
    let mut exit_early_input = Vec::new();
    exit_early_input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "exit"
    })));
    let mut out_early = Vec::new();
    let res_early = run_lsp_server_stream(Cursor::new(exit_early_input), &mut out_early, None);
    assert!(res_early.is_err());
    assert!(res_early.unwrap_err().to_string().contains("exit notification received before shutdown"));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_lsp_hover_inferred_type_and_signature_help_workspace() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_lsp_type_sig_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    let file_path = temp_dir.join("lib.rs");
    let file_str = file_path.to_str().unwrap().to_string();
    let code = "pub fn caller() {\n    calculate(42);\n}\n";
    std::fs::write(&file_path, code).unwrap();

    let index_file = temp_dir.join("test_crate.ridx");
    let mut writer = IndexWriter::new("test_crate", "0.1.0");
    writer.set_unit_key("unit-sig-1");
    writer.set_generation("gen-sig-1");
    let mut hashes = std::collections::HashMap::new();
    hashes.insert(file_str.clone(), ridx_format::normalized_source_hash(code.as_bytes()));
    writer.set_file_hashes(hashes);

    // Definition for calculate with signature
    writer.add_definition(RawDefinition {
        def_id: 100,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "calculate".to_string(),
        path: "test_crate::calculate".to_string(),
        signature: Some("fn(x: i32) -> i32".to_string()),
        file: file_str.clone(),
        start_line: 10,
        start_col: 1,
        end_line: 10,
        end_col: 25,
    });

    // Inferred type at line 1, col 5 (over "fn", no definition)
    writer.add_type(RawType { file: file_str.clone(), start_line: 1, start_col: 5, end_line: 1, end_col: 7, type_name: "fn() -> ()".to_string() });

    let mut out_f = std::fs::File::create(&index_file).unwrap();
    writer.write_to(&mut out_f).unwrap();

    let doc_uri = format!("file://{}", file_str);
    let mut input = Vec::new();
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "rootUri": format!("file://{}", temp_dir.display()),
            "capabilities": {}
        }
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": &doc_uri,
                "languageId": "rust",
                "version": 1,
                "text": code
            }
        }
    })));

    // 1. Hover over inferred type without definition (lines 80-88 of intelligence.rs)
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 10,
        "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": &doc_uri },
            "position": { "line": 0, "character": 5 }
        }
    })));

    // 2. Signature help inside calculate(42) finding calculate in workspace (lines 310-319 of intelligence.rs)
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 11,
        "method": "textDocument/signatureHelp",
        "params": {
            "textDocument": { "uri": &doc_uri },
            "position": { "line": 1, "character": 15 }
        }
    })));

    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "id": 12,
        "method": "shutdown",
        "params": null
    })));
    input.extend(make_lsp_msg(&json!({
        "jsonrpc": "2.0",
        "method": "exit"
    })));

    let mut output = Vec::new();
    let res = run_lsp_server_stream(Cursor::new(input), &mut output, Some(index_file));
    assert!(res.is_ok());

    let responses = parse_lsp_responses(&output);
    assert_eq!(responses.len(), 4);

    let hover_resp = &responses[1];
    assert_eq!(hover_resp["id"], 10);
    assert!(hover_resp["result"]["contents"]["value"].as_str().unwrap().contains("Type: fn() -> ()"));

    let sig_resp = &responses[2];
    assert_eq!(sig_resp["id"], 11);
    assert!(sig_resp["result"]["signatures"][0]["label"].as_str().unwrap().contains("fn calculate(x: i32) -> i32"));

    let _ = std::fs::remove_dir_all(&temp_dir);
}
