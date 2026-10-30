/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! End-to-end tests for `textDocument/codeAction` (Part 3: unused mut, multiple diagnostics, overlay/disk lifecycle, data payload).

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::LspMessage;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

fn extract_response(out: &[u8]) -> Value {
    let s = String::from_utf8_lossy(out);
    for part in s.split("Content-Length: ") {
        if let Some(idx) = part.find("\r\n\r\n") {
            let body = &part[idx + 4..];
            if let Ok(v) = serde_json::from_str::<Value>(body.trim()) {
                return v;
            }
        }
    }
    Value::Null
}

fn setup_initialized_server() -> (MultiRootWorkspace, OverlayManager, LifecycleManager, Arc<Mutex<HashSet<Value>>>) {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let init = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "capabilities": {} }
    });
    dispatch_lsp_message(LspMessage::Client(init), &mut out, &mut lc, &mut mr, &mut ov, &can, None).unwrap();
    (mr, ov, lc, can)
}

fn send_code_action(
    uri: &str,
    range: Value,
    ctx: Value,
    mr: &mut MultiRootWorkspace,
    ov: &mut OverlayManager,
    lc: &mut LifecycleManager,
    can: &Arc<Mutex<HashSet<Value>>>,
) -> Value {
    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0", "id": 100, "method": "textDocument/codeAction",
        "params": { "textDocument": { "uri": uri }, "range": range, "context": ctx }
    });
    dispatch_lsp_message(LspMessage::Client(req), &mut out, lc, mr, ov, can, None).unwrap();
    extract_response(&out)
}

fn send_code_action_req(params: Value, mr: &mut MultiRootWorkspace, ov: &mut OverlayManager, lc: &mut LifecycleManager, can: &Arc<Mutex<HashSet<Value>>>) -> Value {
    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0",
        "id": 100,
        "method": "textDocument/codeAction",
        "params": params
    });
    dispatch_lsp_message(LspMessage::Client(req), &mut out, lc, mr, ov, can, None).unwrap();
    extract_response(&out)
}

#[test]
fn test_code_action_quickfix_unused_mut_remove_mut() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/unused_mut.rs";
    let file = "/tmp/unused_mut.rs";
    ov.open_with_provenance(uri, file, 1, "fn main() {\n    let mut x = 1;\n}\n".to_string(), true);

    let range = json!({ "start": { "line": 1, "character": 8 }, "end": { "line": 1, "character": 13 } });
    let diag = json!({ "range": range, "message": "variable does not need to be mutable", "code": "unused_mut" });
    let res = send_code_action(uri, range, json!({ "only": ["quickfix"], "diagnostics": [diag] }), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["id"], 100);
    let actions = res["result"].as_array().expect("result array");
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0]["title"], "Remove unnecessary `mut`");
    assert_eq!(actions[0]["kind"], "quickfix");
}

#[test]
fn test_code_action_quickfix_unused_mut_message_fallback() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/unused_mut_msg.rs";
    let file = "/tmp/unused_mut_msg.rs";
    ov.open_with_provenance(uri, file, 1, "fn main() {\n    let mut y = 2;\n}\n".to_string(), true);

    let range = json!({ "start": { "line": 1, "character": 8 }, "end": { "line": 1, "character": 13 } });
    let diag = json!({ "range": range, "message": "warning: variable does not need to be mutable: `y`", "code": "" });
    let res = send_code_action(uri, range, json!({ "only": ["quickfix"], "diagnostics": [diag] }), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["id"], 100);
    let actions = res["result"].as_array().expect("result array");
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0]["title"], "Remove unnecessary `mut`");
}

#[test]
fn test_code_action_quickfix_multiple_diagnostics_emits_multiple_actions() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/multi_diag.rs";
    let file = "/tmp/multi_diag.rs";
    ov.open_with_provenance(uri, file, 1, "fn main() {\n    let mut z = 3;\n}\n".to_string(), true);

    let range = json!({ "start": { "line": 1, "character": 8 }, "end": { "line": 1, "character": 13 } });
    let diag1 = json!({ "range": range, "message": "unused variable: `z`", "code": "unused_variable" });
    let diag2 = json!({ "range": range, "message": "variable does not need to be mutable", "code": "unused_mut" });
    let res = send_code_action(uri, range, json!({ "only": ["quickfix"], "diagnostics": [diag1, diag2] }), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["id"], 100);
    let actions = res["result"].as_array().expect("result array");
    assert_eq!(actions.len(), 2);
    assert_eq!(actions[0]["title"], "Prefix with underscore: `_z`");
    assert_eq!(actions[1]["title"], "Remove unnecessary `mut`");
}

#[test]
fn test_code_action_quickfix_filtered_out_by_only_refactor() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/filtered_qf.rs";
    let file = "/tmp/filtered_qf.rs";
    ov.open_with_provenance(uri, file, 1, "fn main() {\n    let a = 1;\n}\n".to_string(), true);

    let diag = json!({
        "range": { "start": { "line": 1, "character": 8 }, "end": { "line": 1, "character": 9 } },
        "message": "unused variable: `a`",
        "code": "unused_variable"
    });
    let params = json!({
        "textDocument": { "uri": uri },
        "range": { "start": { "line": 1, "character": 8 }, "end": { "line": 1, "character": 9 } },
        "context": { "only": ["refactor.extract"], "diagnostics": [diag] }
    });
    let res = send_code_action_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["id"], 100);
    let actions = res["result"].as_array().expect("result array");
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0]["kind"], "refactor.extract");
    assert_eq!(actions[0]["title"], "Extract variable");
}

#[test]
fn test_code_action_quickfix_preserves_diagnostic_range() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/preserve_range.rs";
    let file = "/tmp/preserve_range.rs";
    ov.open_with_provenance(uri, file, 1, "fn main() {\n    let var = 1;\n}\n".to_string(), true);

    let expected_range = json!({ "start": { "line": 1, "character": 8 }, "end": { "line": 1, "character": 11 } });
    let diag = json!({
        "range": expected_range,
        "message": "unused variable: `var`",
        "code": "unused_variable"
    });
    let params = json!({
        "textDocument": { "uri": uri },
        "range": expected_range,
        "context": { "only": ["quickfix"], "diagnostics": [diag] }
    });
    let res = send_code_action_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["id"], 100);
    let actions = res["result"].as_array().expect("result array");
    let edit = &actions[0]["edit"]["changes"][uri][0];
    assert_eq!(edit["range"], expected_range);
    assert_eq!(edit["newText"], "_var");
}

#[test]
fn test_code_action_quickfix_preserves_document_version_in_edit() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/doc_ver.rs";
    let file = "/tmp/doc_ver.rs";
    ov.open_with_provenance(uri, file, 4, "fn main() {\n    let test_var = 1;\n}\n".to_string(), true);

    let diag = json!({
        "range": { "start": { "line": 1, "character": 8 }, "end": { "line": 1, "character": 16 } },
        "message": "unused variable: `test_var`",
        "code": "unused_variable"
    });
    let params = json!({
        "textDocument": { "uri": uri },
        "range": { "start": { "line": 1, "character": 8 }, "end": { "line": 1, "character": 16 } },
        "context": { "only": ["quickfix"], "diagnostics": [diag] }
    });
    let res = send_code_action_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["id"], 100);
    let actions = res["result"].as_array().expect("result array");
    assert_eq!(actions[0]["data"]["version"], 4);
}

#[test]
fn test_code_action_overlay_text_used_over_disk() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/ov_over_disk.rs";
    let file = "/tmp/ov_over_disk.rs";
    ov.open_with_provenance(uri, file, 1, "use z::Z;\nuse a::A;\n".to_string(), true);

    let params = json!({
        "textDocument": { "uri": uri },
        "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
        "context": { "only": ["source.organizeImports"], "diagnostics": [] }
    });
    let res = send_code_action_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["id"], 100);
    let actions = res["result"].as_array().expect("result array");
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0]["kind"], "source.organizeImports");
}

#[test]
fn test_code_action_did_change_updates_available_actions() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/did_change_action.rs";
    let file = "/tmp/did_change_action.rs";
    ov.open_with_provenance(uri, file, 1, "fn main() {}\n".to_string(), true);

    // Initial check -> empty actions
    let params1 = json!({
        "textDocument": { "uri": uri },
        "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
        "context": { "only": ["source.organizeImports"], "diagnostics": [] }
    });
    let res1 = send_code_action_req(params1, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res1["result"], json!([]));

    // didChange applies unsorted imports
    let change = json!({
        "text": "use k::K;\nuse j::J;\n"
    });
    ov.apply_content_changes(file, 2, &[change]);

    // Second check -> organize imports action available
    let params2 = json!({
        "textDocument": { "uri": uri },
        "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
        "context": { "only": ["source.organizeImports"], "diagnostics": [] }
    });
    let res2 = send_code_action_req(params2, &mut mr, &mut ov, &mut lc, &can);
    let actions = res2["result"].as_array().expect("result array");
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0]["title"], "Organize Imports");
}

#[test]
fn test_code_action_did_close_reverts_to_disk_content() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/did_close_action.rs";
    let file = "/tmp/did_close_action.rs";
    ov.open_with_provenance(uri, file, 1, "use b::B;\nuse a::A;\n".to_string(), true);
    assert_eq!(ov.get(file).map(|d| d.version), Some(1));

    ov.close(file);
    assert_eq!(ov.get(file).map(|d| d.version), None);

    let params = json!({
        "textDocument": { "uri": uri },
        "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
        "context": { "diagnostics": [] }
    });
    let res = send_code_action_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["id"], 100);
    assert_eq!(res["result"], json!([]));
}

#[test]
fn test_code_action_data_carries_content_hash() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/content_hash.rs";
    let file = "/tmp/content_hash.rs";
    let code = "use q::Q;\nuse p::P;\n".to_string();
    let expected_hash = ridx_format::normalized_source_hash(code.as_bytes());
    ov.open_with_provenance(uri, file, 1, code, true);

    let params = json!({
        "textDocument": { "uri": uri },
        "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
        "context": { "only": ["source.organizeImports"], "diagnostics": [] }
    });
    let res = send_code_action_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["id"], 100);
    let actions = res["result"].as_array().expect("result array");
    assert_eq!(actions[0]["data"]["content_hash"], json!(expected_hash));
}

#[test]
fn test_code_action_data_carries_uri() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/data_uri.rs";
    let file = "/tmp/data_uri.rs";
    ov.open_with_provenance(uri, file, 1, "use m::M;\nuse l::L;\n".to_string(), true);

    let params = json!({
        "textDocument": { "uri": uri },
        "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
        "context": { "only": ["source.organizeImports"], "diagnostics": [] }
    });
    let res = send_code_action_req(params, &mut mr, &mut ov, &mut lc, &can);
    let actions = res["result"].as_array().expect("result array");
    assert_eq!(actions[0]["data"]["uri"], uri);
}

#[test]
fn test_code_action_non_existent_file_with_no_diagnostics_returns_empty() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/definitely_non_existent_code_action_file.rs";
    let params = json!({
        "textDocument": { "uri": uri },
        "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
        "context": { "diagnostics": [] }
    });
    let res = send_code_action_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["id"], 100);
    assert_eq!(res["result"], json!([]));
}

#[test]
fn test_code_action_multiline_selection_range() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/multiline_action.rs";
    let file = "/tmp/multiline_action.rs";
    ov.open_with_provenance(uri, file, 1, "fn f() {\n    let a = 1;\n    let b = 2;\n}\n".to_string(), true);

    let params = json!({
        "textDocument": { "uri": uri },
        "range": { "start": { "line": 1, "character": 4 }, "end": { "line": 2, "character": 14 } },
        "context": { "diagnostics": [] }
    });
    let res = send_code_action_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["id"], 100);
    let actions = res["result"].as_array().expect("result array");
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0]["kind"], "refactor.extract");
    assert_eq!(actions[0]["title"], "Extract variable");
}

#[test]
fn test_code_action_empty_diagnostics_array_in_context() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/empty_diag.rs";
    let file = "/tmp/empty_diag.rs";
    ov.open_with_provenance(uri, file, 1, "fn main() {}\n".to_string(), true);

    let params = json!({
        "textDocument": { "uri": uri },
        "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
        "context": { "only": ["quickfix"], "diagnostics": [] }
    });
    let res = send_code_action_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["id"], 100);
    assert_eq!(res["result"], json!([]));
}

#[test]
fn test_code_action_unknown_diagnostic_code_ignored() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/unknown_diag.rs";
    let file = "/tmp/unknown_diag.rs";
    ov.open_with_provenance(uri, file, 1, "fn main() {}\n".to_string(), true);

    let diag = json!({
        "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
        "message": "custom warning message",
        "code": "E9999_unknown"
    });
    let params = json!({
        "textDocument": { "uri": uri },
        "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
        "context": { "only": ["quickfix"], "diagnostics": [diag] }
    });
    let res = send_code_action_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["id"], 100);
    assert_eq!(res["result"], json!([]));
}
