/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/prepareCallHierarchy (Domain 6, Part 6).
//! Tests private functions, pub(crate) visibility, method receivers (&self, &mut self, self),
//! UTF-16 coordinate handling, and dirty overlay isolation.

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

static CALL_COUNTER_P6: AtomicU64 = AtomicU64::new(600);

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
    let id = CALL_COUNTER_P6.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_prepcall_p6_{}_{}_{}", name, std::process::id(), id));
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
fn test_prepare_call_hierarchy_private_function() {
    let lib_str = "/mock/prepcall/priv_fn/src/lib.rs";
    let ts = create_custom_shard("priv_fn", vec![
        RawDefinition {
            def_id: 1, kind: DefKind::Fn, visibility: 0, is_macro_expansion: false,
            name: "secret_work".to_string(), path: "priv_fn::secret_work".to_string(),
            signature: Some("fn secret_work()".to_string()),
            file: lib_str.to_string(),
            start_line: 5, start_col: 4, end_line: 5, end_col: 15,
        }
    ]);
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0", "id": 641, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": path_to_uri(lib_str) },
            "position": { "line": 4, "character": 6 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 641);
    assert_eq!(resp["result"], json!([{
        "name": "secret_work",
        "kind": 12,
        "detail": "fn secret_work()",
        "uri": path_to_uri(lib_str),
        "range": { "start": { "line": 4, "character": 3 }, "end": { "line": 4, "character": 14 } },
        "selectionRange": { "start": { "line": 4, "character": 3 }, "end": { "line": 4, "character": 14 } }
    }]));
}

#[test]
fn test_prepare_call_hierarchy_pub_crate_function() {
    let lib_str = "/mock/prepcall/crate_fn/src/lib.rs";
    let ts = create_custom_shard("crate_fn", vec![
        RawDefinition {
            def_id: 2, kind: DefKind::Fn, visibility: 2, is_macro_expansion: false,
            name: "crate_worker".to_string(), path: "crate_fn::crate_worker".to_string(),
            signature: Some("pub(crate) fn crate_worker()".to_string()),
            file: lib_str.to_string(),
            start_line: 8, start_col: 15, end_line: 8, end_col: 27,
        }
    ]);
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0", "id": 642, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": path_to_uri(lib_str) },
            "position": { "line": 7, "character": 18 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 642);
    assert_eq!(resp["result"], json!([{
        "name": "crate_worker",
        "kind": 12,
        "detail": "pub(crate) fn crate_worker()",
        "uri": path_to_uri(lib_str),
        "range": { "start": { "line": 7, "character": 14 }, "end": { "line": 7, "character": 26 } },
        "selectionRange": { "start": { "line": 7, "character": 14 }, "end": { "line": 7, "character": 26 } }
    }]));
}

#[test]
fn test_prepare_call_hierarchy_method_with_ref_self() {
    let lib_str = "/mock/prepcall/self_ref/src/lib.rs";
    let ts = create_custom_shard("self_ref", vec![
        RawDefinition {
            def_id: 4, kind: DefKind::AssocFn, visibility: 1, is_macro_expansion: false,
            name: "read_data".to_string(), path: "self_ref::Data::read_data".to_string(),
            signature: Some("pub fn read_data(&self)".to_string()),
            file: lib_str.to_string(),
            start_line: 10, start_col: 8, end_line: 10, end_col: 17,
        }
    ]);
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0", "id": 643, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": path_to_uri(lib_str) },
            "position": { "line": 9, "character": 10 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 643);
    assert_eq!(resp["result"], json!([{
        "name": "read_data",
        "kind": 6,
        "detail": "pub fn read_data(&self)",
        "uri": path_to_uri(lib_str),
        "range": { "start": { "line": 9, "character": 7 }, "end": { "line": 9, "character": 16 } },
        "selectionRange": { "start": { "line": 9, "character": 7 }, "end": { "line": 9, "character": 16 } }
    }]));
}

#[test]
fn test_prepare_call_hierarchy_method_with_mut_ref_self() {
    let lib_str = "/mock/prepcall/self_mut/src/lib.rs";
    let ts = create_custom_shard("self_mut", vec![
        RawDefinition {
            def_id: 5, kind: DefKind::AssocFn, visibility: 1, is_macro_expansion: false,
            name: "mutate_data".to_string(), path: "self_mut::Data::mutate_data".to_string(),
            signature: Some("pub fn mutate_data(&mut self)".to_string()),
            file: lib_str.to_string(),
            start_line: 12, start_col: 8, end_line: 12, end_col: 19,
        }
    ]);
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0", "id": 644, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": path_to_uri(lib_str) },
            "position": { "line": 11, "character": 12 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 644);
    assert_eq!(resp["result"], json!([{
        "name": "mutate_data",
        "kind": 6,
        "detail": "pub fn mutate_data(&mut self)",
        "uri": path_to_uri(lib_str),
        "range": { "start": { "line": 11, "character": 7 }, "end": { "line": 11, "character": 18 } },
        "selectionRange": { "start": { "line": 11, "character": 7 }, "end": { "line": 11, "character": 18 } }
    }]));
}

#[test]
fn test_prepare_call_hierarchy_method_with_value_self() {
    let lib_str = "/mock/prepcall/self_val/src/lib.rs";
    let ts = create_custom_shard("self_val", vec![
        RawDefinition {
            def_id: 6, kind: DefKind::AssocFn, visibility: 1, is_macro_expansion: false,
            name: "consume".to_string(), path: "self_val::MyStruct::consume".to_string(),
            signature: Some("pub fn consume(self)".to_string()),
            file: lib_str.to_string(),
            start_line: 14, start_col: 5, end_line: 14, end_col: 12,
        }
    ]);
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0", "id": 645, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": path_to_uri(lib_str) },
            "position": { "line": 13, "character": 8 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 645);
    assert_eq!(resp["result"], json!([{
        "name": "consume",
        "kind": 6,
        "detail": "pub fn consume(self)",
        "uri": path_to_uri(lib_str),
        "range": { "start": { "line": 13, "character": 4 }, "end": { "line": 13, "character": 11 } },
        "selectionRange": { "start": { "line": 13, "character": 4 }, "end": { "line": 13, "character": 11 } }
    }]));
}

#[test]
fn test_prepare_call_hierarchy_utf16_multibyte_prefix_columns() {
    let ts = create_custom_shard("utf16_test", vec![]);
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "// 🚀 prefix comment\nfn rocket_launch() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": path_to_uri(file.to_str().unwrap()),
                "languageId": "rust", "version": 1,
                "text": "// 🚀 prefix comment\nfn rocket_launch() {}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 646, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": path_to_uri(file.to_str().unwrap()) },
            "position": { "line": 1, "character": 5 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 646);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "rocket_launch");
    assert_eq!(items[0]["kind"], 12);
}

#[test]
fn test_prepare_call_hierarchy_dirty_overlay_skips_stale_workspace() {
    let ts = create_custom_shard("dirty_overlay", vec![]);
    let file = ts.dir.join("src/lib.rs");
    std::fs::write(&file, "fn base_calc() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": path_to_uri(file.to_str().unwrap()),
                "languageId": "rust", "version": 1,
                "text": "fn base_calc() {}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let change_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": path_to_uri(file.to_str().unwrap()), "version": 2 },
            "contentChanges": [{ "text": "// replaced with comments\n" }]
        }
    });
    assert!(dsp.dispatch(&change_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 647, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": path_to_uri(file.to_str().unwrap()) },
            "position": { "line": 0, "character": 4 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 647);
    assert_eq!(resp["result"], json!([]));
}
