/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for workspace/didDeleteFiles and workspace/willDeleteFiles (Domain 10, Part 4).
//! Verifies full LSP dispatcher pipeline, post-deletion queries, multi-root isolation, and protocol resilience.

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

static DELETE_COUNTER_P4: AtomicU64 = AtomicU64::new(300);

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
    let id = DELETE_COUNTER_P4.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_del_p4_{}_{}_{}", name, std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let shard = dir.join("index.ridx");
    let mut writer = IndexWriter::new(name, "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: sym_name.to_string(),
        path: format!("{}::{}", name, sym_name),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 10,
        start_col: 5,
        end_line: 10,
        end_col: 15,
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
        dispatch_lsp_message(msg.clone(), &mut self.writer, &mut self.lifecycle, &mut self.multi_root, &mut self.overlay_mgr, &self.cancelled, None).unwrap()
    }

    fn parse_response(&mut self) -> serde_json::Value {
        let text = String::from_utf8(self.writer.clone()).unwrap();
        self.writer.clear();
        let body = text.split("\r\n\r\n").nth(1).unwrap();
        serde_json::from_str(body).unwrap()
    }
}

#[test]
fn test_did_delete_files_dispatched_via_lsp_pipeline() {
    let ts = create_temp_shard_with_symbol("p4_pipe", "SymPipe");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let target = ts.dir.join("src/pipeline_del.rs");
    let notify = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didDeleteFiles",
        "params": {
            "files": [{ "uri": to_uri(&target) }]
        }
    });
    assert!(dsp.dispatch(&notify));
    assert_eq!(dsp.writer.len(), 0);
}

#[test]
fn test_did_delete_files_followed_by_workspace_symbol_query() {
    let ts = create_temp_shard_with_symbol("p4_follow_del", "FollowDeleteSym");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let target = ts.dir.join("src/doomed.rs");
    let notify = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didDeleteFiles",
        "params": {
            "files": [{ "uri": to_uri(&target) }]
        }
    });
    assert!(dsp.dispatch(&notify));

    let sym_req = json!({
        "jsonrpc": "2.0",
        "id": 402,
        "method": "workspace/symbol",
        "params": { "query": "FollowDeleteSym" }
    });
    assert!(dsp.dispatch(&sym_req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 402);
    let result = &resp["result"];
    assert_eq!(result[0]["name"], "FollowDeleteSym");
}

#[test]
fn test_did_delete_files_multi_root_dispatch_preserves_roots() {
    let ts1 = create_temp_shard_with_symbol("p4_multi_del_a", "SymMultiA");
    let ts2 = create_temp_shard_with_symbol("p4_multi_del_b", "SymMultiB");
    let mut dsp = TestDispatcher::new(ts1.shard.clone());
    assert!(dsp.multi_root.add_root(ts2.shard.clone()));
    assert_eq!(dsp.multi_root.len(), 2);

    let notify = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didDeleteFiles",
        "params": {
            "files": [{ "uri": to_uri(&ts1.dir.join("src/obsolete_a.rs")) }]
        }
    });
    assert!(dsp.dispatch(&notify));
    assert_eq!(dsp.multi_root.len(), 2);
}

#[test]
fn test_did_delete_files_followed_by_did_open_new_document() {
    let ts = create_temp_shard_with_symbol("p4_open_new", "SymOpenNew");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let doomed_p = ts.dir.join("src/doomed_file.rs");
    let fresh_p = ts.dir.join("src/fresh_file.rs");

    let notify = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didDeleteFiles",
        "params": {
            "files": [{ "uri": to_uri(&doomed_p) }]
        }
    });
    assert!(dsp.dispatch(&notify));

    let open_msg = json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&fresh_p),
                "languageId": "rust",
                "version": 1,
                "text": "pub fn fresh_func() {}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let doc = dsp.overlay_mgr.get(fresh_p.to_str().unwrap()).unwrap();
    assert_eq!(doc.version, 1);
    assert_eq!(doc.text, "pub fn fresh_func() {}\n");
}

#[test]
fn test_did_delete_files_reader_count_invariant() {
    let ts = create_temp_shard_with_symbol("p4_del_readers", "SymReaders");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let count_before = dsp.multi_root.first_workspace().unwrap().readers().len();
    assert_eq!(count_before, 1);

    let notify = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didDeleteFiles",
        "params": {
            "files": [{ "uri": to_uri(&ts.dir.join("src/deleted_mod.rs")) }]
        }
    });
    assert!(dsp.dispatch(&notify));

    let count_after = dsp.multi_root.first_workspace().unwrap().readers().len();
    assert_eq!(count_after, 1);
}

#[test]
fn test_did_delete_files_alternating_will_and_did_delete() {
    let ts = create_temp_shard_with_symbol("p4_alt_del", "SymAltDel");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let will1 = json!({
        "jsonrpc": "2.0",
        "id": 501,
        "method": "workspace/willDeleteFiles",
        "params": {
            "files": [{ "uri": "file:///tmp/seq1_del.rs" }]
        }
    });
    assert!(dsp.dispatch(&will1));
    let resp1 = dsp.parse_response();
    assert_eq!(resp1["id"], 501);
    assert_eq!(resp1["result"], json!({}));

    let did1 = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didDeleteFiles",
        "params": {
            "files": [{ "uri": "file:///tmp/seq1_del.rs" }]
        }
    });
    assert!(dsp.dispatch(&did1));

    let will2 = json!({
        "jsonrpc": "2.0",
        "id": 502,
        "method": "workspace/willDeleteFiles",
        "params": {
            "files": [{ "uri": "file:///tmp/seq2_del.rs" }]
        }
    });
    assert!(dsp.dispatch(&will2));
    let resp2 = dsp.parse_response();
    assert_eq!(resp2["id"], 502);
    assert_eq!(resp2["result"], json!({}));
}

#[test]
fn test_did_delete_files_relative_uri_handling() {
    let ts = create_temp_shard_with_symbol("p4_del_rel", "SymDelRel");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let notify = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didDeleteFiles",
        "params": {
            "files": [{ "uri": "file:///tmp/a/../b/obsolete.rs" }]
        }
    });
    assert!(dsp.dispatch(&notify));
    assert_eq!(dsp.multi_root.len(), 1);
}

#[test]
fn test_did_delete_files_custom_scheme_ignored() {
    let ts = create_temp_shard_with_symbol("p4_del_scheme", "SymDelScheme");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let notify = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didDeleteFiles",
        "params": {
            "files": [{ "uri": "memfs://virtual/deleted.rs" }]
        }
    });
    assert!(dsp.dispatch(&notify));
    assert_eq!(dsp.multi_root.len(), 1);
}

#[test]
fn test_did_delete_files_batch_across_multiple_workspaces() {
    let ts1 = create_temp_shard_with_symbol("p4_del_ws1", "SymDelWs1");
    let ts2 = create_temp_shard_with_symbol("p4_del_ws2", "SymDelWs2");
    let mut dsp = TestDispatcher::new(ts1.shard.clone());
    assert!(dsp.multi_root.add_root(ts2.shard.clone()));

    let notify = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didDeleteFiles",
        "params": {
            "files": [
                { "uri": to_uri(&ts1.dir.join("src/ws1_doomed.rs")) },
                { "uri": to_uri(&ts2.dir.join("src/ws2_doomed.rs")) }
            ]
        }
    });
    assert!(dsp.dispatch(&notify));
    assert_eq!(dsp.multi_root.len(), 2);
}

#[test]
fn test_did_delete_files_mixed_valid_and_invalid_entries() {
    let ts = create_temp_shard_with_symbol("p4_del_mixed", "SymDelMixed");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let valid_p = ts.dir.join("src/valid_delete.rs");

    let notify = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didDeleteFiles",
        "params": {
            "files": [
                { "uri": to_uri(&valid_p) },
                { "uri": "" },
                { "uri": "https://api.github.com/repos" },
                { "no_uri": 123 }
            ]
        }
    });
    assert!(dsp.dispatch(&notify));
    assert_eq!(dsp.writer.len(), 0);
}
