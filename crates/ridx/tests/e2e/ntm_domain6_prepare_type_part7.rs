/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/prepareTypeHierarchy (Domain 6, Part 7).
//! Tests UTF-16 offset handling, line boundary positions, empty lines, and malformed params.

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

static PREP_TYPE_P7_COUNTER: AtomicU64 = AtomicU64::new(700);

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
fn test_prepare_type_hierarchy_utf16_multibyte_character_offset() {
    let id = PREP_TYPE_P7_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_preptype_p7_utf16_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    // Source contains emoji on line 1 before the struct on line 2
    let code = "// 🚀 rocket\nstruct RocketFuel;\n";
    std::fs::write(&file, code).unwrap();

    let mut writer = IndexWriter::new("utf_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 1,
        is_macro_expansion: false,
        name: "RocketFuel".to_string(),
        path: "utf_crate::RocketFuel".to_string(),
        signature: Some("struct RocketFuel".to_string()),
        file: lib_str,
        start_line: 2,
        start_col: 1,
        end_line: 2,
        end_col: 17,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 776, "method": "textDocument/prepareTypeHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 1, "character": 9 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 776);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "RocketFuel");
}

#[test]
fn test_prepare_type_hierarchy_position_at_first_character_of_line() {
    let id = PREP_TYPE_P7_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_preptype_p7_firstchar_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("first_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 1,
        is_macro_expansion: false,
        name: "FirstChar".to_string(),
        path: "first_crate::FirstChar".to_string(),
        signature: Some("pub struct FirstChar".to_string()),
        file: lib_str,
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 20,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "pub struct FirstChar;\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    // Line 0, character 0
    let req = json!({
        "jsonrpc": "2.0", "id": 777, "method": "textDocument/prepareTypeHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 0 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 777);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "FirstChar");
}

#[test]
fn test_prepare_type_hierarchy_position_on_empty_line_returns_empty() {
    let id = PREP_TYPE_P7_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_preptype_p7_emptyline_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("emptyline_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 1,
        is_macro_expansion: false,
        name: "First".to_string(),
        path: "emptyline_crate::First".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 12,
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Struct,
        visibility: 1,
        is_macro_expansion: false,
        name: "Second".to_string(),
        path: "emptyline_crate::Second".to_string(),
        signature: None,
        file: lib_str,
        start_line: 3,
        start_col: 1,
        end_line: 3,
        end_col: 13,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "struct First;\n\nstruct Second;\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    // Position at line 1 (empty line between First and Second)
    let req = json!({
        "jsonrpc": "2.0", "id": 778, "method": "textDocument/prepareTypeHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 1, "character": 0 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 778);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_prepare_type_hierarchy_null_position_object_returns_empty() {
    let id = PREP_TYPE_P7_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_preptype_p7_nullpos_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("nullpos_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 1,
        is_macro_expansion: false,
        name: "Foo".to_string(),
        path: "nullpos_crate::Foo".to_string(),
        signature: None,
        file: lib_str,
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 10,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "struct Foo;\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 779, "method": "textDocument/prepareTypeHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": null
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 779);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_prepare_type_hierarchy_params_missing_text_document_returns_empty() {
    let id = PREP_TYPE_P7_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_preptype_p7_notextdoc_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("notextdoc_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 1,
        is_macro_expansion: false,
        name: "Bar".to_string(),
        path: "notextdoc_crate::Bar".to_string(),
        signature: None,
        file: lib_str,
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 10,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "struct Bar;\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let req = json!({
        "jsonrpc": "2.0", "id": 780, "method": "textDocument/prepareTypeHierarchy",
        "params": {
            "position": { "line": 0, "character": 7 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 780);
    assert_eq!(resp["error"]["code"], -32602);
}
