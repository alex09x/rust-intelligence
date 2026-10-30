/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/semanticTokens/full/delta (Domain 7, Part 6).
//! Tests delta prepends, middle edits, multiple removals, cache eviction, and empty previousResultId.

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

static ST_P6_COUNTER: AtomicU64 = AtomicU64::new(1);

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
fn test_semantic_tokens_delta_single_token_prepended() {
    let id = ST_P6_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_semtok_p6_delta_prep_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("prep_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "second".to_string(),
        path: "prep_crate::second".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 2,
        start_col: 4,
        end_line: 2,
        end_col: 10,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "\nfn second() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);
    let full_req = json!({
        "jsonrpc": "2.0", "id": 118, "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&full_req));
    let full_resp = dsp.parse_response();
    let prev_id = full_resp["result"]["resultId"].as_str().unwrap();
    let did_open = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn first() {}\nfn second() {}\n"
            }
        }
    });
    assert!(dsp.dispatch(&did_open));
    let delta_req = json!({
        "jsonrpc": "2.0", "id": 119, "method": "textDocument/semanticTokens/full/delta",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "previousResultId": prev_id
        }
    });
    assert!(dsp.dispatch(&delta_req));
    let delta_resp = dsp.parse_response();
    assert_eq!(delta_resp["id"], 119);
    let edits = delta_resp["result"]["edits"].as_array().unwrap();
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0]["start"], 0);
}

#[test]
fn test_semantic_tokens_delta_token_modified_in_middle() {
    let id = ST_P6_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_semtok_p6_delta_mid_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("mid_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "first".to_string(),
        path: "mid_crate::first".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 4,
        end_line: 1,
        end_col: 9,
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "mid".to_string(),
        path: "mid_crate::mid".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 2,
        start_col: 4,
        end_line: 2,
        end_col: 7,
    });
    writer.add_definition(RawDefinition {
        def_id: 3,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "last".to_string(),
        path: "mid_crate::last".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 3,
        start_col: 4,
        end_line: 3,
        end_col: 8,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn first() {}\nfn mid() {}\nfn last() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);
    let full_req = json!({
        "jsonrpc": "2.0", "id": 120, "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&full_req));
    let full_resp = dsp.parse_response();
    let prev_id = full_resp["result"]["resultId"].as_str().unwrap();
    let did_open = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn first() {}\nfn longer_middle_name() {}\nfn last() {}\n"
            }
        }
    });
    assert!(dsp.dispatch(&did_open));
    let delta_req = json!({
        "jsonrpc": "2.0", "id": 121, "method": "textDocument/semanticTokens/full/delta",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "previousResultId": prev_id
        }
    });
    assert!(dsp.dispatch(&delta_req));
    let delta_resp = dsp.parse_response();
    assert_eq!(delta_resp["id"], 121);
    let edits = delta_resp["result"]["edits"].as_array().unwrap();
    assert_eq!(edits[0]["start"], 7);
    assert_eq!(edits[0]["deleteCount"], 1);
    assert_eq!(edits[0]["data"], json!([18]));
}

#[test]
fn test_semantic_tokens_delta_multiple_tokens_removed() {
    let id = ST_P6_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_semtok_p6_rem_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("rem_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "first".to_string(),
        path: "rem_crate::first".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 4,
        end_line: 1,
        end_col: 9,
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "second".to_string(),
        path: "rem_crate::second".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 2,
        start_col: 4,
        end_line: 2,
        end_col: 10,
    });
    writer.add_definition(RawDefinition {
        def_id: 3,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "third".to_string(),
        path: "rem_crate::third".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 3,
        start_col: 4,
        end_line: 3,
        end_col: 9,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn first() {}\nfn second() {}\nfn third() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);
    let full_req = json!({
        "jsonrpc": "2.0", "id": 122, "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&full_req));
    let full_resp = dsp.parse_response();
    let prev_id = full_resp["result"]["resultId"].as_str().unwrap();
    let did_open = json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&file), "languageId": "rust", "version": 1,
                "text": "fn first() {}\n"
            }
        }
    });
    assert!(dsp.dispatch(&did_open));
    let delta_req = json!({
        "jsonrpc": "2.0", "id": 123, "method": "textDocument/semanticTokens/full/delta",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "previousResultId": prev_id
        }
    });
    assert!(dsp.dispatch(&delta_req));
    let delta_resp = dsp.parse_response();
    assert_eq!(delta_resp["id"], 123);
    let edits = delta_resp["result"]["edits"].as_array().unwrap();
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0]["start"], 5);
    assert_eq!(edits[0]["deleteCount"], 10);
    assert_eq!(edits[0]["data"], json!([]));
}

#[test]
fn test_semantic_tokens_delta_cache_eviction_over_256() {
    let id = ST_P6_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_semtok_p6_evict_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("evict_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "foo".to_string(),
        path: "evict_crate::foo".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 4,
        end_line: 1,
        end_col: 7,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn foo() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);
    let full_req = json!({
        "jsonrpc": "2.0", "id": 124, "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": to_uri(&file) } }
    });
    assert!(dsp.dispatch(&full_req));
    let full_resp = dsp.parse_response();
    let old_id = full_resp["result"]["resultId"].as_str().unwrap();
    for i in 0..260 {
        let bump_req = json!({
            "jsonrpc": "2.0", "id": 2000 + i, "method": "textDocument/semanticTokens/full",
            "params": { "textDocument": { "uri": to_uri(&file) } }
        });
        assert!(dsp.dispatch(&bump_req));
        let _ = dsp.parse_response();
    }
    let delta_req = json!({
        "jsonrpc": "2.0", "id": 125, "method": "textDocument/semanticTokens/full/delta",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "previousResultId": old_id
        }
    });
    assert!(dsp.dispatch(&delta_req));
    let delta_resp = dsp.parse_response();
    assert_eq!(delta_resp["id"], 125);
    assert_eq!(delta_resp["result"]["data"], json!([0, 3, 3, 11, 2]));
}

#[test]
fn test_semantic_tokens_delta_empty_previous_result_id() {
    let id = ST_P6_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_semtok_p6_emptyid_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let file = dir.join("src/lib.rs");
    let lib_str = file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("emptyid_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "bar".to_string(),
        path: "emptyid_crate::bar".to_string(),
        signature: None,
        file: lib_str.clone(),
        start_line: 1,
        start_col: 4,
        end_line: 1,
        end_col: 7,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    std::fs::write(&file, "fn bar() {}\n").unwrap();
    let mut dsp = TestDispatcher::new(shard);
    let delta_req = json!({
        "jsonrpc": "2.0", "id": 126, "method": "textDocument/semanticTokens/full/delta",
        "params": {
            "textDocument": { "uri": to_uri(&file) },
            "previousResultId": ""
        }
    });
    assert!(dsp.dispatch(&delta_req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 126);
    assert_eq!(resp["result"]["data"], json!([0, 3, 3, 11, 2]));
}
