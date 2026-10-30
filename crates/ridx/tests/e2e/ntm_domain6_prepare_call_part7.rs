/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/prepareCallHierarchy (Domain 6, Part 7).
//! Tests clean overlay retention, out-of-name cursor positions (return types,
//! parameter types, comments, string literals), consecutive queries, and
//! zero-width selection range validity.

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::path_to_uri;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter, RawDefinition};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs::File;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

static CALL_COUNTER_P7: AtomicU64 = AtomicU64::new(700);

struct TempShard {
    dir: PathBuf,
    shard: PathBuf,
}

impl Drop for TempShard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn create_custom_shard(name: &str, defs: Vec<RawDefinition>) -> TempShard {
    let id = CALL_COUNTER_P7.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_prepcall_p7_{}_{}_{}", name, std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let mut writer = IndexWriter::new(name, "0.1.0");
    for def in defs {
        writer.add_definition(def);
    }
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    TempShard { dir, shard }
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
fn test_prepare_call_hierarchy_clean_overlay_retains_definition() {
    let ts = create_custom_shard("clean_overlay", vec![]);
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn stable_target() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": path_to_uri(file.to_str().unwrap()),
                "languageId": "rust", "version": 1,
                "text": "fn stable_target() {}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 648, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": path_to_uri(file.to_str().unwrap()) },
            "position": { "line": 0, "character": 5 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 648);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "stable_target");
}

#[test]
fn test_prepare_call_hierarchy_cursor_on_fn_return_type_returns_empty() {
    let ts = create_custom_shard("ret_type", vec![]);
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn compute_num() -> i32 { 10 }\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": path_to_uri(file.to_str().unwrap()),
                "languageId": "rust", "version": 1,
                "text": "fn compute_num() -> i32 { 10 }\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 649, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": path_to_uri(file.to_str().unwrap()) },
            "position": { "line": 0, "character": 21 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 649);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_prepare_call_hierarchy_cursor_on_fn_param_returns_empty() {
    let ts = create_custom_shard("param_type", vec![]);
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn process(val: u64) {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": path_to_uri(file.to_str().unwrap()),
                "languageId": "rust", "version": 1,
                "text": "fn process(val: u64) {}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 650, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": path_to_uri(file.to_str().unwrap()) },
            "position": { "line": 0, "character": 12 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 650);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_prepare_call_hierarchy_cursor_on_comment_returns_empty() {
    let ts = create_custom_shard("comment_loc", vec![]);
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "// this is an explanatory comment\nfn target() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": path_to_uri(file.to_str().unwrap()),
                "languageId": "rust", "version": 1,
                "text": "// this is an explanatory comment\nfn target() {}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 651, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": path_to_uri(file.to_str().unwrap()) },
            "position": { "line": 0, "character": 10 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 651);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_prepare_call_hierarchy_cursor_on_string_literal_returns_empty() {
    let ts = create_custom_shard("string_loc", vec![]);
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn log_msg() { let s = \"message\"; }\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": path_to_uri(file.to_str().unwrap()),
                "languageId": "rust", "version": 1,
                "text": "fn log_msg() { let s = \"message\"; }\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 652, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": path_to_uri(file.to_str().unwrap()) },
            "position": { "line": 0, "character": 26 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 652);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_prepare_call_hierarchy_consecutive_queries_different_functions() {
    let lib_str = "/mock/prepcall/consec/src/lib.rs";
    let ts = create_custom_shard(
        "consec",
        vec![
            RawDefinition {
                def_id: 1,
                kind: DefKind::Fn,
                visibility: 1,
                is_macro_expansion: false,
                name: "fn_alpha".to_string(),
                path: "consec::fn_alpha".to_string(),
                signature: Some("pub fn fn_alpha()".to_string()),
                file: lib_str.to_string(),
                start_line: 1,
                start_col: 1,
                end_line: 1,
                end_col: 9,
            },
            RawDefinition {
                def_id: 2,
                kind: DefKind::Fn,
                visibility: 1,
                is_macro_expansion: false,
                name: "fn_beta".to_string(),
                path: "consec::fn_beta".to_string(),
                signature: Some("pub fn fn_beta()".to_string()),
                file: lib_str.to_string(),
                start_line: 3,
                start_col: 1,
                end_line: 3,
                end_col: 8,
            },
        ],
    );
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let req1 = json!({
        "jsonrpc": "2.0", "id": 653, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": path_to_uri(lib_str) },
            "position": { "line": 0, "character": 3 }
        }
    });
    assert!(dsp.dispatch(&req1));
    let resp1 = dsp.parse_response();
    assert_eq!(resp1["id"], 653);
    let items1 = resp1["result"].as_array().unwrap();
    assert_eq!(items1.len(), 1);
    assert_eq!(items1[0]["name"], "fn_alpha");

    let req2 = json!({
        "jsonrpc": "2.0", "id": 654, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": path_to_uri(lib_str) },
            "position": { "line": 2, "character": 4 }
        }
    });
    assert!(dsp.dispatch(&req2));
    let resp2 = dsp.parse_response();
    assert_eq!(resp2["id"], 654);
    let items2 = resp2["result"].as_array().unwrap();
    assert_eq!(items2.len(), 1);
    assert_eq!(items2[0]["name"], "fn_beta");
}

#[test]
fn test_prepare_call_hierarchy_zero_width_selection_range_validity() {
    let lib_str = "/mock/prepcall/zerowidth/src/lib.rs";
    let ts = create_custom_shard(
        "zerowidth",
        vec![RawDefinition {
            def_id: 1,
            kind: DefKind::Fn,
            visibility: 1,
            is_macro_expansion: false,
            name: "f".to_string(),
            path: "zerowidth::f".to_string(),
            signature: Some("fn f()".to_string()),
            file: lib_str.to_string(),
            start_line: 1,
            start_col: 4,
            end_line: 1,
            end_col: 5,
        }],
    );
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0", "id": 655, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": path_to_uri(lib_str) },
            "position": { "line": 0, "character": 3 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 655);
    assert_eq!(
        resp["result"],
        json!([{
            "name": "f",
            "kind": 12,
            "detail": "fn f()",
            "uri": path_to_uri(lib_str),
            "range": { "start": { "line": 0, "character": 3 }, "end": { "line": 0, "character": 4 } },
            "selectionRange": { "start": { "line": 0, "character": 3 }, "end": { "line": 0, "character": 4 } }
        }])
    );
}
