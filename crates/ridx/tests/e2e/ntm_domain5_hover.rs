/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/hover request (Domain 5: Intelligence & Completion).

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
        "id": 30,
        "method": "textDocument/hover",
        "params": params
    });
    let mut out = Vec::new();
    let _ = dispatch_lsp_message(LspMessage::Client(req), &mut out, lc, mr, ov, can, None);
    extract_response(&out)
}

#[test]
fn test_hover_fn_definition_shows_signature() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/hover_fn.rs";
    open_test_file(&mut ov, uri, 1, "pub fn calculate_area(width: u32, height: u32) -> u64 { (width as u64) * (height as u64) }\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 10 }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["contents"]["kind"], "markdown");
    assert_eq!(
        res["result"]["contents"]["value"],
        "```rust\nfn calculate_area(width: u32, height: u32) -> u64\n```"
    );
}

#[test]
fn test_hover_struct_definition_shows_struct_name() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/hover_struct.rs";
    open_test_file(&mut ov, uri, 1, "pub struct GeoCoordinate {\n    pub lat: f64,\n    pub lon: f64,\n}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 15 }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["contents"]["kind"], "markdown");
    assert_eq!(
        res["result"]["contents"]["value"],
        "```rust\nstruct GeoCoordinate\n```"
    );
}

#[test]
fn test_hover_enum_definition_shows_enum_name() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/hover_enum.rs";
    open_test_file(&mut ov, uri, 1, "pub enum NetworkProtocol {\n    Tcp,\n    Udp,\n}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 12 }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["contents"]["kind"], "markdown");
    assert_eq!(
        res["result"]["contents"]["value"],
        "```rust\nenum NetworkProtocol\n```"
    );
}

#[test]
fn test_hover_trait_definition_shows_trait_name() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/hover_trait.rs";
    open_test_file(&mut ov, uri, 1, "pub trait StreamDecoder {\n    fn decode(&mut self) -> bool;\n}\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 14 }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["contents"]["kind"], "markdown");
    assert_eq!(
        res["result"]["contents"]["value"],
        "```rust\ntrait StreamDecoder\n```"
    );
}

#[test]
fn test_hover_const_definition_shows_const() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/hover_const.rs";
    open_test_file(&mut ov, uri, 1, "pub const BUFFER_CAPACITY: usize = 4096;\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 14 }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["contents"]["kind"], "markdown");
    assert_eq!(
        res["result"]["contents"]["value"],
        "```rust\nconst BUFFER_CAPACITY\n```"
    );
}

#[test]
fn test_hover_static_definition_shows_static() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/hover_static.rs";
    open_test_file(&mut ov, uri, 1, "pub static GLOBAL_EPOCH: u64 = 1000;\n");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 15 }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert_eq!(res["result"]["contents"]["kind"], "markdown");
    assert_eq!(
        res["result"]["contents"]["value"],
        "```rust\nstatic GLOBAL_EPOCH\n```"
    );
}

#[test]
fn test_hover_empty_document_returns_null() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/hover_empty.rs";
    open_test_file(&mut ov, uri, 1, "");

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 0, "character": 0 }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert!(res["error"].is_null());
    assert!(res["result"].is_null());
}

#[test]
fn test_hover_missing_params_rejected_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let res = send_hover_req(serde_json::Value::Null, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], -32602);
}

#[test]
fn test_hover_missing_text_document_rejected_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "position": { "line": 0, "character": 0 }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], -32602);
}

#[test]
fn test_hover_missing_position_rejected_error() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let params = json!({
        "textDocument": { "uri": "file:///tmp/hover_test.rs" }
    });
    let res = send_hover_req(params, &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["error"]["code"], -32602);
}
