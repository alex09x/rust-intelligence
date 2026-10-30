/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for documentLink/resolve and query idempotency (Domain 7, Part 7).
//! Tests resolve target, range, custom data payloads, tooltip preservation, null params, and isolation.

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::path_to_uri;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use serde_json::json;
use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

static DL_P7_COUNTER: AtomicU64 = AtomicU64::new(700);

fn to_uri(p: &Path) -> String {
    path_to_uri(p.to_str().unwrap())
}

struct TestDispatcher {
    lifecycle: LifecycleManager,
    multi_root: MultiRootWorkspace,
    overlay_mgr: OverlayManager,
    cancelled: Arc<Mutex<HashSet<serde_json::Value>>>,
    writer: Vec<u8>,
}

impl TestDispatcher {
    fn new() -> Self {
        let mut d = Self {
            lifecycle: LifecycleManager::new(),
            multi_root: MultiRootWorkspace::new(),
            overlay_mgr: OverlayManager::new(),
            cancelled: Arc::new(Mutex::new(HashSet::new())),
            writer: Vec::new(),
        };
        let init_msg = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } });
        assert!(d.dispatch(&init_msg));
        d.writer.clear();
        d
    }

    fn dispatch(&mut self, msg: &serde_json::Value) -> bool {
        dispatch_lsp_message(msg.clone(), &mut self.writer, &mut self.lifecycle, &mut self.multi_root, &mut self.overlay_mgr, &self.cancelled, None).unwrap()
    }

    fn parse_response(&mut self) -> serde_json::Value {
        let text = String::from_utf8(self.writer.clone()).unwrap();
        self.writer.clear();
        let body = text.split("\r\n\r\n").nth(1).unwrap();
        serde_json::from_str(body).unwrap()
    }
}

#[test]
fn test_document_link_resolve_preserves_target_and_range() {
    let mut dsp = TestDispatcher::new();

    let link_params = json!({
        "range": {
            "start": { "line": 2, "character": 5 },
            "end": { "line": 2, "character": 30 }
        },
        "target": "https://resolved.org/spec"
    });

    let req = json!({
        "jsonrpc": "2.0", "id": 701, "method": "documentLink/resolve",
        "params": link_params
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 701);
    assert_eq!(resp["result"]["target"], "https://resolved.org/spec");
    assert_eq!(resp["result"]["range"]["start"]["line"], 2);
    assert_eq!(resp["result"]["range"]["start"]["character"], 5);
    assert_eq!(resp["result"]["range"]["end"]["line"], 2);
    assert_eq!(resp["result"]["range"]["end"]["character"], 30);
}

#[test]
fn test_document_link_resolve_preserves_custom_data_payload() {
    let mut dsp = TestDispatcher::new();

    let link_params = json!({
        "range": {
            "start": { "line": 0, "character": 0 },
            "end": { "line": 0, "character": 25 }
        },
        "target": "https://crates.io/crates/serde",
        "data": {
            "crate": "serde",
            "version": "1.0.210",
            "features": ["derive"]
        }
    });

    let req = json!({
        "jsonrpc": "2.0", "id": 702, "method": "documentLink/resolve",
        "params": link_params
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 702);
    assert_eq!(resp["result"]["data"]["crate"], "serde");
    assert_eq!(resp["result"]["data"]["version"], "1.0.210");
    assert_eq!(resp["result"]["data"]["features"][0], "derive");
}

#[test]
fn test_document_link_resolve_preserves_tooltip() {
    let mut dsp = TestDispatcher::new();

    let link_params = json!({
        "range": {
            "start": { "line": 1, "character": 8 },
            "end": { "line": 1, "character": 35 }
        },
        "target": "https://doc.rust-lang.org/std/",
        "tooltip": "Open standard library documentation in browser"
    });

    let req = json!({
        "jsonrpc": "2.0", "id": 703, "method": "documentLink/resolve",
        "params": link_params
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 703);
    assert_eq!(resp["result"]["tooltip"], "Open standard library documentation in browser");
}

#[test]
fn test_document_link_resolve_null_params_returns_null() {
    let mut dsp = TestDispatcher::new();

    let req = json!({
        "jsonrpc": "2.0", "id": 704, "method": "documentLink/resolve",
        "params": null
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 704);
    assert_eq!(resp["result"], serde_json::Value::Null);
}

#[test]
fn test_document_link_consecutive_resolve_requests_isolation() {
    let mut dsp = TestDispatcher::new();

    let req1 = json!({
        "jsonrpc": "2.0", "id": 705, "method": "documentLink/resolve",
        "params": {
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 10 } },
            "target": "https://first-resolve.org"
        }
    });
    assert!(dsp.dispatch(&req1));
    let resp1 = dsp.parse_response();
    assert_eq!(resp1["id"], 705);
    assert_eq!(resp1["result"]["target"], "https://first-resolve.org");

    let req2 = json!({
        "jsonrpc": "2.0", "id": 706, "method": "documentLink/resolve",
        "params": {
            "range": { "start": { "line": 1, "character": 0 }, "end": { "line": 1, "character": 10 } },
            "target": "https://second-resolve.org"
        }
    });
    assert!(dsp.dispatch(&req2));
    let resp2 = dsp.parse_response();
    assert_eq!(resp2["id"], 706);
    assert_eq!(resp2["result"]["target"], "https://second-resolve.org");
}

#[test]
fn test_document_link_consecutive_queries_idempotent() {
    let id = DL_P7_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_dl_p7_idempotent_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "// Read https://example.com/docs for info\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req1 = json!({
        "jsonrpc": "2.0", "id": 707, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req1));
    let resp1 = dsp.parse_response();
    assert_eq!(resp1["id"], 707);

    let req2 = json!({
        "jsonrpc": "2.0", "id": 708, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req2));
    let resp2 = dsp.parse_response();
    assert_eq!(resp2["id"], 708);

    assert_eq!(resp1["result"], resp2["result"]);
}
