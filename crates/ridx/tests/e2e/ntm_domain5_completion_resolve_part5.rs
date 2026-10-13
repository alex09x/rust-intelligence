/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for completionItem/resolve request part 5 (Domain 5: Intelligence & Completion).

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
fn test_completion_resolve_unicode_label_preserved() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(
        json!({ "label": "crab_🦀_special" }),
        &mut mr, &mut ov, &mut lc, &can,
    );
    assert_eq!(val["result"]["label"], "crab_🦀_special");
}

#[test]
fn test_completion_resolve_unicode_detail_formatted_in_markdown() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(
        json!({
            "label": "math_symbol",
            "detail": "pub fn alpha_beta_gamma(α: f64, β: f64) -> f64"
        }),
        &mut mr, &mut ov, &mut lc, &can,
    );
    assert_eq!(val["result"]["documentation"]["kind"], "markdown");
    assert_eq!(
        val["result"]["documentation"]["value"],
        "```rust\npub fn alpha_beta_gamma(α: f64, β: f64) -> f64\n```"
    );
}

#[test]
fn test_completion_resolve_escaped_newlines_in_data() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(
        json!({
            "label": "raw_markdown",
            "data": "line 1\r\nline 2\r\nline 3"
        }),
        &mut mr, &mut ov, &mut lc, &can,
    );
    assert_eq!(val["result"]["documentation"]["kind"], "markdown");
    assert_eq!(val["result"]["documentation"]["value"], "line 1\r\nline 2\r\nline 3");
}

#[test]
fn test_completion_resolve_numeric_kind_constants() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    for kind_id in [1, 2, 3, 5, 6, 7, 9, 10, 14, 22] {
        let val = send_resolve_req(
            json!({ "label": "item_kind", "kind": kind_id }),
            &mut mr, &mut ov, &mut lc, &can,
        );
        assert_eq!(val["result"]["kind"], kind_id);
    }
}

#[test]
fn test_completion_resolve_insert_text_mode_preserved() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(
        json!({ "label": "mode_item", "insertTextMode": 2 }),
        &mut mr, &mut ov, &mut lc, &can,
    );
    assert_eq!(val["result"]["insertTextMode"], 2);
}

#[test]
fn test_completion_resolve_label_details_preserved() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(
        json!({
            "label": "collect",
            "labelDetails": {
                "detail": " (from Iterator)",
                "description": "core::iter"
            }
        }),
        &mut mr, &mut ov, &mut lc, &can,
    );
    assert_eq!(val["result"]["labelDetails"]["detail"], " (from Iterator)");
    assert_eq!(val["result"]["labelDetails"]["description"], "core::iter");
}

#[test]
fn test_completion_resolve_multiple_tags_deprecated_preserved() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(
        json!({
            "label": "old_method",
            "tags": [1]
        }),
        &mut mr, &mut ov, &mut lc, &can,
    );
    let tags = val["result"]["tags"].as_array().expect("tags array");
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0], 1);
}

#[test]
fn test_completion_resolve_data_non_string_leaves_documentation_none() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(
        json!({
            "label": "structured_data_item",
            "data": { "crate": "serde", "version": "1.0" }
        }),
        &mut mr, &mut ov, &mut lc, &can,
    );
    assert!(val["result"]["documentation"].is_null());
}

#[test]
fn test_completion_resolve_empty_item_after_completion_workflow() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/bench_workflow.rs";
    let file = ridx::lsp::protocol::uri_to_path(uri);
    ov.open_or_update(uri, &file, 1, "fn sample_workflow() { let counter = 100; c }\n".to_string());

    let mut out = Vec::new();
    let comp_req = json!({
        "jsonrpc": "2.0",
        "id": 10,
        "method": "textDocument/completion",
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": 0, "character": 45 }
        }
    });
    dispatch_lsp_message(LspMessage::Client(comp_req), &mut out, &mut lc, &mut mr, &mut ov, &can, None).unwrap();
    let comp_res = extract_response(&out);
    let items = comp_res["result"]["items"].as_array().expect("items array");
    let counter_item = items.iter().find(|i| i["label"] == "counter").expect("counter item");

    let resolved = send_resolve_req(counter_item.clone(), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(resolved["result"]["label"], "counter");
}

#[test]
fn test_completion_resolve_cancelled_request_returns_cancelled() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    {
        let mut guard = can.lock().unwrap();
        guard.insert(json!(20));
    }
    let val = send_resolve_req(
        json!({ "label": "cancelled_item" }),
        &mut mr, &mut ov, &mut lc, &can,
    );
    assert_eq!(val["error"]["code"], -32800);
}
