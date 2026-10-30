/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/semanticTokens/full (Domain 7, Part 4).
//! Tests module hierarchy, same-line tokens, line gaps, reference tokens, and overlap filtering.

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

static ST_P4_COUNTER: AtomicU64 = AtomicU64::new(1);

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
fn test_semantic_tokens_full_module_hierarchy() {
    let id = ST_P4_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_semtok_p4_mod_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("mod_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Mod,
        visibility: 1,
        is_macro_expansion: false,
        name: "outer".to_string(),
        path: "mod_crate::outer".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 5,
        end_line: 1,
        end_col: 10,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "mod outer {}\n").unwrap();

    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 107, "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 107);
    assert_eq!(resp["result"]["data"], json!([0, 4, 5, 0, 0]));
}

#[test]
fn test_semantic_tokens_full_same_line_multiple_tokens() {
    let id = ST_P4_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_semtok_p4_sameline_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("sameline_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "foo".to_string(),
        path: "sameline_crate::foo".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 4,
        end_line: 1,
        end_col: 7,
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "bar".to_string(),
        path: "sameline_crate::bar".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 15,
        end_line: 1,
        end_col: 18,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn foo() {} fn bar() {}\n").unwrap();

    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 108, "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 108);
    assert_eq!(resp["result"]["data"], json!([0, 3, 3, 11, 2, 0, 11, 3, 11, 2]));
}

#[test]
fn test_semantic_tokens_full_tokens_with_gap_lines() {
    let id = ST_P4_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_semtok_p4_gap_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("gap_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "top".to_string(),
        path: "gap_crate::top".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 4,
        end_line: 1,
        end_col: 7,
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "bottom".to_string(),
        path: "gap_crate::bottom".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 5,
        start_col: 4,
        end_line: 5,
        end_col: 10,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn top() {}\n\n\n\nfn bottom() {}\n").unwrap();

    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 109, "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 109);
    assert_eq!(resp["result"]["data"], json!([0, 3, 3, 11, 2, 4, 3, 6, 11, 2]));
}

#[test]
fn test_semantic_tokens_full_references_and_definitions_combined() {
    let id = ST_P4_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_semtok_p4_ref_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("ref_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "target".to_string(),
        path: "ref_crate::target".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 4,
        end_line: 1,
        end_col: 10,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 2,
        start_col: 5,
        end_line: 2,
        end_col: 11,
        target_def_id: Some(1),
        target_path: "ref_crate::target".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 4,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn target() {}\nfn caller() { target(); }\n").unwrap();

    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 110, "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 110);
    assert_eq!(resp["result"]["data"], json!([0, 3, 6, 11, 2, 1, 4, 6, 11, 0]));
}

#[test]
fn test_semantic_tokens_full_overlapping_tokens_filtered() {
    let id = ST_P4_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_semtok_p4_overlap_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("overlap_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "foo".to_string(),
        path: "overlap_crate::foo".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 4,
        end_line: 1,
        end_col: 10,
    });
    writer.add_reference(RawReference {
        file: lib_str.clone(),
        start_line: 1,
        start_col: 6,
        end_line: 1,
        end_col: 12,
        target_def_id: Some(1),
        target_path: "overlap_crate::foo".to_string(),
        target_file: Some(lib_str.clone()),
        target_unit: None,
        target_line: 1,
        target_col: 4,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn foobar() {}\n").unwrap();

    let mut dsp = TestDispatcher::new(shard);
    let req = json!({
        "jsonrpc": "2.0", "id": 111, "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 111);
    let data = resp["result"]["data"].as_array().unwrap();
    assert_eq!(data.len(), 5);
    assert_eq!(data[0], 0);
    assert_eq!(data[1], 3);
}
