/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for completionItem/resolve request part 2 (Domain 5: Intelligence & Completion).

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::LspMessage;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use serde_json::json;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

fn extract_response(out: &[u8]) -> serde_json::Value {
    let s = String::from_utf8_lossy(out);
    for part in s.split("Content-Length: ") {
        if let Some(idx) = part.find("\r\n\r\n") {
            let body = &part[idx + 4..];
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(body.trim()) {
                return v;
            }
        }
    }
    serde_json::Value::Null
}

fn setup_initialized_server() -> (
    MultiRootWorkspace,
    OverlayManager,
    LifecycleManager,
    Arc<Mutex<HashSet<serde_json::Value>>>,
) {
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

fn send_resolve_req(
    params: serde_json::Value,
    mr: &mut MultiRootWorkspace,
    ov: &mut OverlayManager,
    lc: &mut LifecycleManager,
    can: &Arc<Mutex<HashSet<serde_json::Value>>>,
) -> serde_json::Value {
    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0",
        "id": 20,
        "method": "completionItem/resolve",
        "params": params
    });
    let _ = dispatch_lsp_message(LspMessage::Client(req), &mut out, lc, mr, ov, can, None);
    extract_response(&out)
}

#[test]
fn test_completion_resolve_filter_text_preserved() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(
        json!({ "label": "test_fn", "filterText": "test" }),
        &mut mr, &mut ov, &mut lc, &can,
    );
    assert_eq!(val["result"]["filterText"], "test");
}

#[test]
fn test_completion_resolve_insert_text_format_preserved() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(
        json!({ "label": "snippet", "insertTextFormat": 2 }),
        &mut mr, &mut ov, &mut lc, &can,
    );
    assert_eq!(val["result"]["insertTextFormat"], 2);
}

#[test]
fn test_completion_resolve_text_edit_preserved() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let edit = json!({
        "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 4 } },
        "newText": "replacement"
    });
    let val = send_resolve_req(
        json!({ "label": "edit_item", "textEdit": edit }),
        &mut mr, &mut ov, &mut lc, &can,
    );
    assert_eq!(val["result"]["textEdit"]["newText"], "replacement");
}

#[test]
fn test_completion_resolve_additional_text_edits_preserved() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(
        json!({
            "label": "import_item",
            "additionalTextEdits": [
                {
                    "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
                    "newText": "use std::collections::HashMap;\n"
                }
            ]
        }),
        &mut mr, &mut ov, &mut lc, &can,
    );
    let edits = val["result"]["additionalTextEdits"].as_array().unwrap();
    assert_eq!(edits.len(), 1);
}

#[test]
fn test_completion_resolve_command_preserved() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let cmd = json!({ "title": "Trigger Parameter Hints", "command": "editor.action.triggerParameterHints" });
    let val = send_resolve_req(
        json!({ "label": "with_cmd", "command": cmd }),
        &mut mr, &mut ov, &mut lc, &can,
    );
    assert_eq!(val["result"]["command"]["title"], "Trigger Parameter Hints");
}

#[test]
fn test_completion_resolve_commit_characters_preserved() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(
        json!({ "label": "chars_item", "commitCharacters": [".", "("] }),
        &mut mr, &mut ov, &mut lc, &can,
    );
    let chars = val["result"]["commitCharacters"].as_array().unwrap();
    assert_eq!(chars.len(), 2);
}

#[test]
fn test_completion_resolve_deprecated_preserved() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(
        json!({ "label": "old_fn", "deprecated": true }),
        &mut mr, &mut ov, &mut lc, &can,
    );
    assert_eq!(val["result"]["deprecated"], true);
}

#[test]
fn test_completion_resolve_tags_preserved() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(
        json!({ "label": "tagged_item", "tags": [1] }),
        &mut mr, &mut ov, &mut lc, &can,
    );
    let tags = val["result"]["tags"].as_array().unwrap();
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0], 1);
}

#[test]
fn test_completion_resolve_complex_data_object_preserved() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let complex_data = json!({ "symbol_id": 12345, "crate": "core" });
    let val = send_resolve_req(
        json!({ "label": "complex", "data": complex_data }),
        &mut mr, &mut ov, &mut lc, &can,
    );
    assert_eq!(val["result"]["data"]["symbol_id"], 12345);
}

#[test]
fn test_completion_resolve_idempotent_re_resolve() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let initial = json!({ "label": "fn_name", "detail": "fn fn_name()" });
    let val1 = send_resolve_req(initial, &mut mr, &mut ov, &mut lc, &can);
    let val2 = send_resolve_req(val1["result"].clone(), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(val1["result"], val2["result"]);
}
