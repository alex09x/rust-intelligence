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
use ridx_format::writer::{IndexWriter, RawDefinition, RawReference, RawType};
use ridx_format::DefKind;
use serde_json::json;
use std::io::Cursor;

#[test]
fn test_lsp_server_stream_stage8_full_workspace_dispatch() {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id_num = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temp_dir = std::env::temp_dir().join(format!("ridx_stream_stage8_{}_{}", std::process::id(), id_num));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    let file_path = temp_dir.join("lib.rs");
    let file_str = file_path.to_str().unwrap().to_string();
    let code = "pub struct Calculator;\n\nimpl Calculator {\n    pub fn calculate(x: i32) -> i32 {\n        x + 1\n    }\n}\n";
    std::fs::write(&file_path, code).unwrap();

    let index_file = temp_dir.join("test_crate.ridx");
    let mut writer = IndexWriter::new("test_crate", "0.1.0");
    writer.set_unit_key("unit-test-1");
    writer.set_generation("gen-test-1");
    let mut hashes = std::collections::HashMap::new();
    hashes.insert(file_str.clone(), ridx_format::normalized_source_hash(code.as_bytes()));
    writer.set_file_hashes(hashes);

    writer.add_definition(RawDefinition {
        def_id: 101,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "Calculator".to_string(),
        path: "test_crate::Calculator".to_string(),
        signature: None,
        file: file_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 22,
    });
    writer.add_definition(RawDefinition {
        def_id: 100,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "calculate".to_string(),
        path: "test_crate::Calculator::calculate".to_string(),
        signature: Some("fn(i32) -> i32".to_string()),
        file: file_str.clone(),
        start_line: 4,
        start_col: 5,
        end_line: 4,
        end_col: 36,
    });
    writer.add_definition(RawDefinition {
        def_id: 102,
        kind: DefKind::Impl,
        visibility: 0,
        is_macro_expansion: false,
        name: "Calculator".to_string(),
        path: "test_crate::Calculator".to_string(),
        signature: None,
        file: file_str.clone(),
        start_line: 3,
        start_col: 1,
        end_line: 3,
        end_col: 16,
    });
    writer.add_reference(RawReference {
        file: file_str.clone(),
        start_line: 5,
        start_col: 9,
        end_line: 5,
        end_col: 10,
        target_def_id: Some(100),
        target_path: "test_crate::Calculator::calculate".to_string(),
        target_file: Some(file_str.clone()),
        target_unit: Some("unit-test-1".to_string()),
        target_line: 4,
        target_col: 12,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.add_type(RawType { file: file_str.clone(), start_line: 4, start_col: 22, end_line: 4, end_col: 23, type_name: "i32".to_string() });

    let mut out_file = std::fs::File::create(&index_file).unwrap();
    writer.write_to(&mut out_file).unwrap();
    drop(out_file);

    let doc_uri = format!("file://{}", file_str);
    let mut input = Vec::new();

    // 0. Request before initialize -> returns -32002 Server not initialized
    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 999, "method": "textDocument/hover", "params": {} })));

    input.extend(make_lsp_msg(
        &json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "workspaceFolders": [ { "uri": format!("file://{}", temp_dir.to_str().unwrap()), "name": "test_ws" } ], "capabilities": {} } }),
    ));
    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} })));

    // Cancel request -> returns -32800 Request cancelled. The cancel follows its request, as in the protocol (a cancel of
    // a request not sent is ignored): a request waiting (`minVersion` on a file no index holds) is still pending then.
    let pending_uri = "file:///tmp/ridx_stage8_cancel_pending.rs";
    input.extend(make_lsp_msg(
        &json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": { "textDocument": { "uri": pending_uri, "languageId": "rust", "version": 1, "text": "fn f() {}\n" } } }),
    ));
    input.extend(make_lsp_msg(
        &json!({ "jsonrpc": "2.0", "id": 55, "method": "textDocument/documentSymbol", "params": { "textDocument": { "uri": pending_uri }, "minVersion": 1 } }),
    ));
    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "method": "$/cancelRequest", "params": { "id": 55 } })));

    input.extend(make_lsp_msg(
        &json!({ "jsonrpc": "2.0", "method": "workspace/didChangeWorkspaceFolders", "params": { "event": { "added": [{ "uri": format!("file://{}", temp_dir.to_str().unwrap()), "name": "ws2" }], "removed": [] } } }),
    ));

    input.extend(make_lsp_msg(
        &json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": { "textDocument": { "uri": doc_uri, "languageId": "rust", "version": 1, "text": code } } }),
    ));

    input.extend(make_lsp_msg(
        &json!({ "jsonrpc": "2.0", "method": "textDocument/didChange", "params": { "textDocument": { "uri": doc_uri, "version": 2 }, "contentChanges": [{ "text": code }] } }),
    ));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "method": "textDocument/didSave", "params": { "textDocument": { "uri": doc_uri } } })));

    input.extend(make_lsp_msg(
        &json!({ "jsonrpc": "2.0", "id": 10, "method": "textDocument/definition", "params": { "textDocument": { "uri": doc_uri }, "position": { "line": 3, "character": 12 } } }),
    ));

    input.extend(make_lsp_msg(
        &json!({ "jsonrpc": "2.0", "id": 11, "method": "textDocument/declaration", "params": { "textDocument": { "uri": doc_uri }, "position": { "line": 3, "character": 12 } } }),
    ));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 12, "method": "textDocument/typeDefinition", "params": { "textDocument": { "uri": doc_uri }, "position": { "line": 3, "character": 12 } } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 13, "method": "textDocument/implementation", "params": { "textDocument": { "uri": doc_uri }, "position": { "line": 0, "character": 12 } } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 14, "method": "textDocument/references", "params": { "textDocument": { "uri": doc_uri }, "position": { "line": 3, "character": 12 }, "context": { "includeDeclaration": true } } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 15, "method": "textDocument/documentHighlight", "params": { "textDocument": { "uri": doc_uri }, "position": { "line": 3, "character": 12 } } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 16, "method": "workspace/symbol", "params": { "query": "calc" } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 17, "method": "workspaceSymbol/resolve", "params": { "name": "calculate", "kind": 12 } })));

    input.extend(make_lsp_msg(
        &json!({ "jsonrpc": "2.0", "id": 18, "method": "textDocument/hover", "params": { "textDocument": { "uri": doc_uri }, "position": { "line": 3, "character": 12 } } }),
    ));

    input.extend(make_lsp_msg(
        &json!({ "jsonrpc": "2.0", "id": 19, "method": "textDocument/completion", "params": { "textDocument": { "uri": doc_uri }, "position": { "line": 3, "character": 12 } } }),
    ));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 20, "method": "completionItem/resolve", "params": { "label": "calculate" } })));

    input.extend(make_lsp_msg(
        &json!({ "jsonrpc": "2.0", "id": 21, "method": "textDocument/signatureHelp", "params": { "textDocument": { "uri": doc_uri }, "position": { "line": 4, "character": 9 } } }),
    ));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 22, "method": "textDocument/formatting", "params": { "textDocument": { "uri": doc_uri }, "options": { "tabSize": 4, "insertSpaces": true } } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 23, "method": "textDocument/prepareRename", "params": { "textDocument": { "uri": doc_uri }, "position": { "line": 3, "character": 12 } } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 24, "method": "textDocument/rename", "params": { "textDocument": { "uri": doc_uri }, "position": { "line": 3, "character": 12 }, "newName": "do_calc" } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 25, "method": "textDocument/prepareCallHierarchy", "params": { "textDocument": { "uri": doc_uri }, "position": { "line": 3, "character": 12 } } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 26, "method": "callHierarchy/incomingCalls", "params": { "item": { "name": "calculate", "kind": 12, "uri": doc_uri, "range": { "start": { "line": 3, "character": 4 }, "end": { "line": 3, "character": 35 } }, "selectionRange": { "start": { "line": 3, "character": 11 }, "end": { "line": 3, "character": 20 } } } } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 27, "method": "callHierarchy/outgoingCalls", "params": { "item": { "name": "calculate", "kind": 12, "uri": doc_uri, "range": { "start": { "line": 3, "character": 4 }, "end": { "line": 3, "character": 35 } }, "selectionRange": { "start": { "line": 3, "character": 11 }, "end": { "line": 3, "character": 20 } } } } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 28, "method": "textDocument/prepareTypeHierarchy", "params": { "textDocument": { "uri": doc_uri }, "position": { "line": 0, "character": 12 } } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 29, "method": "typeHierarchy/supertypes", "params": { "item": { "name": "Calculator", "kind": 23, "uri": doc_uri, "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 22 } }, "selectionRange": { "start": { "line": 0, "character": 11 }, "end": { "line": 0, "character": 21 } } } } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 30, "method": "typeHierarchy/subtypes", "params": { "item": { "name": "Calculator", "kind": 23, "uri": doc_uri, "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 22 } }, "selectionRange": { "start": { "line": 0, "character": 11 }, "end": { "line": 0, "character": 21 } } } } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 31, "method": "textDocument/selectionRange", "params": { "textDocument": { "uri": doc_uri }, "positions": [{ "line": 3, "character": 12 }] } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 32, "method": "textDocument/documentLink", "params": { "textDocument": { "uri": doc_uri } } })));
    input.extend(make_lsp_msg(
        &json!({ "jsonrpc": "2.0", "id": 33, "method": "documentLink/resolve", "params": { "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 10 } }, "target": "https://example.com" } }),
    ));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 34, "method": "textDocument/semanticTokens/full", "params": { "textDocument": { "uri": doc_uri } } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 35, "method": "textDocument/foldingRange", "params": { "textDocument": { "uri": doc_uri } } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 36, "method": "textDocument/codeAction", "params": { "textDocument": { "uri": doc_uri }, "range": { "start": { "line": 3, "character": 4 }, "end": { "line": 3, "character": 35 } }, "context": { "diagnostics": [] } } })));
    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 37, "method": "codeAction/resolve", "params": { "title": "Quick Fix" } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 38, "method": "textDocument/codeLens", "params": { "textDocument": { "uri": doc_uri } } })));
    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 39, "method": "codeLens/resolve", "params": { "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 10 } } } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 40, "method": "textDocument/inlayHint", "params": { "textDocument": { "uri": doc_uri }, "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 6, "character": 0 } } } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 41, "method": "textDocument/diagnostic", "params": { "textDocument": { "uri": doc_uri } } })));

    input.extend(make_lsp_msg(
        &json!({ "jsonrpc": "2.0", "id": 42, "method": "textDocument/moniker", "params": { "textDocument": { "uri": doc_uri }, "position": { "line": 3, "character": 12 } } }),
    ));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 43, "method": "workspace/executeCommand", "params": { "command": "cargo.reload" } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 44, "method": "textDocument/documentColor", "params": { "textDocument": { "uri": doc_uri } } })));
    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 45, "method": "textDocument/colorPresentation", "params": { "textDocument": { "uri": doc_uri }, "color": { "red": 1.0, "green": 0.0, "blue": 0.0, "alpha": 1.0 }, "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 1 } } } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 46, "method": "textDocument/linkedEditingRange", "params": { "textDocument": { "uri": doc_uri }, "position": { "line": 0, "character": 12 } } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 47, "method": "textDocument/inlineValue", "params": { "textDocument": { "uri": doc_uri }, "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 6, "character": 0 } }, "context": { "frameId": 1, "stoppedLocation": { "line": 3, "character": 4 } } } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 48, "method": "textDocument/inlineCompletion", "params": { "textDocument": { "uri": doc_uri }, "position": { "line": 3, "character": 12 } } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 49, "method": "workspace/textDocumentContent", "params": { "uri": doc_uri } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 53, "method": "textDocument/rangeFormatting", "params": { "textDocument": { "uri": doc_uri }, "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 1, "character": 0 } }, "options": { "tabSize": 4, "insertSpaces": true } } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 54, "method": "textDocument/onTypeFormatting", "params": { "textDocument": { "uri": doc_uri }, "position": { "line": 0, "character": 0 }, "ch": ";", "options": { "tabSize": 4, "insertSpaces": true } } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 56, "method": "textDocument/semanticTokens/range", "params": { "textDocument": { "uri": doc_uri }, "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 5, "character": 0 } } } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 57, "method": "inlayHint/resolve", "params": { "position": { "line": 0, "character": 0 }, "label": ": i32" } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 58, "method": "moniker/resolve", "params": { "scheme": "ridx", "identifier": "calc" } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "method": "textDocument/willSave", "params": { "textDocument": { "uri": doc_uri }, "reason": 1 } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 59, "method": "textDocument/willSaveWaitUntil", "params": { "textDocument": { "uri": doc_uri }, "reason": 1 } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 60, "method": "workspace/configuration", "params": { "items": [{ "section": "rust-analyzer" }] } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "method": "workspace/didCreateFiles", "params": { "files": [{ "uri": doc_uri }] } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 61, "method": "workspace/willCreateFiles", "params": { "files": [{ "uri": doc_uri }] } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 62, "method": "workspace/willRenameFiles", "params": { "files": [{ "oldUri": doc_uri, "newUri": doc_uri }] } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 63, "method": "workspace/willDeleteFiles", "params": { "files": [{ "uri": doc_uri }] } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "method": "$/setTrace", "params": { "value": "verbose" } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "method": "workspace/didChangeConfiguration", "params": { "settings": {} } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "method": "workspace/didRenameFiles", "params": { "files": [{ "oldUri": doc_uri, "newUri": doc_uri }] } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "method": "workspace/didDeleteFiles", "params": { "files": [{ "uri": doc_uri }] } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "method": "window/workDoneProgress/cancel", "params": { "token": "tok" } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 50, "method": "custom/nonExistentMethod", "params": {} })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "method": "textDocument/didClose", "params": { "textDocument": { "uri": doc_uri } } })));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "id": 51, "method": "shutdown", "params": null })));

    input.extend(make_lsp_msg(
        &json!({ "jsonrpc": "2.0", "id": 52, "method": "textDocument/hover", "params": { "textDocument": { "uri": doc_uri }, "position": { "line": 0, "character": 0 } } }),
    ));

    input.extend(make_lsp_msg(&json!({ "jsonrpc": "2.0", "method": "exit" })));

    let mut output = Vec::new();
    let res = run_lsp_server_stream(Cursor::new(input), &mut output, Some(index_file));
    assert!(res.is_ok());

    let responses = parse_lsp_responses(&output);
    assert!(!responses.is_empty());

    let unknown_resp = responses.iter().find(|r| r["id"] == 50);
    assert!(unknown_resp.is_some());
    assert_eq!(unknown_resp.unwrap()["error"]["code"], -32601);

    let after_shutdown_resp = responses.iter().find(|r| r["id"] == 52);
    assert!(after_shutdown_resp.is_some());
    assert_eq!(after_shutdown_resp.unwrap()["error"]["code"], -32600);

    let pre_init_resp = responses.iter().find(|r| r["id"] == 999);
    assert!(pre_init_resp.is_some());
    assert_eq!(pre_init_resp.unwrap()["error"]["code"], -32002);

    let cancel_resp = responses.iter().find(|r| r["id"] == 55);
    assert!(cancel_resp.is_some());
    assert_eq!(cancel_resp.unwrap()["error"]["code"], -32800);

    let sem_range = responses.iter().find(|r| r["id"] == 56);
    assert!(sem_range.is_some() && sem_range.unwrap()["result"]["data"].is_array());

    let inlay_res = responses.iter().find(|r| r["id"] == 57);
    assert!(inlay_res.is_some() && inlay_res.unwrap()["result"]["tooltip"].is_string());

    let moniker_res = responses.iter().find(|r| r["id"] == 58);
    assert!(moniker_res.is_some() && moniker_res.unwrap()["result"]["scheme"] == "ridx");

    let will_save_wait = responses.iter().find(|r| r["id"] == 59);
    assert!(will_save_wait.is_some() && will_save_wait.unwrap()["result"].is_array());

    let ws_config = responses.iter().find(|r| r["id"] == 60);
    assert!(ws_config.is_some() && ws_config.unwrap()["result"].is_array());

    let will_create = responses.iter().find(|r| r["id"] == 61);
    assert!(will_create.is_some() && will_create.unwrap()["result"].is_object());

    let will_rename = responses.iter().find(|r| r["id"] == 62);
    assert!(will_rename.is_some() && will_rename.unwrap()["result"].is_object());

    let will_delete = responses.iter().find(|r| r["id"] == 63);
    assert!(will_delete.is_some() && will_delete.unwrap()["result"].is_object());

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_find_declaration_ident_col_empty_target_and_lifetimes() {
    use crate::lsp::editing::ident::find_declaration_ident_col;
    use ridx_format::DefKind;

    // Must not infinite loop on empty target_name
    assert_eq!(find_declaration_ident_col("pub fn foo() {}", "", DefKind::Fn), None);

    // Test with lifetime in signature
    let line = "pub fn bar<'a, T: 'a>(val: &'a T) -> &'a T";
    assert_eq!(find_declaration_ident_col(line, "bar", DefKind::Fn), Some(8));
    assert_eq!(find_declaration_ident_col(line, "nonexistent", DefKind::Fn), None);
}

#[test]
fn test_dispatch_cancel_request_notification() {
    use crate::lsp::dispatch::dispatch_lsp_message;
    use crate::lsp::lifecycle::LifecycleManager;
    use crate::lsp::protocol::LspMessage;
    use crate::lsp::workspace::MultiRootWorkspace;
    use crate::overlay::OverlayManager;
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex};

    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    // 1. $/cancelRequest without id -> Ok(true), out is empty, cancelled_requests set is empty
    let cancel_no_id = json!({
        "jsonrpc": "2.0",
        "method": "$/cancelRequest",
        "params": {}
    });
    let res = dispatch_lsp_message(LspMessage::Client(cancel_no_id), &mut out, &mut lc, &mut mr, &mut ov, &can, None);
    assert!(res.unwrap());
    assert_eq!(out, b"");
    assert_eq!(*can.lock().unwrap(), HashSet::new());

    // 2. $/cancelRequest with id = 42 -> Ok(true), out is empty, id in set
    let cancel_with_id = json!({
        "jsonrpc": "2.0",
        "method": "$/cancelRequest",
        "params": { "id": 42 }
    });
    let res = dispatch_lsp_message(LspMessage::Client(cancel_with_id), &mut out, &mut lc, &mut mr, &mut ov, &can, None);
    assert!(res.unwrap());
    assert_eq!(out, b"");
    assert_eq!(*can.lock().unwrap(), HashSet::from([json!(42)]));
}
