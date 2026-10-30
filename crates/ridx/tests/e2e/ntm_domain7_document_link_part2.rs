/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/documentLink and documentLink/resolve (Domain 7, Part 2).
//! Tests negative cases, positive controls, ID echoing, and documentLink/resolve.

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::path_to_uri;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter, RawDefinition};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

static DOCLINK_P2_COUNTER: AtomicU64 = AtomicU64::new(1);

struct TempShard {
    dir: PathBuf,
    shard: PathBuf,
}

impl Drop for TempShard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn create_temp_shard(name: &str) -> TempShard {
    let id = DOCLINK_P2_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_doclink_p2_{}_{}_{}", name, std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let mut writer = IndexWriter::new(name, "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "test_fn".to_string(),
        path: format!("{}::test_fn", name),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 10,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    TempShard { dir, shard }
}

fn to_uri(p: &Path) -> String {
    path_to_uri(p.to_str().unwrap())
}

struct TestDispatcher {
    lifecycle: LifecycleManager,
    multi_root: MultiRootWorkspace,
    overlay_mgr: OverlayManager,
    cancelled: Arc<Mutex<HashSet<Value>>>,
    writer: Vec<u8>,
}

impl TestDispatcher {
    fn new(shard: PathBuf) -> Self {
        let mut d = Self {
            lifecycle: LifecycleManager::new(),
            multi_root: MultiRootWorkspace::new(),
            overlay_mgr: OverlayManager::new(),
            cancelled: Arc::new(Mutex::new(HashSet::new())),
            writer: Vec::new(),
        };
        assert!(d.multi_root.add_root(shard));
        let init_msg = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } });
        assert!(d.dispatch(&init_msg));
        d.writer.clear();
        d
    }

    fn dispatch(&mut self, msg: &Value) -> bool {
        dispatch_lsp_message(msg.clone(), &mut self.writer, &mut self.lifecycle, &mut self.multi_root, &mut self.overlay_mgr, &self.cancelled, None).unwrap()
    }

    fn parse_response(&mut self) -> Value {
        let text = String::from_utf8(self.writer.clone()).unwrap();
        self.writer.clear();
        let body = text.split("\r\n\r\n").nth(1).unwrap();
        serde_json::from_str(body).unwrap()
    }
}

#[test]
fn test_document_link_empty_file_returns_empty() {
    let ts = create_temp_shard("empty");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 711, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let links = resp["result"].as_array().unwrap();
    assert_eq!(links.len(), 0);

    // Positive control query on file with link
    let ctrl_file = ts.dir.join("src/ctrl.rs");
    std::fs::write(&ctrl_file, "// https://example.com\n").unwrap();
    let ctrl_req = json!({
        "jsonrpc": "2.0", "id": 7110, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&ctrl_file) } }
    });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_links = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_links.len(), 1);
    assert_eq!(ctrl_links[0]["target"], "https://example.com");
}

#[test]
fn test_document_link_file_without_links_returns_empty() {
    let ts = create_temp_shard("no_links");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "pub fn add(a: i32, b: i32) -> i32 { a + b }\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 712, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let links = resp["result"].as_array().unwrap();
    assert_eq!(links.len(), 0);

    // Positive control query by opening overlay with link
    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "// See https://doc.rust-lang.org/book/\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));
    let ctrl_req = json!({
        "jsonrpc": "2.0", "id": 7120, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_links = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_links.len(), 1);
    assert_eq!(ctrl_links[0]["target"], "https://doc.rust-lang.org/book/");
}

#[test]
fn test_document_link_nonexistent_file_returns_empty() {
    let ts = create_temp_shard("missing");
    let file = ts.dir.join("src/missing.rs");
    let existing = ts.dir.join("src/lib.rs");
    std::fs::write(&existing, "// https://rust-lang.org\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 713, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let links = resp["result"].as_array().unwrap();
    assert_eq!(links.len(), 0);

    // Positive control query on existing file
    let ctrl_req = json!({
        "jsonrpc": "2.0", "id": 7130, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&existing) } }
    });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_links = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_links.len(), 1);
}

#[test]
fn test_document_link_dispatcher_echoes_numeric_id() {
    let ts = create_temp_shard("numeric_id");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "// https://example.com\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 999123, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 999123);
    let links = resp["result"].as_array().unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0]["target"], "https://example.com");
}

#[test]
fn test_document_link_dispatcher_echoes_string_id() {
    let ts = create_temp_shard("string_id");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "// https://example.com\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": "doclink-req-42", "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], "doclink-req-42");
    let links = resp["result"].as_array().unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0]["target"], "https://example.com");
}

#[test]
fn test_document_link_resolve_echoes_valid_link_params() {
    let ts = create_temp_shard("resolve_echo");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 716, "method": "documentLink/resolve",
        "params": {
            "range": {
                "start": { "line": 0, "character": 3 },
                "end": { "line": 0, "character": 22 }
            },
            "target": "https://example.com"
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 716);
    assert_eq!(resp["result"]["target"], "https://example.com");
    assert_eq!(resp["result"]["range"]["start"]["line"], 0);
    assert_eq!(resp["result"]["range"]["start"]["character"], 3);
}

#[test]
fn test_document_link_resolve_with_custom_tooltip_and_data() {
    let ts = create_temp_shard("resolve_tooltip");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 717, "method": "documentLink/resolve",
        "params": {
            "range": {
                "start": { "line": 1, "character": 0 },
                "end": { "line": 1, "character": 20 }
            },
            "target": "https://rust-lang.org",
            "tooltip": "Rust Homepage",
            "data": { "requestId": 100 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 717);
    assert_eq!(resp["result"]["tooltip"], "Rust Homepage");
    assert_eq!(resp["result"]["data"]["requestId"], 100);
}

#[test]
fn test_document_link_resolve_missing_params_returns_null() {
    let ts = create_temp_shard("resolve_null");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "// https://valid.com\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 718, "method": "documentLink/resolve"
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 718);
    assert_eq!(resp["result"], Value::Null);

    // Positive control query by resolving valid link params
    let ctrl_req = json!({
        "jsonrpc": "2.0", "id": 7180, "method": "documentLink/resolve",
        "params": { "target": "https://valid.com" }
    });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    assert_eq!(ctrl_resp["result"]["target"], "https://valid.com");
}

#[test]
fn test_document_link_resolve_numeric_request_id_echoed() {
    let ts = create_temp_shard("resolve_num_id");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 444555, "method": "documentLink/resolve",
        "params": { "target": "https://test.rs" }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 444555);
    assert_eq!(resp["result"]["target"], "https://test.rs");
}

#[test]
fn test_document_link_resolve_string_request_id_echoed() {
    let ts = create_temp_shard("resolve_str_id");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": "resolve-req-str", "method": "documentLink/resolve",
        "params": { "target": "https://test.rs" }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], "resolve-req-str");
    assert_eq!(resp["result"]["target"], "https://test.rs");
}
