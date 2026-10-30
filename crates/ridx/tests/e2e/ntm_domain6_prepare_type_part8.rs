/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/prepareTypeHierarchy (Domain 6, Part 8).
//! Tests live overlay buffers for enum, trait, type alias, and dirty vs clean overlay suppression.

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

static PREP_TYPE_P8_COUNTER: AtomicU64 = AtomicU64::new(800);

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
fn test_prepare_type_hierarchy_dirty_overlay_suppresses_workspace_index() {
    let id = PREP_TYPE_P8_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_preptype_p8_dirty_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("dirty_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 1,
        is_macro_expansion: false,
        name: "OldStruct".to_string(),
        path: "dirty_crate::OldStruct".to_string(),
        signature: None,
        file: lib_str,
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 16,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "struct OldStruct;\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    // Open overlay buffer then change it to code without any types
    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": { "uri": to_uri(&file), "languageId": "rust", "version": 1, "text": "struct OldStruct;\n" }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let change_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": to_uri(&file), "version": 2 },
            "contentChanges": [{ "text": "fn no_types_here() {}\n" }]
        }
    });
    assert!(dsp.dispatch(&change_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 781, "method": "textDocument/prepareTypeHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 4 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 781);
    assert_eq!(resp["result"], json!([]));
}

#[test]
fn test_prepare_type_hierarchy_clean_overlay_retains_on_disk_struct() {
    let id = PREP_TYPE_P8_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_preptype_p8_clean_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("clean_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 1,
        is_macro_expansion: false,
        name: "DiskStruct".to_string(),
        path: "clean_crate::DiskStruct".to_string(),
        signature: Some("pub struct DiskStruct".to_string()),
        file: lib_str,
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 22,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "pub struct DiskStruct;\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    // Open overlay with same text (clean)
    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": { "uri": to_uri(&file), "languageId": "rust", "version": 1, "text": "pub struct DiskStruct;\n" }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 782, "method": "textDocument/prepareTypeHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 12 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 782);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "DiskStruct");
}

#[test]
fn test_prepare_type_hierarchy_overlay_enum_in_live_buffer() {
    let id = PREP_TYPE_P8_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_preptype_p8_live_enum_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");

    let mut writer = IndexWriter::new("live_enum_crate", "0.1.0");
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "// empty disk\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "enum LiveStatus { Active, Inactive }\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 783, "method": "textDocument/prepareTypeHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 7 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 783);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "LiveStatus");
    assert_eq!(items[0]["kind"], 10);
}

#[test]
fn test_prepare_type_hierarchy_overlay_trait_in_live_buffer() {
    let id = PREP_TYPE_P8_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_preptype_p8_live_trait_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");

    let mut writer = IndexWriter::new("live_trait_crate", "0.1.0");
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "// empty disk\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "trait LiveService { fn handle(&self); }\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 784, "method": "textDocument/prepareTypeHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 8 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 784);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "LiveService");
    assert_eq!(items[0]["kind"], 11);
}

#[test]
fn test_prepare_type_hierarchy_overlay_type_alias_in_live_buffer() {
    let id = PREP_TYPE_P8_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_preptype_p8_live_alias_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");

    let mut writer = IndexWriter::new("live_alias_crate", "0.1.0");
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "// empty disk\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);

    let open_msg = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "type LiveAlias = u64;\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let req = json!({
        "jsonrpc": "2.0", "id": 785, "method": "textDocument/prepareTypeHierarchy",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "position": { "line": 0, "character": 7 }
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 785);
    let items = resp["result"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "LiveAlias");
    assert_eq!(items[0]["kind"], 26);
}
