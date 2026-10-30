/*
 * Rust Intelligence (ridx)
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/prepareRename (Domain 8, Part 4).
//! Tests statics, reference sites, external crate symbol rejection, blank lines, nonexistent files, and missing params.

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

static PREP_P4_COUNTER: AtomicU64 = AtomicU64::new(1);

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

    fn new_empty() -> Self {
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
fn test_prepare_rename_index_static_definition_exact_range() {
    let id = PREP_P4_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_prep_p4_stat_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("test_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Static,
        visibility: 1,
        is_macro_expansion: false,
        name: "GLOBAL_COUNTER".to_string(),
        path: "test_crate::GLOBAL_COUNTER".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 8,
        end_line: 1,
        end_col: 22,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "static GLOBAL_COUNTER: i32 = 0;\n").unwrap();

    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 401, "method": "textDocument/prepareRename",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 10 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 401);
    assert_eq!(resp["result"]["placeholder"], "GLOBAL_COUNTER");
    assert_eq!(resp["result"]["range"]["start"]["character"], 7);
    assert_eq!(resp["result"]["range"]["end"]["character"], 21);
}

#[test]
fn test_prepare_rename_index_reference_site_prepares_target() {
    let id = PREP_P4_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_prep_p4_ref_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("test_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "target_fn".to_string(),
        path: "test_crate::target_fn".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 4,
        end_line: 1,
        end_col: 13,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 3,
        start_col: 5,
        end_line: 3,
        end_col: 14,
        target_def_id: Some(1),
        target_path: "test_crate::target_fn".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 4,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn target_fn() {}\nfn caller() {\n    target_fn();\n}\n").unwrap();

    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 402, "method": "textDocument/prepareRename",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 2, "character": 7 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 402);
    assert_eq!(resp["result"]["placeholder"], "target_fn");
    assert_eq!(resp["result"]["range"]["start"]["line"], 2);
    assert_eq!(resp["result"]["range"]["start"]["character"], 4);
    assert_eq!(resp["result"]["range"]["end"]["line"], 2);
    assert_eq!(resp["result"]["range"]["end"]["character"], 13);
}

#[test]
fn test_prepare_rename_external_crate_symbol_rejected_with_exact_message() {
    let id = PREP_P4_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_prep_p4_ext_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("ext_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "external_fn".to_string(),
        path: "dependency::external_fn".to_string(),
        signature: None,
        file: "/opt/.cargo/registry/src/index.crates.io-0000000000000000/dep-1.0.0/src/lib.rs".to_string(),
        start_line: 1,
        start_col: 4,
        end_line: 1,
        end_col: 15,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 2,
        start_col: 5,
        end_line: 2,
        end_col: 16,
        target_def_id: Some(1),
        target_path: "dependency::external_fn".to_string(),
        target_file: Some("/opt/.cargo/registry/src/index.crates.io-0000000000000000/dep-1.0.0/src/lib.rs".to_string()),
        target_unit: None,
        target_line: 1,
        target_col: 4,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn main() {\n    external_fn();\n}\n").unwrap();

    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 403, "method": "textDocument/prepareRename",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 1, "character": 8 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 403);
    assert_eq!(resp["error"]["code"], -32803);
    assert_eq!(resp["error"]["message"], "Cannot rename external symbol 'external_fn'");
}

#[test]
fn test_prepare_rename_blank_line_returns_error_32803() {
    let mut dsp = TestDispatcher::new_empty();
    let file = std::env::temp_dir().join(format!("ridx_prep_p4_blank_{}.rs", PREP_P4_COUNTER.fetch_add(1, Ordering::Relaxed)));
    std::fs::write(&file, "fn main() {\n\n    let x = 1;\n}\n").unwrap();

    let req = json!({
        "jsonrpc": "2.0", "id": 404, "method": "textDocument/prepareRename",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 1, "character": 0 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 404);
    assert_eq!(resp["error"]["code"], -32803);
    assert_eq!(resp["error"]["message"], "Cannot prepare rename: no identifier found at this position");
}

#[test]
fn test_prepare_rename_nonexistent_file_uri_returns_error_32803() {
    let mut dsp = TestDispatcher::new_empty();

    let req = json!({
        "jsonrpc": "2.0", "id": 405, "method": "textDocument/prepareRename",
        "params": {
            "textDocument": { "uri": "file:///tmp/nonexistent/ghost_prep_rename.rs" },
            "position": { "line": 0, "character": 0 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 405);
    assert_eq!(resp["error"]["code"], -32803);
    assert_eq!(resp["error"]["message"], "Cannot prepare rename: no identifier found at this position");
}

#[test]
fn test_prepare_rename_missing_params_completely_returns_error() {
    let mut dsp = TestDispatcher::new_empty();

    let req = json!({
        "jsonrpc": "2.0", "id": 406, "method": "textDocument/prepareRename"
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 406);
    assert_eq!(resp["error"]["code"], -32803);
}
