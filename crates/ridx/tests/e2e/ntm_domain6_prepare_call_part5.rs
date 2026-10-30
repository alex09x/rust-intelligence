/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/prepareCallHierarchy (Domain 6, Part 5).
//! Tests non-function symbols, protocol errors, and nested functions.

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

static CALL_COUNTER_P5: AtomicU64 = AtomicU64::new(200);

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
    let id = CALL_COUNTER_P5.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_prepcall_p5_{}_{}_{}", name, std::process::id(), id));
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
fn test_prepare_call_hierarchy_cursor_on_struct_returns_empty() {
    let lib_str = "/mock/prepcall/struct_nodef/src/lib.rs";
    let ts = create_custom_shard(
        "struct_nodef",
        vec![RawDefinition {
            def_id: 9,
            kind: DefKind::Struct,
            visibility: 1,
            is_macro_expansion: false,
            name: "ServerConfig".to_string(),
            path: "struct_nodef::ServerConfig".to_string(),
            signature: Some("pub struct ServerConfig".to_string()),
            file: lib_str.to_string(),
            start_line: 2,
            start_col: 1,
            end_line: 2,
            end_col: 13,
        }],
    );
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0", "id": 629, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": path_to_uri(lib_str) },
            "position": { "line": 1, "character": 5 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 629);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_prepare_call_hierarchy_cursor_on_enum_returns_empty() {
    let lib_str = "/mock/prepcall/enum_nodef/src/lib.rs";
    let ts = create_custom_shard(
        "enum_nodef",
        vec![RawDefinition {
            def_id: 10,
            kind: DefKind::Enum,
            visibility: 1,
            is_macro_expansion: false,
            name: "StateEnum".to_string(),
            path: "enum_nodef::StateEnum".to_string(),
            signature: Some("pub enum StateEnum".to_string()),
            file: lib_str.to_string(),
            start_line: 4,
            start_col: 1,
            end_line: 4,
            end_col: 10,
        }],
    );
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0", "id": 630, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": path_to_uri(lib_str) },
            "position": { "line": 3, "character": 4 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 630);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_prepare_call_hierarchy_cursor_on_type_alias_returns_empty() {
    let lib_str = "/mock/prepcall/alias_nodef/src/lib.rs";
    let ts = create_custom_shard(
        "alias_nodef",
        vec![RawDefinition {
            def_id: 11,
            kind: DefKind::TyAlias,
            visibility: 1,
            is_macro_expansion: false,
            name: "IdType".to_string(),
            path: "alias_nodef::IdType".to_string(),
            signature: Some("pub type IdType = u64".to_string()),
            file: lib_str.to_string(),
            start_line: 6,
            start_col: 1,
            end_line: 6,
            end_col: 7,
        }],
    );
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0", "id": 631, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": path_to_uri(lib_str) },
            "position": { "line": 5, "character": 3 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 631);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_prepare_call_hierarchy_cursor_on_const_returns_empty() {
    let lib_str = "/mock/prepcall/const_nodef/src/lib.rs";
    let ts = create_custom_shard(
        "const_nodef",
        vec![RawDefinition {
            def_id: 12,
            kind: DefKind::Const,
            visibility: 1,
            is_macro_expansion: false,
            name: "BUFFER_SIZE".to_string(),
            path: "const_nodef::BUFFER_SIZE".to_string(),
            signature: Some("pub const BUFFER_SIZE: usize = 4096".to_string()),
            file: lib_str.to_string(),
            start_line: 20,
            start_col: 1,
            end_line: 20,
            end_col: 12,
        }],
    );
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0", "id": 632, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": path_to_uri(lib_str) },
            "position": { "line": 19, "character": 5 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 632);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_prepare_call_hierarchy_cursor_on_static_returns_empty() {
    let lib_str = "/mock/prepcall/static_nodef/src/lib.rs";
    let ts = create_custom_shard(
        "static_nodef",
        vec![RawDefinition {
            def_id: 13,
            kind: DefKind::Static,
            visibility: 1,
            is_macro_expansion: false,
            name: "GLOBAL_COUNTER".to_string(),
            path: "static_nodef::GLOBAL_COUNTER".to_string(),
            signature: Some("static GLOBAL_COUNTER: AtomicU64 = AtomicU64::new(0)".to_string()),
            file: lib_str.to_string(),
            start_line: 25,
            start_col: 1,
            end_line: 25,
            end_col: 15,
        }],
    );
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0", "id": 633, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": path_to_uri(lib_str) },
            "position": { "line": 24, "character": 8 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 633);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_prepare_call_hierarchy_missing_text_document_param_rejected_32602() {
    let ts = create_custom_shard("missing_td", vec![]);
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0", "id": 634, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "position": { "line": 0, "character": 0 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 634);
    assert_eq!(resp["error"]["code"], -32602);
}

#[test]
fn test_prepare_call_hierarchy_invalid_params_type_rejected_32602() {
    let ts = create_custom_shard("invalid_params", vec![]);
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0", "id": 635, "method": "textDocument/prepareCallHierarchy",
        "params": "not an object"
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 635);
    assert_eq!(resp["error"]["code"], -32602);
}

#[test]
fn test_prepare_call_hierarchy_nested_function_in_block() {
    let lib_str = "/mock/prepcall/nested/src/lib.rs";
    let ts = create_custom_shard(
        "nested",
        vec![RawDefinition {
            def_id: 1,
            kind: DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            name: "inner_worker".to_string(),
            path: "nested::outer::inner_worker".to_string(),
            signature: Some("fn inner_worker()".to_string()),
            file: lib_str.to_string(),
            start_line: 4,
            start_col: 9,
            end_line: 4,
            end_col: 21,
        }],
    );
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0", "id": 641, "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": { "uri": path_to_uri(lib_str) },
            "position": { "line": 3, "character": 12 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 641);
    assert_eq!(
        resp["result"],
        json!([{
            "name": "inner_worker",
            "kind": 12,
            "detail": "fn inner_worker()",
            "uri": path_to_uri(lib_str),
            "range": { "start": { "line": 3, "character": 8 }, "end": { "line": 3, "character": 20 } },
            "selectionRange": { "start": { "line": 3, "character": 8 }, "end": { "line": 3, "character": 20 } }
        }])
    );
}
