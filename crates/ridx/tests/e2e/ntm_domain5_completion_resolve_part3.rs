/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for completionItem/resolve request part 3 (Domain 5: Intelligence & Completion).

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
fn test_completion_resolve_sort_text_preserved() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(
        json!({ "label": "sorted_fn", "sortText": "0001" }),
        &mut mr, &mut ov, &mut lc, &can,
    );
    assert_eq!(val["result"]["sortText"], "0001");
}

#[test]
fn test_completion_resolve_preselect_preserved() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(
        json!({ "label": "preselected_fn", "preselect": true }),
        &mut mr, &mut ov, &mut lc, &can,
    );
    assert_eq!(val["result"]["preselect"], true);
}

#[test]
fn test_completion_resolve_detail_without_data_produces_codeblock() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(
        json!({
            "label": "generic_fn",
            "detail": "pub fn foo<T: Clone>(x: T) -> Result<T, ()>"
        }),
        &mut mr, &mut ov, &mut lc, &can,
    );
    assert_eq!(val["result"]["documentation"]["kind"], "markdown");
    assert_eq!(
        val["result"]["documentation"]["value"],
        "```rust\npub fn foo<T: Clone>(x: T) -> Result<T, ()>\n```"
    );
}

#[test]
fn test_completion_resolve_data_string_not_wrapped_in_codeblock() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(
        json!({
            "label": "markdown_doc_item",
            "data": "# Section Header\nDetailed explanation."
        }),
        &mut mr, &mut ov, &mut lc, &can,
    );
    assert_eq!(val["result"]["documentation"]["kind"], "markdown");
    assert_eq!(
        val["result"]["documentation"]["value"],
        "# Section Header\nDetailed explanation."
    );
}

#[test]
fn test_completion_resolve_both_detail_and_data_detail_takes_precedence() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(
        json!({
            "label": "dual_item",
            "detail": "pub struct Dual;",
            "data": "fallback documentation"
        }),
        &mut mr, &mut ov, &mut lc, &can,
    );
    assert_eq!(val["result"]["documentation"]["kind"], "markdown");
    assert_eq!(
        val["result"]["documentation"]["value"],
        "```rust\npub struct Dual;\n```"
    );
}

#[test]
fn test_completion_resolve_plain_text_existing_documentation_preserved() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(
        json!({
            "label": "plain_item",
            "detail": "fn detail_to_ignore()",
            "documentation": {
                "kind": "plaintext",
                "value": "This is raw documentation text."
            }
        }),
        &mut mr, &mut ov, &mut lc, &can,
    );
    assert_eq!(val["result"]["documentation"]["kind"], "plaintext");
    assert_eq!(
        val["result"]["documentation"]["value"],
        "This is raw documentation text."
    );
}

#[test]
fn test_completion_resolve_string_existing_documentation_preserved() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(
        json!({
            "label": "string_doc_item",
            "detail": "fn ignored()",
            "documentation": "String doc format without object wrapper"
        }),
        &mut mr, &mut ov, &mut lc, &can,
    );
    assert_eq!(
        val["result"]["documentation"],
        "String doc format without object wrapper"
    );
}

#[test]
fn test_completion_resolve_null_detail_falls_back_to_data() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(
        json!({
            "label": "fallback_item",
            "detail": null,
            "data": "Fallback markdown text"
        }),
        &mut mr, &mut ov, &mut lc, &can,
    );
    assert_eq!(val["result"]["documentation"]["kind"], "markdown");
    assert_eq!(val["result"]["documentation"]["value"], "Fallback markdown text");
}

#[test]
fn test_completion_resolve_empty_detail_produces_empty_codeblock() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(
        json!({
            "label": "empty_detail_item",
            "detail": ""
        }),
        &mut mr, &mut ov, &mut lc, &can,
    );
    assert_eq!(val["result"]["documentation"]["kind"], "markdown");
    assert_eq!(val["result"]["documentation"]["value"], "```rust\n\n```");
}

#[test]
fn test_completion_resolve_whitespace_only_label_preserved() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let val = send_resolve_req(
        json!({ "label": "   " }),
        &mut mr, &mut ov, &mut lc, &can,
    );
    assert_eq!(val["result"]["label"], "   ");
}
