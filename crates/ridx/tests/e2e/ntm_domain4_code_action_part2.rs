/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! End-to-end tests for `textDocument/codeAction` (Part 2: parameter validation, organizeImports, refactor.extract, quickfix basics).

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::error::INVALID_PARAMS;
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
fn test_code_action_missing_params_rejected_code_32602() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0",
        "id": 100,
        "method": "textDocument/codeAction"
    });
    dispatch_lsp_message(LspMessage::Client(req), &mut out, &mut lc, &mut mr, &mut ov, &can, None).unwrap();
    let res = extract_response(&out);
    assert_eq!(res["id"], 100);
    assert_eq!(res["error"]["code"], INVALID_PARAMS);
    assert_eq!(res["error"]["message"], "Missing params object");
}

#[test]
fn test_code_action_missing_uri_rejected_code_32602() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "textDocument": {},
        "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
        "context": { "diagnostics": [] }
    });
    let res = send_code_action_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["id"], 100);
    assert_eq!(res["error"]["code"], INVALID_PARAMS);
    assert_eq!(res["error"]["message"], "Missing textDocument.uri");
}

#[test]
fn test_code_action_empty_uri_rejected_code_32602() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "textDocument": { "uri": "" },
        "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
        "context": { "diagnostics": [] }
    });
    let res = send_code_action_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["id"], 100);
    assert_eq!(res["error"]["code"], INVALID_PARAMS);
    assert_eq!(res["error"]["message"], "Empty textDocument.uri");
}

#[test]
fn test_code_action_missing_range_rejected_code_32602() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "textDocument": { "uri": "file:///tmp/action_test.rs" },
        "context": { "diagnostics": [] }
    });
    let res = send_code_action_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["id"], 100);
    assert_eq!(res["error"]["code"], INVALID_PARAMS);
    assert_eq!(res["error"]["message"], "Missing range object");
}

#[test]
fn test_code_action_empty_buffer_returns_empty_actions() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/empty_action.rs";
    let file = "/tmp/empty_action.rs";
    ov.open_with_provenance(uri, file, 1, "".to_string(), true);

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
fn test_code_action_clean_doc_no_diagnostics_returns_empty_actions() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/clean_action.rs";
    let file = "/tmp/clean_action.rs";
    ov.open_with_provenance(uri, file, 1, "pub fn hello() -> u32 {\n    42\n}\n".to_string(), true);

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
fn test_code_action_organize_imports_single_line() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/org_imports.rs";
    let file = "/tmp/org_imports.rs";
    ov.open_with_provenance(uri, file, 1, "use b::B;\nuse a::A;\n\nfn main() {}\n".to_string(), true);

    let params = json!({
        "textDocument": { "uri": uri },
        "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
        "context": { "diagnostics": [] }
    });
    let res = send_code_action_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["id"], 100);
    let actions = res["result"].as_array().expect("result array");
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0]["title"], "Organize Imports");
    assert_eq!(actions[0]["kind"], "source.organizeImports");
}

#[test]
fn test_code_action_organize_imports_with_duplicates() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/dup_imports.rs";
    let file = "/tmp/dup_imports.rs";
    ov.open_with_provenance(uri, file, 1, "use std::fmt;\nuse std::fmt;\n".to_string(), true);

    let params = json!({
        "textDocument": { "uri": uri },
        "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
        "context": { "diagnostics": [] }
    });
    let res = send_code_action_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["id"], 100);
    let actions = res["result"].as_array().expect("result array");
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0]["title"], "Organize Imports");
}

#[test]
fn test_code_action_organize_imports_filter_only_source_organize_imports() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/filter_org.rs";
    let file = "/tmp/filter_org.rs";
    ov.open_with_provenance(uri, file, 1, "use y::Y;\nuse x::X;\n".to_string(), true);

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
fn test_code_action_organize_imports_filtered_out_by_only_quickfix() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/filter_out_org.rs";
    let file = "/tmp/filter_out_org.rs";
    ov.open_with_provenance(uri, file, 1, "use y::Y;\nuse x::X;\n".to_string(), true);

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
fn test_code_action_extract_variable_non_empty_range() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/extract_var.rs";
    let file = "/tmp/extract_var.rs";
    ov.open_with_provenance(uri, file, 1, "fn calc() -> i32 {\n    1 + 2\n}\n".to_string(), true);

    let params = json!({
        "textDocument": { "uri": uri },
        "range": { "start": { "line": 1, "character": 4 }, "end": { "line": 1, "character": 9 } },
        "context": { "only": ["refactor.extract"], "diagnostics": [] }
    });
    let res = send_code_action_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["id"], 100);
    let actions = res["result"].as_array().expect("result array");
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0]["title"], "Extract variable");
    assert_eq!(actions[0]["kind"], "refactor.extract");
}

#[test]
fn test_code_action_extract_variable_empty_range_yields_no_action() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/extract_empty.rs";
    let file = "/tmp/extract_empty.rs";
    ov.open_with_provenance(uri, file, 1, "fn calc() -> i32 {\n    1 + 2\n}\n".to_string(), true);

    let params = json!({
        "textDocument": { "uri": uri },
        "range": { "start": { "line": 1, "character": 4 }, "end": { "line": 1, "character": 4 } },
        "context": { "only": ["refactor.extract"], "diagnostics": [] }
    });
    let res = send_code_action_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["id"], 100);
    assert_eq!(res["result"], json!([]));
}

#[test]
fn test_code_action_extract_variable_filtered_out_by_only_quickfix() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/extract_filtered.rs";
    let file = "/tmp/extract_filtered.rs";
    ov.open_with_provenance(uri, file, 1, "fn calc() -> i32 {\n    1 + 2\n}\n".to_string(), true);

    let params = json!({
        "textDocument": { "uri": uri },
        "range": { "start": { "line": 1, "character": 4 }, "end": { "line": 1, "character": 9 } },
        "context": { "only": ["quickfix"], "diagnostics": [] }
    });
    let res = send_code_action_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["id"], 100);
    assert_eq!(res["result"], json!([]));
}

#[test]
fn test_code_action_quickfix_unused_variable_prefix_underscore() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/qf_unused_var.rs";
    let file = "/tmp/qf_unused_var.rs";
    ov.open_with_provenance(uri, file, 1, "fn main() {\n    let val = 10;\n}\n".to_string(), true);

    let diag = json!({
        "range": { "start": { "line": 1, "character": 8 }, "end": { "line": 1, "character": 11 } },
        "message": "unused variable: `val`",
        "code": "unused_variable"
    });
    let params = json!({
        "textDocument": { "uri": uri },
        "range": { "start": { "line": 1, "character": 8 }, "end": { "line": 1, "character": 11 } },
        "context": { "only": ["quickfix"], "diagnostics": [diag] }
    });
    let res = send_code_action_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["id"], 100);
    let actions = res["result"].as_array().expect("result array");
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0]["title"], "Prefix with underscore: `_val`");
    assert_eq!(actions[0]["kind"], "quickfix");
}

#[test]
fn test_code_action_quickfix_unused_variable_message_fallback() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/qf_msg_fallback.rs";
    let file = "/tmp/qf_msg_fallback.rs";
    ov.open_with_provenance(uri, file, 1, "fn main() {\n    let x = 1;\n}\n".to_string(), true);

    let diag = json!({
        "range": { "start": { "line": 1, "character": 8 }, "end": { "line": 1, "character": 9 } },
        "message": "warning: unused variable: `x`",
        "code": ""
    });
    let params = json!({
        "textDocument": { "uri": uri },
        "range": { "start": { "line": 1, "character": 8 }, "end": { "line": 1, "character": 9 } },
        "context": { "only": ["quickfix"], "diagnostics": [diag] }
    });
    let res = send_code_action_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["id"], 100);
    let actions = res["result"].as_array().expect("result array");
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0]["title"], "Prefix with underscore: `_x`");
    assert_eq!(actions[0]["kind"], "quickfix");
}
