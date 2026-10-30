/*
 * Rust Intelligence (ridx)
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/rename (Domain 8, Part 3).
//! Tests workspace index-backed symbols: functions, structs, enums,
//! traits, type aliases, constants, statics, caller triggers, and external symbol rejection.

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

static REN_P3_COUNTER: AtomicU64 = AtomicU64::new(1);

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
fn test_rename_index_fn_declaration_and_caller_edits() {
    let id = REN_P3_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_ren_p3_fn_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let lib_file = dir.join("src/lib.rs");
    let caller_file = dir.join("src/caller.rs");
    let lib_str = lib_file.to_str().unwrap().to_string();
    let caller_str = caller_file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("pkg", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "calculate".to_string(),
        path: "pkg::calculate".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 8,
        end_line: 1,
        end_col: 17,
    });
    writer.add_reference(RawReference {
        file: caller_str.clone(),
        start_line: 2,
        start_col: 10,
        end_line: 2,
        end_col: 19,
        target_def_id: Some(1),
        target_path: "pkg::calculate".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 8,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&lib_file, "pub fn calculate(x: i32) -> i32 { x * 2 }\n").unwrap();
    std::fs::write(&caller_file, "pub fn run() -> i32 {\n    pkg::calculate(42)\n}\n").unwrap();

    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 301, "method": "textDocument/rename",
        "params": {
            "textDocument": { "uri": to_uri(&lib_file) },
            "position": { "line": 0, "character": 8 },
            "newName": "compute_val"
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 301);

    let lib_uri = to_uri(&lib_file);
    let caller_uri = to_uri(&caller_file);
    let lib_edits = resp["result"]["changes"][&lib_uri].as_array().unwrap();
    let caller_edits = resp["result"]["changes"][&caller_uri].as_array().unwrap();
    assert_eq!(lib_edits.len(), 1);
    assert_eq!(caller_edits.len(), 1);
    assert_eq!(lib_edits[0]["newText"], "compute_val");
    assert_eq!(caller_edits[0]["newText"], "compute_val");
}

#[test]
fn test_rename_index_fn_triggered_from_caller_site() {
    let id = REN_P3_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_ren_p3_caller_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let lib_file = dir.join("src/lib.rs");
    let caller_file = dir.join("src/caller.rs");
    let lib_str = lib_file.to_str().unwrap().to_string();
    let caller_str = caller_file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("pkg", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "transform".to_string(),
        path: "pkg::transform".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 8,
        end_line: 1,
        end_col: 17,
    });
    writer.add_reference(RawReference {
        file: caller_str.clone(),
        start_line: 2,
        start_col: 10,
        end_line: 2,
        end_col: 19,
        target_def_id: Some(1),
        target_path: "pkg::transform".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 8,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&lib_file, "pub fn transform(s: &str) -> usize { s.len() }\n").unwrap();
    std::fs::write(&caller_file, "pub fn run() -> usize {\n    pkg::transform(\"hi\")\n}\n").unwrap();

    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 302, "method": "textDocument/rename",
        "params": {
            "textDocument": { "uri": to_uri(&caller_file) },
            "position": { "line": 1, "character": 12 },
            "newName": "measure_str"
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 302);

    let lib_uri = to_uri(&lib_file);
    let caller_uri = to_uri(&caller_file);
    let lib_edits = resp["result"]["changes"][&lib_uri].as_array().unwrap();
    let caller_edits = resp["result"]["changes"][&caller_uri].as_array().unwrap();
    assert_eq!(lib_edits.len(), 1);
    assert_eq!(caller_edits.len(), 1);
    assert_eq!(lib_edits[0]["newText"], "measure_str");
    assert_eq!(caller_edits[0]["newText"], "measure_str");
}

#[test]
fn test_rename_index_struct_edits_declaration_and_usages() {
    let id = REN_P3_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_ren_p3_struct_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let lib_file = dir.join("src/lib.rs");
    let lib_str = lib_file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("pkg", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 1,
        is_macro_expansion: false,
        name: "Config".to_string(),
        path: "pkg::Config".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 12,
        end_line: 1,
        end_col: 18,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 4,
        start_col: 16,
        end_line: 4,
        end_col: 22,
        target_def_id: Some(1),
        target_path: "pkg::Config".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 12,
        kind: DefKind::Struct,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&lib_file, "pub struct Config {\n    pub port: u16,\n}\npub fn init(c: Config) -> u16 { c.port }\n").unwrap();

    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 303, "method": "textDocument/rename",
        "params": {
            "textDocument": { "uri": to_uri(&lib_file) },
            "position": { "line": 0, "character": 12 },
            "newName": "ServerSettings"
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 303);

    let lib_uri = to_uri(&lib_file);
    let edits = resp["result"]["changes"][&lib_uri].as_array().unwrap();
    assert_eq!(edits.len(), 2);
    assert_eq!(edits[0]["newText"], "ServerSettings");
    assert_eq!(edits[1]["newText"], "ServerSettings");
}

#[test]
fn test_rename_index_enum_edits_declaration_and_usages() {
    let id = REN_P3_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_ren_p3_enum_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let lib_file = dir.join("src/lib.rs");
    let lib_str = lib_file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("pkg", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Enum,
        visibility: 1,
        is_macro_expansion: false,
        name: "Status".to_string(),
        path: "pkg::Status".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 10,
        end_line: 1,
        end_col: 16,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 5,
        start_col: 17,
        end_line: 5,
        end_col: 23,
        target_def_id: Some(1),
        target_path: "pkg::Status".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 10,
        kind: DefKind::Enum,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&lib_file, "pub enum Status {\n    Ready,\n    Done,\n}\npub fn check(s: Status) -> bool { true }\n").unwrap();

    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 304, "method": "textDocument/rename",
        "params": {
            "textDocument": { "uri": to_uri(&lib_file) },
            "position": { "line": 0, "character": 10 },
            "newName": "Phase"
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 304);

    let lib_uri = to_uri(&lib_file);
    let edits = resp["result"]["changes"][&lib_uri].as_array().unwrap();
    assert_eq!(edits.len(), 2);
    assert_eq!(edits[0]["newText"], "Phase");
    assert_eq!(edits[1]["newText"], "Phase");
}

#[test]
fn test_rename_index_trait_edits_declaration_and_usages() {
    let id = REN_P3_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_ren_p3_trait_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let lib_file = dir.join("src/lib.rs");
    let lib_str = lib_file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("pkg", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Trait,
        visibility: 1,
        is_macro_expansion: false,
        name: "Worker".to_string(),
        path: "pkg::Worker".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 11,
        end_line: 1,
        end_col: 17,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 4,
        start_col: 21,
        end_line: 4,
        end_col: 27,
        target_def_id: Some(1),
        target_path: "pkg::Worker".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 11,
        kind: DefKind::Trait,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&lib_file, "pub trait Worker {\n    fn run(&self);\n}\npub fn exec(w: &dyn Worker) {}\n").unwrap();

    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 305, "method": "textDocument/rename",
        "params": {
            "textDocument": { "uri": to_uri(&lib_file) },
            "position": { "line": 0, "character": 12 },
            "newName": "Operator"
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 305);

    let lib_uri = to_uri(&lib_file);
    let edits = resp["result"]["changes"][&lib_uri].as_array().unwrap();
    assert_eq!(edits.len(), 2);
    assert_eq!(edits[0]["newText"], "Operator");
    assert_eq!(edits[1]["newText"], "Operator");
}
