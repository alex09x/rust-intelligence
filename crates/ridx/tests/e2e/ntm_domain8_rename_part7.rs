/*
 * Rust Intelligence (ridx)
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/rename (Domain 8, Part 7).
//! Tests workspace index-backed symbols: type aliases, constants, statics, and external symbol rejection.

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

static REN_P7_COUNTER: AtomicU64 = AtomicU64::new(1);

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
fn test_rename_index_type_alias_edits_declaration_and_usages() {
    let id = REN_P7_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_ren_p7_tyalias_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let lib_file = dir.join("src/lib.rs");
    let lib_str = lib_file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("pkg", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::TyAlias,
        visibility: 1,
        is_macro_expansion: false,
        name: "ResultCode".to_string(),
        path: "pkg::ResultCode".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 10,
        end_line: 1,
        end_col: 20,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 2,
        start_col: 18,
        end_line: 2,
        end_col: 28,
        target_def_id: Some(1),
        target_path: "pkg::ResultCode".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 10,
        kind: DefKind::TyAlias,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&lib_file, "pub type ResultCode = i32;\npub fn code() -> ResultCode { 0 }\n").unwrap();

    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 306, "method": "textDocument/rename",
        "params": {
            "textDocument": { "uri": to_uri(&lib_file) },
            "position": { "line": 0, "character": 12 },
            "newName": "StatusCode"
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 306);

    let lib_uri = to_uri(&lib_file);
    let edits = resp["result"]["changes"][&lib_uri].as_array().unwrap();
    assert_eq!(edits.len(), 2);
    assert_eq!(edits[0]["newText"], "StatusCode");
    assert_eq!(edits[1]["newText"], "StatusCode");
}

#[test]
fn test_rename_index_const_edits_declaration_and_usages() {
    let id = REN_P7_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_ren_p7_const_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let lib_file = dir.join("src/lib.rs");
    let lib_str = lib_file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("pkg", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Const,
        visibility: 1,
        is_macro_expansion: false,
        name: "MAX_SIZE".to_string(),
        path: "pkg::MAX_SIZE".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 11,
        end_line: 1,
        end_col: 19,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 2,
        start_col: 27,
        end_line: 2,
        end_col: 35,
        target_def_id: Some(1),
        target_path: "pkg::MAX_SIZE".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 11,
        kind: DefKind::Const,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&lib_file, "pub const MAX_SIZE: usize = 1024;\npub fn limit() -> usize { MAX_SIZE }\n").unwrap();

    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 307, "method": "textDocument/rename",
        "params": {
            "textDocument": { "uri": to_uri(&lib_file) },
            "position": { "line": 0, "character": 12 },
            "newName": "BUFFER_LIMIT"
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 307);

    let lib_uri = to_uri(&lib_file);
    let edits = resp["result"]["changes"][&lib_uri].as_array().unwrap();
    assert_eq!(edits.len(), 2);
    assert_eq!(edits[0]["newText"], "BUFFER_LIMIT");
    assert_eq!(edits[1]["newText"], "BUFFER_LIMIT");
}

#[test]
fn test_rename_index_static_edits_declaration_and_usages() {
    let id = REN_P7_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_ren_p7_static_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let lib_file = dir.join("src/lib.rs");
    let lib_str = lib_file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("pkg", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Static,
        visibility: 1,
        is_macro_expansion: false,
        name: "COUNTER".to_string(),
        path: "pkg::COUNTER".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 12,
        end_line: 1,
        end_col: 19,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 2,
        start_col: 23,
        end_line: 2,
        end_col: 30,
        target_def_id: Some(1),
        target_path: "pkg::COUNTER".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 12,
        kind: DefKind::Static,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&lib_file, "pub static COUNTER: i32 = 0;\npub fn get() -> i32 { COUNTER }\n").unwrap();

    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 308, "method": "textDocument/rename",
        "params": {
            "textDocument": { "uri": to_uri(&lib_file) },
            "position": { "line": 0, "character": 13 },
            "newName": "TOTAL_COUNT"
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 308);

    let lib_uri = to_uri(&lib_file);
    let edits = resp["result"]["changes"][&lib_uri].as_array().unwrap();
    assert_eq!(edits.len(), 2);
    assert_eq!(edits[0]["newText"], "TOTAL_COUNT");
    assert_eq!(edits[1]["newText"], "TOTAL_COUNT");
}

#[test]
fn test_rename_index_external_symbol_rejected_with_exact_error() {
    let id = REN_P7_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_ren_p7_ext_{}_{}", std::process::id(), id));
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
        "jsonrpc": "2.0", "id": 309, "method": "textDocument/rename",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 1, "character": 8 },
            "newName": "my_local_fn"
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 309);
    assert_eq!(resp["error"]["code"], -32803);
    assert_eq!(resp["error"]["message"], "Cannot rename external symbol 'external_fn'");
}
