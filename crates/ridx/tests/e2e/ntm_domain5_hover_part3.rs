/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/hover request part 3 (Domain 5: Intelligence & Completion).

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

fn open_test_file(ov: &mut OverlayManager, uri: &str, version: i32, text: &str) {
    let file = ridx::lsp::protocol::uri_to_path(uri);
    ov.open_or_update(uri, &file, version, text.to_string());
}

fn send_hover_req(
    params: serde_json::Value,
    mr: &mut MultiRootWorkspace,
    ov: &mut OverlayManager,
    lc: &mut LifecycleManager,
    can: &Arc<Mutex<HashSet<serde_json::Value>>>,
) -> serde_json::Value {
    let req = json!({
        "jsonrpc": "2.0",
        "id": 32,
        "method": "textDocument/hover",
        "params": params
    });
    let mut out = Vec::new();
    let _ = dispatch_lsp_message(LspMessage::Client(req), &mut out, lc, mr, ov, can, None);
    extract_response(&out)
}

#[test]
fn test_hover_fn_parameter_shows_parameter_signature() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/hover_param.rs";
    open_test_file(&mut ov, uri, 1, "fn compute(factor: i64) -> i64 { factor * 2 }\n");

    // Position on `factor` in parameters
    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 12 }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["contents"]["kind"], "markdown");
    assert_eq!(
        res["result"]["contents"]["value"],
        "```rust\nfactor: i64\n```"
    );
}

#[test]
fn test_hover_nonexistent_file_returns_null() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "textDocument": { "uri": "file:///tmp/ghost_file.rs" },
        "position": { "line": 0, "character": 0 }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert!(res["result"].is_null());
}

#[test]
fn test_hover_file_scheme_uri_normalized() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/normal_file.rs";
    open_test_file(&mut ov, uri, 1, "pub fn normalized_call() {}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 10 }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(
        res["result"]["contents"]["value"],
        "```rust\nfn normalized_call()\n```"
    );
}

#[test]
fn test_hover_pub_crate_fn_shows_fn_name() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/hover_vis.rs";
    open_test_file(&mut ov, uri, 1, "pub(crate) fn crate_internal() {}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 16 }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(
        res["result"]["contents"]["value"],
        "```rust\nfn crate_internal()\n```"
    );
}

#[test]
fn test_hover_async_fn_shows_fn_name() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/hover_async.rs";
    open_test_file(&mut ov, uri, 1, "pub async fn async_fetch() {}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 18 }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(
        res["result"]["contents"]["value"],
        "```rust\nfn async_fetch()\n```"
    );
}

#[test]
fn test_hover_unsafe_fn_shows_fn_name() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/hover_unsafe.rs";
    open_test_file(&mut ov, uri, 1, "pub unsafe fn raw_operation() {}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 18 }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(
        res["result"]["contents"]["value"],
        "```rust\nfn raw_operation()\n```"
    );
}

#[test]
fn test_hover_generic_fn_shows_fn_name() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/hover_generic.rs";
    open_test_file(&mut ov, uri, 1, "pub fn transform<T: Clone>(item: T) -> T { item }\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 10 }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(
        res["result"]["contents"]["value"],
        "```rust\nfn transform<T: Clone>(item: T) -> T\n```"
    );
}

#[test]
fn test_hover_impl_method_shows_fn_name() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/hover_impl.rs";
    open_test_file(&mut ov, uri, 1, "struct Driver;\nimpl Driver {\n    pub fn start(&self) {}\n}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 2, "character": 12 }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(
        res["result"]["contents"]["value"],
        "```rust\nfn start(&self)\n```"
    );
}

#[test]
fn test_hover_empty_result_is_json_null() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/hover_none.rs";
    open_test_file(&mut ov, uri, 1, "fn existing() {}\n\n\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 2, "character": 0 }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert!(res["result"].is_null());
}

#[test]
fn test_hover_character_zero_before_ident_returns_null() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/hover_before.rs";
    open_test_file(&mut ov, uri, 1, "    fn indented() {}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 0 }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert!(res["result"].is_null());
}
