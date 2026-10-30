/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/documentLink (Domain 7, Part 4).
//! Tests multiple lines, CRLF, Cyrillic UTF-16, emoji surrogate pairs, blank lines, and string literals.

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

static DL_P4_COUNTER: AtomicU64 = AtomicU64::new(400);

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
fn test_document_link_three_consecutive_lines() {
    let id = DL_P4_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_dl_p4_three_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "// https://alpha.example.com\n// https://beta.example.com\n// https://gamma.example.com\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 401, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 401);
    let links = resp["result"].as_array().unwrap();
    assert_eq!(links.len(), 3);

    assert_eq!(links[0]["target"], "https://alpha.example.com");
    assert_eq!(links[0]["range"]["start"]["line"], 0);
    assert_eq!(links[0]["range"]["start"]["character"], 3);
    assert_eq!(links[0]["range"]["end"]["line"], 0);
    assert_eq!(links[0]["range"]["end"]["character"], 28);

    assert_eq!(links[1]["target"], "https://beta.example.com");
    assert_eq!(links[1]["range"]["start"]["line"], 1);
    assert_eq!(links[1]["range"]["start"]["character"], 3);
    assert_eq!(links[1]["range"]["end"]["line"], 1);
    assert_eq!(links[1]["range"]["end"]["character"], 27);

    assert_eq!(links[2]["target"], "https://gamma.example.com");
    assert_eq!(links[2]["range"]["start"]["line"], 2);
    assert_eq!(links[2]["range"]["start"]["character"], 3);
    assert_eq!(links[2]["range"]["end"]["line"], 2);
    assert_eq!(links[2]["range"]["end"]["character"], 28);
}

#[test]
fn test_document_link_crlf_line_endings_preserved() {
    let id = DL_P4_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_dl_p4_crlf_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "// link: https://example.com/crlf\r\nfn f() {}\r\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 402, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 402);
    let links = resp["result"].as_array().unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0]["target"], "https://example.com/crlf");
    assert_eq!(links[0]["range"]["start"]["line"], 0);
    assert_eq!(links[0]["range"]["start"]["character"], 9);
    assert_eq!(links[0]["range"]["end"]["line"], 0);
    assert_eq!(links[0]["range"]["end"]["character"], 33);
}

#[test]
fn test_document_link_cyrillic_prefix_utf16_offsets() {
    let id = DL_P4_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_dl_p4_cyr_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "// Βιβλιογραφία: https://doc.rust-lang.org/book/ τέλος\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 403, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 403);
    let links = resp["result"].as_array().unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0]["target"], "https://doc.rust-lang.org/book/");
    assert_eq!(links[0]["range"]["start"]["line"], 0);
    assert_eq!(links[0]["range"]["start"]["character"], 17);
    assert_eq!(links[0]["range"]["end"]["line"], 0);
    assert_eq!(links[0]["range"]["end"]["character"], 48);
}

#[test]
fn test_document_link_emoji_surrogate_pair_utf16_offsets() {
    let id = DL_P4_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_dl_p4_emoji_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "// 🦀 link: https://crates.io end\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 404, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 404);
    let links = resp["result"].as_array().unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0]["target"], "https://crates.io");
    assert_eq!(links[0]["range"]["start"]["line"], 0);
    assert_eq!(links[0]["range"]["start"]["character"], 12);
    assert_eq!(links[0]["range"]["end"]["line"], 0);
    assert_eq!(links[0]["range"]["end"]["character"], 29);
}

#[test]
fn test_document_link_blank_lines_between_links() {
    let id = DL_P4_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_dl_p4_blank_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "// First: https://first.org\n\n\n// Second: https://second.org\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 405, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 405);
    let links = resp["result"].as_array().unwrap();
    assert_eq!(links.len(), 2);

    assert_eq!(links[0]["target"], "https://first.org");
    assert_eq!(links[0]["range"]["start"]["line"], 0);
    assert_eq!(links[0]["range"]["start"]["character"], 10);
    assert_eq!(links[0]["range"]["end"]["line"], 0);
    assert_eq!(links[0]["range"]["end"]["character"], 27);

    assert_eq!(links[1]["target"], "https://second.org");
    assert_eq!(links[1]["range"]["start"]["line"], 3);
    assert_eq!(links[1]["range"]["start"]["character"], 11);
    assert_eq!(links[1]["range"]["end"]["line"], 3);
    assert_eq!(links[1]["range"]["end"]["character"], 29);
}

#[test]
fn test_document_link_links_inside_string_literals() {
    let id = DL_P4_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_dl_p4_str_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "let url = \"https://api.github.com/repos\";\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 406, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 406);
    let links = resp["result"].as_array().unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0]["target"], "https://api.github.com/repos");
    assert_eq!(links[0]["range"]["start"]["line"], 0);
    assert_eq!(links[0]["range"]["start"]["character"], 11);
    assert_eq!(links[0]["range"]["end"]["line"], 0);
    assert_eq!(links[0]["range"]["end"]["character"], 39);
}
