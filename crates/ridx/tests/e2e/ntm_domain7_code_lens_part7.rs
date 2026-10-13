/*
 * Rust Intelligence (ridx)
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/codeLens and codeLens/resolve (Domain 7, Part 7).
//! Tests shutdown rejection (-32600), JSON-RPC protocol version invariants, and plural references title formatting.

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::path_to_uri;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter, RawDefinition, RawReference};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

static LENS_P7_COUNTER: AtomicU64 = AtomicU64::new(1);

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
    fn new(shard: Option<PathBuf>) -> Self {
        let mut d = Self {
            lifecycle: LifecycleManager::new(),
            multi_root: MultiRootWorkspace::new(),
            overlay_mgr: OverlayManager::new(),
            cancelled: Arc::new(Mutex::new(HashSet::new())),
            writer: Vec::new(),
        };
        if let Some(s) = shard {
            assert!(d.multi_root.add_root(s));
        }
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
fn test_code_lens_after_shutdown_returns_code_32600() {
    let mut dsp = TestDispatcher::new(None);
    let file = std::env::temp_dir().join(format!("ridx_lens_p7_shut_{}.rs", LENS_P7_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let shut = json!({ "jsonrpc": "2.0", "id": 701, "method": "shutdown" });
    assert!(dsp.dispatch(&shut));
    let resp_shut = dsp.parse_response();
    assert_eq!(resp_shut["id"], 701);
    assert_eq!(resp_shut["result"], Value::Null);

    let req = json!({
        "jsonrpc": "2.0", "id": 702, "method": "textDocument/codeLens",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 702);
    assert_eq!(resp["error"]["code"], -32600);
    assert_eq!(resp["error"]["message"], "Server is shutting down");
}

#[test]
fn test_code_lens_resolve_after_shutdown_returns_code_32600() {
    let mut dsp = TestDispatcher::new(None);
    let file = std::env::temp_dir().join(format!("ridx_lens_p7_res_shut_{}.rs", LENS_P7_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let shut = json!({ "jsonrpc": "2.0", "id": 703, "method": "shutdown" });
    assert!(dsp.dispatch(&shut));
    let resp_shut = dsp.parse_response();
    assert_eq!(resp_shut["id"], 703);
    assert_eq!(resp_shut["result"], Value::Null);

    let req = json!({
        "jsonrpc": "2.0", "id": 704, "method": "codeLens/resolve",
        "params": {
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 5 } },
            "data": { "uri": to_uri(&file), "line": 1, "character": 1, "name": "foo" }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 704);
    assert_eq!(resp["error"]["code"], -32600);
    assert_eq!(resp["error"]["message"], "Server is shutting down");
}

#[test]
fn test_code_lens_jsonrpc_protocol_version_is_2_0() {
    let mut dsp = TestDispatcher::new(None);
    let file = std::env::temp_dir().join(format!("ridx_lens_p7_v20_{}.rs", LENS_P7_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let req = json!({
        "jsonrpc": "2.0", "id": "code-lens-proto-v2", "method": "textDocument/codeLens",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], "code-lens-proto-v2");
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_code_lens_resolve_jsonrpc_protocol_version_is_2_0() {
    let mut dsp = TestDispatcher::new(None);
    let file = std::env::temp_dir().join(format!("ridx_lens_p7_res_v20_{}.rs", LENS_P7_COUNTER.fetch_add(1, Ordering::Relaxed)));

    let req = json!({
        "jsonrpc": "2.0", "id": "code-lens-res-proto-v2", "method": "codeLens/resolve",
        "params": {
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 5 } },
            "data": { "uri": to_uri(&file), "line": 1, "character": 1, "name": "proto_fn" }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], "code-lens-res-proto-v2");
    assert_eq!(resp["result"]["command"]["command"], "ridx.showReferences");
}

#[test]
fn test_code_lens_resolve_three_references_plural_formatting() {
    let id = LENS_P7_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_lens_p7_3ref_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let mut writer = IndexWriter::new("tri_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "shared_target".to_string(), path: "tri_crate::shared_target".to_string(),
        signature: None, file: "src/lib.rs".to_string(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 14,
    });
    for line in 3..=5 {
        writer.add_reference(RawReference {
            file: "src/lib.rs".to_string(),
            start_line: line, start_col: 5, end_line: line, end_col: 18,
            target_def_id: Some(1), target_path: "tri_crate::shared_target".to_string(),
            target_file: Some("src/lib.rs".to_string()),
            target_unit: None, target_line: 1, target_col: 1,
            kind: DefKind::Fn, is_write: false, is_macro_expansion: false,
        });
    }
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    let file = dir.join("src/lib.rs");
    std::fs::write(&file, "fn shared_target() {}\nfn c1() {\n    shared_target();\n    shared_target();\n    shared_target();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(Some(shard));

    let req = json!({
        "jsonrpc": "2.0", "id": 707, "method": "codeLens/resolve",
        "params": {
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 13 } },
            "data": { "uri": to_uri(&file), "line": 1, "character": 1, "name": "shared_target" }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 707);
    assert_eq!(resp["result"]["command"]["title"], "3 references");
    let locs = resp["result"]["command"]["arguments"][2].as_array().unwrap();
    assert_eq!(locs.len(), 3);
    let _ = std::fs::remove_dir_all(&dir);
}
