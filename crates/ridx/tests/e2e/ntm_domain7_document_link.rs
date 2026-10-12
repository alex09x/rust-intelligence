/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/documentLink (Domain 7, Part 1).
//! Tests link detection, URL delimiters, UTF-16 character offsets, and overlay precedence.

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

static DOCLINK_COUNTER: AtomicU64 = AtomicU64::new(1);

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
    let id = DOCLINK_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_doclink_{}_{}_{}", name, std::process::id(), id));
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
fn test_document_link_single_https_in_doc_comment() {
    let ts = create_temp_shard("single");
    let file = ts.dir.join("src/lib.rs");
    let content = "/// See https://crates.io/crates/ridx for docs\npub fn run() {}\n";
    std::fs::write(&file, content).unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 701, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 701);
    let links = resp["result"].as_array().unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0]["target"], "https://crates.io/crates/ridx");
    assert_eq!(links[0]["range"]["start"]["line"], 0);
    assert_eq!(links[0]["range"]["start"]["character"], 8);
    assert_eq!(links[0]["range"]["end"]["line"], 0);
    assert_eq!(links[0]["range"]["end"]["character"], 37);
}

#[test]
fn test_document_link_multiple_links_same_line() {
    let ts = create_temp_shard("multi_line");
    let file = ts.dir.join("src/lib.rs");
    let content = "// Visit https://rust-lang.org or https://crates.io today\n";
    std::fs::write(&file, content).unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 702, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let links = resp["result"].as_array().unwrap();
    assert_eq!(links.len(), 2);
    assert_eq!(links[0]["target"], "https://rust-lang.org");
    assert_eq!(links[0]["range"]["start"]["character"], 9);
    assert_eq!(links[0]["range"]["end"]["character"], 30);
    assert_eq!(links[1]["target"], "https://crates.io");
    assert_eq!(links[1]["range"]["start"]["character"], 34);
    assert_eq!(links[1]["range"]["end"]["character"], 51);
}

#[test]
fn test_document_link_markdown_bracket_and_parenthesis_delimiters() {
    let ts = create_temp_shard("md_delim");
    let file = ts.dir.join("src/lib.rs");
    let content = "//! Read [Guide](https://docs.rs/ridx/guide) first.\n";
    std::fs::write(&file, content).unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 703, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let links = resp["result"].as_array().unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0]["target"], "https://docs.rs/ridx/guide");
    assert_eq!(links[0]["range"]["start"]["character"], 17);
    assert_eq!(links[0]["range"]["end"]["character"], 43);
}

#[test]
fn test_document_link_angle_bracket_delimiter() {
    let ts = create_temp_shard("angle_delim");
    let file = ts.dir.join("src/lib.rs");
    let content = "// RFC: <https://github.com/rust-lang/rfcs/1234>\n";
    std::fs::write(&file, content).unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 704, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let links = resp["result"].as_array().unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0]["target"], "https://github.com/rust-lang/rfcs/1234");
    assert_eq!(links[0]["range"]["start"]["character"], 9);
    assert_eq!(links[0]["range"]["end"]["character"], 47);
}

#[test]
fn test_document_link_quote_delimiter() {
    let ts = create_temp_shard("quote_delim");
    let file = ts.dir.join("src/lib.rs");
    let content = "const URL: &str = \"https://api.github.com/v3\";\n";
    std::fs::write(&file, content).unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 705, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let links = resp["result"].as_array().unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0]["target"], "https://api.github.com/v3");
    assert_eq!(links[0]["range"]["start"]["character"], 19);
    assert_eq!(links[0]["range"]["end"]["character"], 44);
}

#[test]
fn test_document_link_square_bracket_delimiter() {
    let ts = create_temp_shard("sq_bracket");
    let file = ts.dir.join("src/lib.rs");
    let content = "// Reference [https://tools.ietf.org/html/rfc1] here\n";
    std::fs::write(&file, content).unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 706, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let links = resp["result"].as_array().unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0]["target"], "https://tools.ietf.org/html/rfc1");
    assert_eq!(links[0]["range"]["start"]["character"], 14);
    assert_eq!(links[0]["range"]["end"]["character"], 46);
}

#[test]
fn test_document_link_utf16_offsets_with_multibyte_prefix() {
    let ts = create_temp_shard("utf16_multibyte");
    let file = ts.dir.join("src/lib.rs");
    // "🚀 " is 1 emoji (surrogate pair = 2 UTF-16 code units) + 1 space = 3 UTF-16 units
    let content = "// 🚀 https://rocket.rs\n";
    std::fs::write(&file, content).unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 707, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let links = resp["result"].as_array().unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0]["target"], "https://rocket.rs");
    // "// " is 3 UTF-16 units + 2 for 🚀 + 1 for ' ' = 6 UTF-16 units
    assert_eq!(links[0]["range"]["start"]["character"], 6);
    assert_eq!(links[0]["range"]["end"]["character"], 23);
}

#[test]
fn test_document_link_overlay_precedence_over_disk() {
    let ts = create_temp_shard("overlay_prec");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "// Disk: https://disk.example.com\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "// Overlay: https://overlay.example.com\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 708, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let links = resp["result"].as_array().unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0]["target"], "https://overlay.example.com");
}

#[test]
fn test_document_link_overlay_did_change_updates_links() {
    let ts = create_temp_shard("overlay_change");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "// Initial: https://v1.example.com\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let change_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": to_uri(&file), "version": 2 },
            "contentChanges": [{ "text": "// Updated: https://v2.example.com\n" }]
        }
    });
    assert!(dsp.dispatch(&change_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 709, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let links = resp["result"].as_array().unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0]["target"], "https://v2.example.com");
}

#[test]
fn test_document_link_overlay_did_close_falls_back_to_disk() {
    let ts = create_temp_shard("overlay_close");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "// Disk: https://disk-fallback.org\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "// Live: https://live.example.com\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let close_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didClose",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&close_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 710, "method": "textDocument/documentLink",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    let links = resp["result"].as_array().unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0]["target"], "https://disk-fallback.org");
}
