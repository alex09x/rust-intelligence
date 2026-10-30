/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/documentLink (Domain 7, Part 6).
//! Tests delimiters and URL boundaries: parentheses, brackets, angle brackets, tabs, and mixed delimiters.

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

static DL_P6_COUNTER: AtomicU64 = AtomicU64::new(600);

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
fn test_document_link_parenthesis_in_markdown_link() {
    let id = DL_P6_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_dl_p6_mdparen_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "// See [tokio](https://docs.rs/tokio) runtime\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 601, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 601);
    let links = resp["result"].as_array().unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0]["target"], "https://docs.rs/tokio");
    assert_eq!(links[0]["range"]["start"]["line"], 0);
    assert_eq!(links[0]["range"]["start"]["character"], 15);
    assert_eq!(links[0]["range"]["end"]["line"], 0);
    assert_eq!(links[0]["range"]["end"]["character"], 36);
}

#[test]
fn test_document_link_square_bracket_in_reference() {
    let id = DL_P6_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_dl_p6_sqbracket_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "// Spec: [https://reference.rust-lang.org] spec\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 602, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 602);
    let links = resp["result"].as_array().unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0]["target"], "https://reference.rust-lang.org");
    assert_eq!(links[0]["range"]["start"]["line"], 0);
    assert_eq!(links[0]["range"]["start"]["character"], 10);
    assert_eq!(links[0]["range"]["end"]["line"], 0);
    assert_eq!(links[0]["range"]["end"]["character"], 41);
}

#[test]
fn test_document_link_angle_bracket_in_doc() {
    let id = DL_P6_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_dl_p6_angbracket_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "// RFC: <https://tools.ietf.org/rfc> doc\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 603, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 603);
    let links = resp["result"].as_array().unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0]["target"], "https://tools.ietf.org/rfc");
    assert_eq!(links[0]["range"]["start"]["line"], 0);
    assert_eq!(links[0]["range"]["start"]["character"], 9);
    assert_eq!(links[0]["range"]["end"]["line"], 0);
    assert_eq!(links[0]["range"]["end"]["character"], 35);
}

#[test]
fn test_document_link_whitespace_tab_delimiter() {
    let id = DL_P6_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_dl_p6_tab_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "// url:\thttps://tab.example.com\tfor details\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 604, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 604);
    let links = resp["result"].as_array().unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0]["target"], "https://tab.example.com");
    assert_eq!(links[0]["range"]["start"]["line"], 0);
    assert_eq!(links[0]["range"]["start"]["character"], 8);
    assert_eq!(links[0]["range"]["end"]["line"], 0);
    assert_eq!(links[0]["range"]["end"]["character"], 31);
}

#[test]
fn test_document_link_multiple_links_with_mixed_delimiters() {
    let id = DL_P6_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_dl_p6_mixed_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "// <https://link1.org> and (https://link2.org) and \"https://link3.org\"\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 605, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 605);
    let links = resp["result"].as_array().unwrap();
    assert_eq!(links.len(), 3);
    assert_eq!(links[0]["target"], "https://link1.org");
    assert_eq!(links[1]["target"], "https://link2.org");
    assert_eq!(links[2]["target"], "https://link3.org");
}

#[test]
fn test_document_link_url_with_trailing_slash() {
    let id = DL_P6_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("ridx_dl_p6_slash_{}_{}.rs", std::process::id(), id));
    let mut dsp = TestDispatcher::new();

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "// Visit https://rust-lang.org/ now\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 606, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 606);
    let links = resp["result"].as_array().unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0]["target"], "https://rust-lang.org/");
    assert_eq!(links[0]["range"]["start"]["line"], 0);
    assert_eq!(links[0]["range"]["start"]["character"], 9);
    assert_eq!(links[0]["range"]["end"]["line"], 0);
    assert_eq!(links[0]["range"]["end"]["character"], 31);
}
