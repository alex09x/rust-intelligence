/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for workspace/didCreateFiles and workspace/willCreateFiles (Domain 10, Part 4).
//! Verifies full LSP dispatcher pipeline, post-creation query resolution, multi-root dispatch, and protocol resilience.

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::path_to_uri;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter, RawDefinition};
use serde_json::json;
use std::collections::HashSet;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

static CREATE_COUNTER_P4: AtomicU64 = AtomicU64::new(300);

struct TempShard {
    dir: PathBuf,
    shard: PathBuf,
}

impl Drop for TempShard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn create_temp_shard_with_symbol(name: &str, sym_name: &str) -> TempShard {
    let id = CREATE_COUNTER_P4.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_create_p4_{}_{}_{}", name, std::process::id(), id));
    std::fs::create_dir_all(&dir).unwrap();
    let shard = dir.join("index.ridx");
    let mut writer = IndexWriter::new(name, "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Fn, visibility: 1, is_macro_expansion: false,
        name: sym_name.to_string(), path: format!("{}::{}", name, sym_name),
        signature: None, file: "src/lib.rs".to_string(),
        start_line: 10, start_col: 5, end_line: 10, end_col: 15,
    });
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    TempShard { dir, shard }
}

fn to_uri(p: &Path) -> String {
    path_to_uri(p.to_str().unwrap())
}

struct TestDispatcher {
    lifecycle: LifecycleManager,
    multi_root: MultiRootWorkspace,
    overlay_mgr: OverlayManager,
    cancelled: Arc<Mutex<HashSet<serde_json::Value>>>,
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

    fn dispatch(&mut self, msg: &serde_json::Value) -> bool {
        dispatch_lsp_message(
            msg.clone(), &mut self.writer, &mut self.lifecycle,
            &mut self.multi_root, &mut self.overlay_mgr, &self.cancelled, None,
        ).unwrap()
    }

    fn parse_response(&mut self) -> serde_json::Value {
        let text = String::from_utf8(self.writer.clone()).unwrap();
        self.writer.clear();
        let body = text.split("\r\n\r\n").nth(1).unwrap();
        serde_json::from_str(body).unwrap()
    }
}

#[test]
fn test_did_create_files_dispatched_via_lsp_pipeline() {
    let ts = create_temp_shard_with_symbol("p4_pipe", "PipeControl");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let new_file = ts.dir.join("src/pipeline_mod.rs");
    let notify = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didCreateFiles",
        "params": { "files": [{ "uri": to_uri(&new_file) }] }
    });
    assert!(dsp.dispatch(&notify));
    assert_eq!(dsp.writer.len(), 0);
}

#[test]
fn test_did_create_files_dispatch_followed_by_workspace_symbol() {
    let ts = create_temp_shard_with_symbol("p4_follow", "FollowSym");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let new_file = ts.dir.join("src/handler.rs");
    let notify = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didCreateFiles",
        "params": { "files": [{ "uri": to_uri(&new_file) }] }
    });
    assert!(dsp.dispatch(&notify));

    let sym_req = json!({
        "jsonrpc": "2.0",
        "id": 402,
        "method": "workspace/symbol",
        "params": { "query": "FollowSym" }
    });
    assert!(dsp.dispatch(&sym_req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 402);
    let result = &resp["result"];
    assert_eq!(result[0]["name"], "FollowSym");
}

#[test]
fn test_did_create_files_dispatch_batch_in_multiroot() {
    let ts1 = create_temp_shard_with_symbol("p4_multi_a", "SymA");
    let ts2 = create_temp_shard_with_symbol("p4_multi_b", "SymB");
    let mut dsp = TestDispatcher::new(ts1.shard.clone());
    assert!(dsp.multi_root.add_root(ts2.shard.clone()));
    assert_eq!(dsp.multi_root.len(), 2);

    let notify = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didCreateFiles",
        "params": {
            "files": [
                { "uri": to_uri(&ts1.dir.join("src/sub_a.rs")) },
                { "uri": to_uri(&ts2.dir.join("src/sub_b.rs")) }
            ]
        }
    });
    assert!(dsp.dispatch(&notify));
    assert_eq!(dsp.multi_root.len(), 2);
}

#[test]
fn test_did_create_files_empty_notification_params() {
    let ts = create_temp_shard_with_symbol("p4_empty_par", "EmptyParControl");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let notify = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didCreateFiles",
        "params": {}
    });
    assert!(dsp.dispatch(&notify));
    assert_eq!(dsp.writer.len(), 0);
}

#[test]
fn test_did_create_files_null_files_array() {
    let ts = create_temp_shard_with_symbol("p4_null_files", "NullFilesControl");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let notify = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didCreateFiles",
        "params": { "files": serde_json::Value::Null }
    });
    assert!(dsp.dispatch(&notify));
    assert_eq!(dsp.writer.len(), 0);
}

#[test]
fn test_did_create_files_symbol_references_preserved() {
    let ts = create_temp_shard_with_symbol("p4_sym_pres", "PreservedFn");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let reader = &dsp.multi_root.first_workspace().unwrap().readers()[0];
    let defs_before = reader.search_symbols("PreservedFn");
    assert_eq!(defs_before.len(), 1);
    assert_eq!(reader.get_str(defs_before[0].name_offset), "PreservedFn");

    let notify = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didCreateFiles",
        "params": { "files": [{ "uri": "file:///tmp/other_mod.rs" }] }
    });
    assert!(dsp.dispatch(&notify));

    let reader_after = &dsp.multi_root.first_workspace().unwrap().readers()[0];
    let defs_after = reader_after.search_symbols("PreservedFn");
    assert_eq!(defs_after.len(), 1);
    assert_eq!(reader_after.get_str(defs_after[0].name_offset), "PreservedFn");
}

#[test]
fn test_did_create_files_mixed_valid_and_invalid_uris() {
    let ts = create_temp_shard_with_symbol("p4_mixed", "MixedControl");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let valid_path = ts.dir.join("src/valid.rs");
    let notify = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didCreateFiles",
        "params": {
            "files": [
                { "uri": to_uri(&valid_path) },
                { "uri": "" },
                { "uri": "https://crates.io/api/v1/crates" },
                { "invalid_key": true }
            ]
        }
    });
    assert!(dsp.dispatch(&notify));
    assert_eq!(dsp.writer.len(), 0);
}

#[test]
fn test_did_create_files_directory_creation_ignored() {
    let ts = create_temp_shard_with_symbol("p4_dir", "DirControl");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let dir_path = ts.dir.join("src/controllers/");
    let notify = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didCreateFiles",
        "params": { "files": [{ "uri": to_uri(&dir_path) }] }
    });
    assert!(dsp.dispatch(&notify));
    assert_eq!(dsp.multi_root.len(), 1);
}

#[test]
fn test_did_create_files_workspace_reader_count_invariant() {
    let ts = create_temp_shard_with_symbol("p4_reader_inv", "ReaderInvControl");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let reader_count_before = dsp.multi_root.first_workspace().unwrap().readers().len();
    assert_eq!(reader_count_before, 1);

    let notify = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didCreateFiles",
        "params": { "files": [{ "uri": to_uri(&ts.dir.join("src/added.rs")) }] }
    });
    assert!(dsp.dispatch(&notify));

    let reader_count_after = dsp.multi_root.first_workspace().unwrap().readers().len();
    assert_eq!(reader_count_after, 1);
}

#[test]
fn test_did_create_files_alternating_will_and_did_create() {
    let ts = create_temp_shard_with_symbol("p4_alt", "AltControl");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let will1 = json!({
        "jsonrpc": "2.0",
        "id": 501,
        "method": "workspace/willCreateFiles",
        "params": { "files": [{ "uri": "file:///tmp/seq1.rs" }] }
    });
    assert!(dsp.dispatch(&will1));
    let resp1 = dsp.parse_response();
    assert_eq!(resp1["id"], 501);
    assert_eq!(resp1["result"], json!({}));

    let did1 = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didCreateFiles",
        "params": { "files": [{ "uri": "file:///tmp/seq1.rs" }] }
    });
    assert!(dsp.dispatch(&did1));

    let will2 = json!({
        "jsonrpc": "2.0",
        "id": 502,
        "method": "workspace/willCreateFiles",
        "params": { "files": [{ "uri": "file:///tmp/seq2.rs" }] }
    });
    assert!(dsp.dispatch(&will2));
    let resp2 = dsp.parse_response();
    assert_eq!(resp2["id"], 502);
    assert_eq!(resp2["result"], json!({}));
}
