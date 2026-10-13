/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for callHierarchy/outgoingCalls (Domain 6, Part 10).
//! Tests fromRanges call-site coordinates, exact response schema shape,
//! and clean overlay retention.

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

static OUT_COUNTER_P10: AtomicU64 = AtomicU64::new(3000);

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
fn test_call_hierarchy_outgoing_calls_from_range_matches_call_site_span() {
    let id = OUT_COUNTER_P10.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_out_p10_coords_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("coords_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "target".to_string(), path: "coords_crate::target".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 7,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "caller".to_string(), path: "coords_crate::caller".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 3, start_col: 1, end_line: 5, end_col: 2,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(), start_line: 4, start_col: 5, end_line: 4, end_col: 11,
        target_def_id: Some(1), target_path: "coords_crate::target".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None, target_line: 1, target_col: 1,
        kind: DefKind::Fn, is_write: false, is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn target() {}\n\nfn caller() {\n    target();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 761, "method": "callHierarchy/outgoingCalls",
        "params": {
            "item": {
                "name": "caller", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 2, "character": 0 }, "end": { "line": 4, "character": 1 } },
                "selectionRange": { "start": { "line": 2, "character": 0 }, "end": { "line": 4, "character": 1 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 761);
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    let range = &calls[0]["fromRanges"][0];
    assert_eq!(range["start"]["line"], 3);
    assert_eq!(range["start"]["character"], 4);
    assert_eq!(range["end"]["line"], 3);
    assert_eq!(range["end"]["character"], 10);
}

#[test]
fn test_call_hierarchy_outgoing_calls_response_shape_exact_fields() {
    let id = OUT_COUNTER_P10.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_out_p10_shape_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("shape_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "target".to_string(), path: "shape_crate::target".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 7,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "caller".to_string(), path: "shape_crate::caller".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 3, start_col: 1, end_line: 5, end_col: 2,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(), start_line: 4, start_col: 5, end_line: 4, end_col: 11,
        target_def_id: Some(1), target_path: "shape_crate::target".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None, target_line: 1, target_col: 1,
        kind: DefKind::Fn, is_write: false, is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn target() {}\n\nfn caller() {\n    target();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 762, "method": "callHierarchy/outgoingCalls",
        "params": {
            "item": {
                "name": "caller", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 2, "character": 0 }, "end": { "line": 4, "character": 1 } },
                "selectionRange": { "start": { "line": 2, "character": 0 }, "end": { "line": 4, "character": 1 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 762);
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    let item = &calls[0];
    assert_eq!(item["fromRanges"], json!([{ "start": { "line": 3, "character": 4 }, "end": { "line": 3, "character": 10 } }]));
    let to = &item["to"];
    assert_eq!(to["name"], "target");
    assert_eq!(to["kind"], 12);
    assert_eq!(to["uri"], to_uri(&file));
    assert_eq!(to["range"], json!({ "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 6 } }));
    assert_eq!(to["selectionRange"], json!({ "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 6 } }));
}

#[test]
fn test_call_hierarchy_outgoing_calls_clean_overlay_retains_outgoing() {
    let id = OUT_COUNTER_P10.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_out_p10_overlay_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("clean_overlay_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "target".to_string(), path: "clean_overlay_crate::target".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 7,
    });
    writer.add_definition(RawDefinition {
        def_id: 2, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: "caller".to_string(), path: "clean_overlay_crate::caller".to_string(),
        signature: None, file: lib_str.clone(),
        start_line: 3, start_col: 1, end_line: 5, end_col: 2,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(), start_line: 4, start_col: 5, end_line: 4, end_col: 11,
        target_def_id: Some(1), target_path: "clean_overlay_crate::target".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None, target_line: 1, target_col: 1,
        kind: DefKind::Fn, is_write: false, is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn target() {}\n\nfn caller() {\n    target();\n}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file),
                "languageId": "rust", "version": 1,
                "text": "fn target() {}\n\nfn caller() {\n    target();\n}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 763, "method": "callHierarchy/outgoingCalls",
        "params": {
            "item": {
                "name": "caller", "kind": 12, "uri": to_uri(&file),
                "range": { "start": { "line": 2, "character": 0 }, "end": { "line": 4, "character": 1 } },
                "selectionRange": { "start": { "line": 2, "character": 0 }, "end": { "line": 4, "character": 1 } }
            }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 763);
    let calls = resp["result"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["to"]["name"], "target");
}
