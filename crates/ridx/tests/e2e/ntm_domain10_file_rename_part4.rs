/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for workspace/didRenameFiles and workspace/willRenameFiles (Domain 10, Part 4).
//! Verifies full LSP dispatcher pipeline, subsequent edits on rebound URIs, multi-root isolation, and protocol invariants.

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

static RENAME_COUNTER_P4: AtomicU64 = AtomicU64::new(300);

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
    let id = RENAME_COUNTER_P4.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_ren_p4_{}_{}_{}", name, std::process::id(), id));
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
fn test_did_rename_files_dispatched_via_lsp_pipeline() {
    let ts = create_temp_shard_with_symbol("p4_pipe", "SymPipe");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let old_f = ts.dir.join("src/before_pipe.rs");
    let new_f = ts.dir.join("src/after_pipe.rs");
    let notify = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didRenameFiles",
        "params": {
            "files": [{ "oldUri": to_uri(&old_f), "newUri": to_uri(&new_f) }]
        }
    });
    assert!(dsp.dispatch(&notify));
    assert_eq!(dsp.writer.len(), 0);
}

#[test]
fn test_did_rename_files_followed_by_workspace_symbol_query() {
    let ts = create_temp_shard_with_symbol("p4_sym_query", "FollowRenameSym");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let old_f = ts.dir.join("src/old_q.rs");
    let new_f = ts.dir.join("src/new_q.rs");
    let notify = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didRenameFiles",
        "params": {
            "files": [{ "oldUri": to_uri(&old_f), "newUri": to_uri(&new_f) }]
        }
    });
    assert!(dsp.dispatch(&notify));

    let sym_req = json!({
        "jsonrpc": "2.0",
        "id": 402,
        "method": "workspace/symbol",
        "params": { "query": "FollowRenameSym" }
    });
    assert!(dsp.dispatch(&sym_req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 402);
    let result = &resp["result"];
    assert_eq!(result[0]["name"], "FollowRenameSym");
}

#[test]
fn test_did_rename_files_multi_root_dispatch_preserves_roots() {
    let ts1 = create_temp_shard_with_symbol("p4_root_a", "SymRootA");
    let ts2 = create_temp_shard_with_symbol("p4_root_b", "SymRootB");
    let mut dsp = TestDispatcher::new(ts1.shard.clone());
    assert!(dsp.multi_root.add_root(ts2.shard.clone()));
    assert_eq!(dsp.multi_root.len(), 2);

    let notify = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didRenameFiles",
        "params": {
            "files": [{
                "oldUri": to_uri(&ts1.dir.join("src/old_a.rs")),
                "newUri": to_uri(&ts1.dir.join("src/renamed_a.rs"))
            }]
        }
    });
    assert!(dsp.dispatch(&notify));
    assert_eq!(dsp.multi_root.len(), 2);
}

#[test]
fn test_did_rename_files_followed_by_did_open_on_new_path() {
    let ts = create_temp_shard_with_symbol("p4_open_new", "SymOpenNew");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let old_p = ts.dir.join("src/unopened_old.rs");
    let new_p = ts.dir.join("src/newly_opened.rs");

    let notify = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didRenameFiles",
        "params": {
            "files": [{ "oldUri": to_uri(&old_p), "newUri": to_uri(&new_p) }]
        }
    });
    assert!(dsp.dispatch(&notify));

    let open_msg = json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&new_p),
                "languageId": "rust",
                "version": 1,
                "text": "pub fn opened_on_new() {}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let doc = dsp.overlay_mgr.get(new_p.to_str().unwrap()).unwrap();
    assert_eq!(doc.version, 1);
    assert_eq!(doc.text, "pub fn opened_on_new() {}\n");
}

#[test]
fn test_did_rename_files_followed_by_did_change_on_new_path() {
    let ts = create_temp_shard_with_symbol("p4_change_new", "SymChangeNew");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let old_p = ts.dir.join("src/source_change.rs");
    let new_p = ts.dir.join("src/dest_change.rs");

    let open_msg = json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": to_uri(&old_p),
                "languageId": "rust",
                "version": 1,
                "text": "pub fn v1() {}\n"
            }
        }
    });
    assert!(dsp.dispatch(&open_msg));

    let rename_msg = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didRenameFiles",
        "params": {
            "files": [{ "oldUri": to_uri(&old_p), "newUri": to_uri(&new_p) }]
        }
    });
    assert!(dsp.dispatch(&rename_msg));

    let change_msg = json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didChange",
        "params": {
            "textDocument": {
                "uri": to_uri(&new_p),
                "version": 2
            },
            "contentChanges": [{ "text": "pub fn v2_modified() {}\n" }]
        }
    });
    assert!(dsp.dispatch(&change_msg));

    let doc = dsp.overlay_mgr.get(new_p.to_str().unwrap()).unwrap();
    assert_eq!(doc.version, 2);
    assert_eq!(doc.text, "pub fn v2_modified() {}\n");
}

#[test]
fn test_did_rename_files_reader_count_invariant() {
    let ts = create_temp_shard_with_symbol("p4_readers", "SymReaders");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let count_before = dsp.multi_root.first_workspace().unwrap().readers().len();
    assert_eq!(count_before, 1);

    let notify = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didRenameFiles",
        "params": {
            "files": [{
                "oldUri": to_uri(&ts.dir.join("src/a.rs")),
                "newUri": to_uri(&ts.dir.join("src/b.rs"))
            }]
        }
    });
    assert!(dsp.dispatch(&notify));

    let count_after = dsp.multi_root.first_workspace().unwrap().readers().len();
    assert_eq!(count_after, 1);
}

#[test]
fn test_did_rename_files_alternating_will_and_did_rename() {
    let ts = create_temp_shard_with_symbol("p4_alt_ren", "SymAltRen");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let will1 = json!({
        "jsonrpc": "2.0",
        "id": 501,
        "method": "workspace/willRenameFiles",
        "params": {
            "files": [{ "oldUri": "file:///tmp/first_old.rs", "newUri": "file:///tmp/first_new.rs" }]
        }
    });
    assert!(dsp.dispatch(&will1));
    let resp1 = dsp.parse_response();
    assert_eq!(resp1["id"], 501);
    assert_eq!(resp1["result"], json!({}));

    let did1 = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didRenameFiles",
        "params": {
            "files": [{ "oldUri": "file:///tmp/first_old.rs", "newUri": "file:///tmp/first_new.rs" }]
        }
    });
    assert!(dsp.dispatch(&did1));

    let will2 = json!({
        "jsonrpc": "2.0",
        "id": 502,
        "method": "workspace/willRenameFiles",
        "params": {
            "files": [{ "oldUri": "file:///tmp/second_old.rs", "newUri": "file:///tmp/second_new.rs" }]
        }
    });
    assert!(dsp.dispatch(&will2));
    let resp2 = dsp.parse_response();
    assert_eq!(resp2["id"], 502);
    assert_eq!(resp2["result"], json!({}));
}

#[test]
fn test_did_rename_files_relative_uri_handling() {
    let ts = create_temp_shard_with_symbol("p4_rel_uri", "SymRelUri");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let notify = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didRenameFiles",
        "params": {
            "files": [{
                "oldUri": "file:///tmp/foo/../bar/old.rs",
                "newUri": "file:///tmp/baz/../bar/new.rs"
            }]
        }
    });
    assert!(dsp.dispatch(&notify));
    assert_eq!(dsp.multi_root.len(), 1);
}

#[test]
fn test_did_rename_files_custom_scheme_ignored() {
    let ts = create_temp_shard_with_symbol("p4_custom_scheme", "SymScheme");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let notify = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didRenameFiles",
        "params": {
            "files": [{
                "oldUri": "gitfs://repo/branch/old.rs",
                "newUri": "gitfs://repo/branch/new.rs"
            }]
        }
    });
    assert!(dsp.dispatch(&notify));
    assert_eq!(dsp.multi_root.len(), 1);
}

#[test]
fn test_did_rename_files_batch_across_multiple_workspaces() {
    let ts1 = create_temp_shard_with_symbol("p4_batch_ws1", "SymWs1");
    let ts2 = create_temp_shard_with_symbol("p4_batch_ws2", "SymWs2");
    let mut dsp = TestDispatcher::new(ts1.shard.clone());
    assert!(dsp.multi_root.add_root(ts2.shard.clone()));

    let notify = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didRenameFiles",
        "params": {
            "files": [
                {
                    "oldUri": to_uri(&ts1.dir.join("src/ws1_old.rs")),
                    "newUri": to_uri(&ts1.dir.join("src/ws1_new.rs"))
                },
                {
                    "oldUri": to_uri(&ts2.dir.join("src/ws2_old.rs")),
                    "newUri": to_uri(&ts2.dir.join("src/ws2_new.rs"))
                }
            ]
        }
    });
    assert!(dsp.dispatch(&notify));
    assert_eq!(dsp.multi_root.len(), 2);
}
