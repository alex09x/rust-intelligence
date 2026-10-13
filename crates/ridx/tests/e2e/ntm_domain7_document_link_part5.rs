/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/documentLink (Domain 7, Part 5).
//! Tests overlay mutations (replace, append, remove), document isolation, and disk fallback re-read.

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

static DL_P5_COUNTER: AtomicU64 = AtomicU64::new(500);

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
        dispatch_lsp_message(
            msg.clone(), &mut self.writer, &mut self.lifecycle,
            &mut self.multi_root, &mut self.overlay_mgr, &self.cancelled, None,
        ).unwrap()
    }

    fn parse_response(&mut self) -> serde_json::Value {
        let text = String::from_utf8(self.writer.clone()).unwrap();
        self.writer.clear();
        let body = text.split("\r\n\r\n").nth(1).unwrap();
        serde_json::from_str(body).unwrap()
    }
}

#[test]
fn test_document_link_did_change_replaces_link() {
    let id = DL_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_dl_p5_replace_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "// old: https://old-link.com end\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req1 = json!({
        "jsonrpc": "2.0", "id": 501, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req1));
    let resp1 = dsp.parse_response();
    assert_eq!(resp1["id"], 501);
    assert_eq!(resp1["result"][0]["target"], "https://old-link.com");

    let change_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": to_uri(&file), "version": 2 },
            "contentChanges": [
                {
                    "range": { "start": { "line": 0, "character": 8 }, "end": { "line": 0, "character": 28 } },
                    "text": "https://new-link.org"
                }
            ]
        }
    });
    assert!(dsp.dispatch(&change_msg));

    let req2 = json!({
        "jsonrpc": "2.0", "id": 502, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req2));
    let resp2 = dsp.parse_response();
    assert_eq!(resp2["id"], 502);
    let links = resp2["result"].as_array().unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0]["target"], "https://new-link.org");
    assert_eq!(links[0]["range"]["start"]["line"], 0);
    assert_eq!(links[0]["range"]["start"]["character"], 8);
    assert_eq!(links[0]["range"]["end"]["line"], 0);
    assert_eq!(links[0]["range"]["end"]["character"], 28);
}

#[test]
fn test_document_link_did_change_appends_second_link() {
    let id = DL_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_dl_p5_append_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "// first: https://first.org\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let change_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": to_uri(&file), "version": 2 },
            "contentChanges": [
                {
                    "range": { "start": { "line": 1, "character": 0 }, "end": { "line": 1, "character": 0 } },
                    "text": "// second: https://second.org\n"
                }
            ]
        }
    });
    assert!(dsp.dispatch(&change_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 503, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 503);
    let links = resp["result"].as_array().unwrap();
    assert_eq!(links.len(), 2);
    assert_eq!(links[0]["target"], "https://first.org");
    assert_eq!(links[1]["target"], "https://second.org");
}

#[test]
fn test_document_link_did_change_removes_link() {
    let id = DL_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_dl_p5_remove_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "// https://delete-me.com\nfn f() {}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let change_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": to_uri(&file), "version": 2 },
            "contentChanges": [
                {
                    "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 24 } },
                    "text": "// clean"
                }
            ]
        }
    });
    assert!(dsp.dispatch(&change_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 504, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 504);
    assert_eq!(resp["result"].as_array().unwrap().len(), 0);
}

#[test]
fn test_document_link_two_open_documents_independent_links() {
    let id1 = DL_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    let id2 = DL_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file_a = std::env::temp_dir().join(format!("ridx_dl_p5_doca_{}_{}.rs", std::process::id(), id1));
    let file_b = std::env::temp_dir().join(format!("ridx_dl_p5_docb_{}_{}.rs", std::process::id(), id2));
    let mut dsp = TestDispatcher::new();

    let open_a = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": { "uri": to_uri(&file_a), "languageId": "rust", "version": 1, "text": "// https://domain-a.org\n" }
        }
    });
    let open_b = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": { "uri": to_uri(&file_b), "languageId": "rust", "version": 1, "text": "// https://domain-b.org\n" }
        }
    });
    assert!(dsp.dispatch(&open_a));
    assert!(dsp.dispatch(&open_b));

    let req_a = json!({
        "jsonrpc": "2.0", "id": 505, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file_a) } }
    });
    assert!(dsp.dispatch(&req_a));
    let resp_a = dsp.parse_response();
    assert_eq!(resp_a["id"], 505);
    let links_a = resp_a["result"].as_array().unwrap();
    assert_eq!(links_a.len(), 1);
    assert_eq!(links_a[0]["target"], "https://domain-a.org");

    let req_b = json!({
        "jsonrpc": "2.0", "id": 506, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file_b) } }
    });
    assert!(dsp.dispatch(&req_b));
    let resp_b = dsp.parse_response();
    assert_eq!(resp_b["id"], 506);
    let links_b = resp_b["result"].as_array().unwrap();
    assert_eq!(links_b.len(), 1);
    assert_eq!(links_b[0]["target"], "https://domain-b.org");
}

#[test]
fn test_document_link_disk_fallback_file_without_overlay() {
    let id = DL_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_dl_p5_diskfallback_{}_{}.rs", std::process::id(), id));
    std::fs::write(&file, "// on disk: https://disk-only.org link\n").unwrap();
    let mut dsp = TestDispatcher::new();

    let req = json!({
        "jsonrpc": "2.0", "id": 507, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 507);
    let links = resp["result"].as_array().unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0]["target"], "https://disk-only.org");
    assert_eq!(links[0]["range"]["start"]["line"], 0);
    assert_eq!(links[0]["range"]["start"]["character"], 12);
    assert_eq!(links[0]["range"]["end"]["line"], 0);
    assert_eq!(links[0]["range"]["end"]["character"], 33);
}

#[test]
fn test_document_link_disk_file_updates_re_read() {
    let id = DL_P5_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_dl_p5_diskup_{}_{}.rs", std::process::id(), id));
    std::fs::write(&file, "// v1: https://version1.com end\n").unwrap();
    let mut dsp = TestDispatcher::new();

    let req1 = json!({
        "jsonrpc": "2.0", "id": 508, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req1));
    let resp1 = dsp.parse_response();
    assert_eq!(resp1["id"], 508);
    assert_eq!(resp1["result"][0]["target"], "https://version1.com");

    // Overwrite disk file with v2 link
    std::fs::write(&file, "// v2: https://version2.com end\n").unwrap();

    let req2 = json!({
        "jsonrpc": "2.0", "id": 509, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req2));
    let resp2 = dsp.parse_response();
    assert_eq!(resp2["id"], 509);
    assert_eq!(resp2["result"][0]["target"], "https://version2.com");
}
