/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/colorPresentation (stub) (Domain 7, Part 2).
//! Tests stub color presentation handler, color parameters, range formats, ID echoing, and positive controls.

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

static COLOR_P2_COUNTER: AtomicU64 = AtomicU64::new(1);

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
    let id = COLOR_P2_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_doccolor_p2_{}_{}_{}", name, std::process::id(), id));
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
fn test_color_presentation_stub_rgb_color_params() {
    let ts = create_temp_shard("rgb");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "// https://example.com\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 741, "method": "textDocument/colorPresentation",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "color": { "red": 1.0, "green": 0.0, "blue": 0.0, "alpha": 1.0 },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 7 } }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 741);
    let presentations = resp["result"].as_array().unwrap();
    assert_eq!(presentations.len(), 0);

    // Positive control query on documentLink on same file
    let ctrl_req = json!({ "jsonrpc": "2.0", "id": 7410, "method": "textDocument/documentLink", "params": { "textDocument": { "uri": to_uri(&file) } } });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_links = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_links.len(), 1);
}

#[test]
fn test_color_presentation_stub_rgba_color_params() {
    let ts = create_temp_shard("rgba");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "// https://example.com\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 742, "method": "textDocument/colorPresentation",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "color": { "red": 0.2, "green": 0.4, "blue": 0.8, "alpha": 0.5 },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 10 } }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 742);
    assert_eq!(resp["result"], json!([]));

    let ctrl_req = json!({ "jsonrpc": "2.0", "id": 7420, "method": "textDocument/documentLink", "params": { "textDocument": { "uri": to_uri(&file) } } });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_links = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_links.len(), 1);
}

#[test]
fn test_color_presentation_stub_zero_range() {
    let ts = create_temp_shard("zero_range");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "// https://example.com\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 743, "method": "textDocument/colorPresentation",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "color": { "red": 0.0, "green": 1.0, "blue": 0.0, "alpha": 1.0 },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 743);
    assert_eq!(resp["result"], json!([]));

    let ctrl_req = json!({ "jsonrpc": "2.0", "id": 7430, "method": "textDocument/documentLink", "params": { "textDocument": { "uri": to_uri(&file) } } });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_links = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_links.len(), 1);
}

#[test]
fn test_color_presentation_stub_multi_line_range() {
    let ts = create_temp_shard("multi_line_range");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "// https://example.com\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 744, "method": "textDocument/colorPresentation",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "color": { "red": 0.5, "green": 0.5, "blue": 0.5, "alpha": 1.0 },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 2, "character": 15 } }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 744);
    assert_eq!(resp["result"], json!([]));

    let ctrl_req = json!({ "jsonrpc": "2.0", "id": 7440, "method": "textDocument/documentLink", "params": { "textDocument": { "uri": to_uri(&file) } } });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_links = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_links.len(), 1);
}

#[test]
fn test_color_presentation_stub_dispatcher_echoes_numeric_id() {
    let ts = create_temp_shard("num_id");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 555666, "method": "textDocument/colorPresentation",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "color": { "red": 1.0, "green": 1.0, "blue": 1.0, "alpha": 1.0 },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 5 } }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 555666);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_color_presentation_stub_dispatcher_echoes_string_id() {
    let ts = create_temp_shard("str_id");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": "color-pres-string-id", "method": "textDocument/colorPresentation",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "color": { "red": 0.0, "green": 0.0, "blue": 0.0, "alpha": 1.0 },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 5 } }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], "color-pres-string-id");
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_color_presentation_stub_overlay_document_uri() {
    let ts = create_temp_shard("overlay_doc");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "// Live: https://live-color.example.com\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 747, "method": "textDocument/colorPresentation",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "color": { "red": 1.0, "green": 0.5, "blue": 0.0, "alpha": 1.0 },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 6 } }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 747);
    assert_eq!(resp["result"], json!([]));

    let ctrl_req = json!({ "jsonrpc": "2.0", "id": 7470, "method": "textDocument/documentLink", "params": { "textDocument": { "uri": to_uri(&file) } } });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_links = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_links.len(), 1);
    assert_eq!(ctrl_links[0]["target"], "https://live-color.example.com");
}

#[test]
fn test_color_presentation_stub_missing_color_object() {
    let ts = create_temp_shard("missing_color");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "// https://example.com\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 748, "method": "textDocument/colorPresentation",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 5 } }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 748);
    assert_eq!(resp["result"], json!([]));

    let ctrl_req = json!({ "jsonrpc": "2.0", "id": 7480, "method": "textDocument/documentLink", "params": { "textDocument": { "uri": to_uri(&file) } } });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_links = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_links.len(), 1);
}

#[test]
fn test_color_presentation_stub_missing_range_object() {
    let ts = create_temp_shard("missing_range");
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "// https://example.com\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req = json!({
        "jsonrpc": "2.0", "id": 749, "method": "textDocument/colorPresentation",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "color": { "red": 0.0, "green": 0.0, "blue": 1.0, "alpha": 1.0 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 749);
    assert_eq!(resp["result"], json!([]));

    let ctrl_req = json!({ "jsonrpc": "2.0", "id": 7490, "method": "textDocument/documentLink", "params": { "textDocument": { "uri": to_uri(&file) } } });
    assert!(dsp.dispatch(&ctrl_req));
    let ctrl_resp = dsp.parse_response();
    let ctrl_links = ctrl_resp["result"].as_array().unwrap();
    assert_eq!(ctrl_links.len(), 1);
}

#[test]
fn test_color_presentation_stub_consecutive_queries_isolation() {
    let ts = create_temp_shard("consecutive");
    let file = ts.dir.join("src/lib.rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    for i in 1..=3 {
        let req = json!({
            "jsonrpc": "2.0", "id": 750 + i, "method": "textDocument/colorPresentation",
            "params": {
                "textDocument": { "uri": to_uri(&file) },
                "color": { "red": 0.1 * (i as f64), "green": 0.2, "blue": 0.3, "alpha": 1.0 },
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 2 } }
            }
        });
        assert!(dsp.dispatch(&req));
        let resp = dsp.parse_response();
        assert_eq!(resp["id"], 750 + i);
        assert_eq!(resp["result"], json!([]));
    }
}
