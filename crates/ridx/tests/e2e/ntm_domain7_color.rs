/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/documentColor (stub) (Domain 7, Part 1).
//! Tests stub handler behavior, request ID echoing, overlay lifecycle, and positive control verification.

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

static COLOR_COUNTER: AtomicU64 = AtomicU64::new(1);

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
    let id = COLOR_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_doccolor_{}_{}_{}", name, std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let mut writer = IndexWriter::new(name, "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "test_fn".to_string(), path: format!("{}::test_fn", name),
        signature: None, file: "src/lib.rs".to_string(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 10,
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
        dispatch_lsp_message(
            msg.clone(), &mut self.writer, &mut self.lifecycle,
            &mut self.multi_root, &mut self.overlay_mgr, &self.cancelled, None,
        ).unwrap()
    }

    fn parse_response(&mut self) -> Value {
        let text = String::from_utf8(self.writer.clone()).unwrap();
        self.writer.clear();
        let body = text.split("\r\n\r\n").nth(1).unwrap();
        serde_json::from_str(body).unwrap()
    }
}

#[test]
fn test_document_color_stub_disk_file_returns_empty_array() {
    let ts = create_temp_shard("disk");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "const RED: &str = \"#ff0000\";\nfn f() {\n    let _ = 1;\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 731, "method": "textDocument/documentColor",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 731);
    let colors = resp["result"].as_array().unwrap();
    assert_eq!(colors.len(), 0);

    // Positive control query on foldingRange to verify active dispatcher
    let ctrl_req = json!({
        "jsonrpc": "2.0", "id": 7310, "method": "textDocument/foldingRange",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_ranges = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_ranges.len(), 1);
}

#[test]
fn test_document_color_stub_overlay_buffer_returns_empty_array() {
    let ts = create_temp_shard("overlay");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "const BLUE: u32 = 0x0000ff;\n// https://example.com\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 732, "method": "textDocument/documentColor",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 732);
    let colors = resp["result"].as_array().unwrap();
    assert_eq!(colors.len(), 0);

    // Positive control query on documentLink on same overlay
    let ctrl_req = json!({
        "jsonrpc": "2.0", "id": 7320, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_links = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_links.len(), 1);
}

#[test]
fn test_document_color_stub_empty_file_returns_empty_array() {
    let ts = create_temp_shard("empty");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 733, "method": "textDocument/documentColor",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let colors = resp["result"].as_array().unwrap();
    assert_eq!(colors.len(), 0);

    // Positive control query on file with link
    let ctrl_file = ts.dir.join("src/ctrl.rs");
    std::fs::write(&ctrl_file, "// https://example.com\n").unwrap();
    let ctrl_req = json!({
        "jsonrpc": "2.0", "id": 7330, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&ctrl_file) } }
    });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_links = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_links.len(), 1);
}

#[test]
fn test_document_color_stub_nonexistent_file_returns_empty_array() {
    let ts = create_temp_shard("missing");
    let file = ts.dir.join("src/missing.rs");
    let existing = ts.dir.join("src/lib.rs");
    std::fs::write(&existing, "// https://rust-lang.org\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 734, "method": "textDocument/documentColor",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let colors = resp["result"].as_array().unwrap();
    assert_eq!(colors.len(), 0);

    // Positive control query on existing file
    let ctrl_req = json!({
        "jsonrpc": "2.0", "id": 7340, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&existing) } }
    });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_links = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_links.len(), 1);
}

#[test]
fn test_document_color_stub_after_did_change_returns_empty_array() {
    let ts = create_temp_shard("did_change");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": { "uri": to_uri(&file), "languageId": "rust", "version": 1, "text": "let c = 1;\n" }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let change_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": to_uri(&file), "version": 2 },
            "contentChanges": [{ "text": "// https://changed.example.com\n" }]
        }
    });
    assert!(dsp.dispatch(&change_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 735, "method": "textDocument/documentColor",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let colors = resp["result"].as_array().unwrap();
    assert_eq!(colors.len(), 0);

    // Positive control query on updated overlay link
    let ctrl_req = json!({
        "jsonrpc": "2.0", "id": 7350, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_links = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_links.len(), 1);
}

#[test]
fn test_document_color_stub_after_did_close_returns_empty_array() {
    let ts = create_temp_shard("did_close");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "// https://disk.org\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": { "uri": to_uri(&file), "languageId": "rust", "version": 1, "text": "fn dummy() {}\n" }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let close_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didClose",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&close_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 736, "method": "textDocument/documentColor",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let colors = resp["result"].as_array().unwrap();
    assert_eq!(colors.len(), 0);

    // Positive control query on disk link
    let ctrl_req = json!({
        "jsonrpc": "2.0", "id": 7360, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_links = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_links.len(), 1);
}

#[test]
fn test_document_color_stub_dispatcher_echoes_numeric_id() {
    let ts = create_temp_shard("num_id");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 888777, "method": "textDocument/documentColor",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 888777);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_document_color_stub_dispatcher_echoes_string_id() {
    let ts = create_temp_shard("str_id");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": "color-req-99", "method": "textDocument/documentColor",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], "color-req-99");
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_document_color_stub_missing_text_document_param() {
    let ts = create_temp_shard("missing_param");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "// https://example.com\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 739, "method": "textDocument/documentColor",
        "params": {}
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 739);
    assert_eq!(resp["result"], json!([]));

    // Positive control query with proper param
    let ctrl_req = json!({
        "jsonrpc": "2.0", "id": 7390, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_links = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_links.len(), 1);
}

#[test]
fn test_document_color_stub_sequential_requests_isolation() {
    let ts = create_temp_shard("seq");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "// https://example.com\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req1 = json!({
        "jsonrpc": "2.0", "id": 1, "method": "textDocument/documentColor",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req1));
    let resp1 = dsp.parse_response();
    assert_eq!(resp1["id"], 1);
    assert_eq!(resp1["result"], json!([]));

    let req2 = json!({
        "jsonrpc": "2.0", "id": 2, "method": "textDocument/documentColor",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req2));
    let resp2 = dsp.parse_response();
    assert_eq!(resp2["id"], 2);
    assert_eq!(resp2["result"], json!([]));
}
