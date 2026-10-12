/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for workspace/willRenameFiles (Domain 10, Part 2).
//! Verifies pre-rename negotiation with cross-directory moves, complex URIs, custom IDs, and batch payloads.

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

static RENAME_COUNTER_P2: AtomicU64 = AtomicU64::new(100);

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
    let id = RENAME_COUNTER_P2.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_ren_p2_{}_{}_{}", name, std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
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
fn test_will_rename_files_across_directories() {
    let ts = create_temp_shard_with_symbol("p2_ren_dir", "SymDir");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let old_f = ts.dir.join("src/legacy/module.rs");
    let new_f = ts.dir.join("src/modern/module.rs");
    let req = json!({
        "jsonrpc": "2.0",
        "id": 201,
        "method": "workspace/willRenameFiles",
        "params": {
            "files": [{ "oldUri": to_uri(&old_f), "newUri": to_uri(&new_f) }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 201);
    assert_eq!(resp["result"], json!({}));
}

#[test]
fn test_will_rename_files_with_special_characters() {
    let ts = create_temp_shard_with_symbol("p2_ren_spec", "SymSpec");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let old_f = ts.dir.join("src/my-old_service.v1.rs");
    let new_f = ts.dir.join("src/my-new_service.v2.rs");
    let req = json!({
        "jsonrpc": "2.0",
        "id": 202,
        "method": "workspace/willRenameFiles",
        "params": {
            "files": [{ "oldUri": to_uri(&old_f), "newUri": to_uri(&new_f) }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 202);
    assert_eq!(resp["result"], json!({}));
}

#[test]
fn test_will_rename_files_windows_path_uris() {
    let ts = create_temp_shard_with_symbol("p2_ren_win", "SymWin");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0",
        "id": 203,
        "method": "workspace/willRenameFiles",
        "params": {
            "files": [{
                "oldUri": "file:///C:/Workspace/src/old.rs",
                "newUri": "file:///C:/Workspace/src/renamed.rs"
            }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 203);
    assert_eq!(resp["result"], json!({}));
}

#[test]
fn test_will_rename_files_negative_numeric_id() {
    let ts = create_temp_shard_with_symbol("p2_ren_neg", "SymNeg");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0",
        "id": -777,
        "method": "workspace/willRenameFiles",
        "params": {
            "files": [{ "oldUri": "file:///tmp/old.rs", "newUri": "file:///tmp/new.rs" }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], -777);
    assert_eq!(resp["result"], json!({}));
}

#[test]
fn test_will_rename_files_floating_point_id() {
    let ts = create_temp_shard_with_symbol("p2_ren_flt", "SymFlt");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0",
        "id": 88.25,
        "method": "workspace/willRenameFiles",
        "params": {
            "files": [{ "oldUri": "file:///tmp/old.rs", "newUri": "file:///tmp/new.rs" }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 88.25);
    assert_eq!(resp["result"], json!({}));
}

#[test]
fn test_will_rename_files_custom_string_id() {
    let ts = create_temp_shard_with_symbol("p2_ren_str", "SymStr");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0",
        "id": "rename-req-uuid-99",
        "method": "workspace/willRenameFiles",
        "params": {
            "files": [{ "oldUri": "file:///tmp/alpha.rs", "newUri": "file:///tmp/beta.rs" }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], "rename-req-uuid-99");
    assert_eq!(resp["result"], json!({}));
}

#[test]
fn test_will_rename_files_batch_twenty_files() {
    let ts = create_temp_shard_with_symbol("p2_ren_batch", "SymBatch");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let mut files = Vec::new();
    for i in 0..20 {
        files.push(json!({
            "oldUri": format!("file:///tmp/old_mod_{}.rs", i),
            "newUri": format!("file:///tmp/new_mod_{}.rs", i)
        }));
    }
    let req = json!({
        "jsonrpc": "2.0",
        "id": 220,
        "method": "workspace/willRenameFiles",
        "params": { "files": files }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 220);
    assert_eq!(resp["result"], json!({}));
}

#[test]
fn test_will_rename_files_query_and_fragment_in_uris() {
    let ts = create_temp_shard_with_symbol("p2_ren_uri", "SymUri");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0",
        "id": 221,
        "method": "workspace/willRenameFiles",
        "params": {
            "files": [{
                "oldUri": "file:///tmp/old.rs?version=1#L5",
                "newUri": "file:///tmp/new.rs?version=2#L10"
            }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 221);
    assert_eq!(resp["result"], json!({}));
}

#[test]
fn test_will_rename_files_noop_same_source_and_dest() {
    let ts = create_temp_shard_with_symbol("p2_ren_same", "SymSame");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let u = "file:///tmp/unchanged_path.rs";
    let req = json!({
        "jsonrpc": "2.0",
        "id": 222,
        "method": "workspace/willRenameFiles",
        "params": {
            "files": [{ "oldUri": u, "newUri": u }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 222);
    assert_eq!(resp["result"], json!({}));
}

#[test]
fn test_will_rename_files_extra_fields_in_rename_param() {
    let ts = create_temp_shard_with_symbol("p2_ren_extra", "SymExtra");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0",
        "id": 223,
        "method": "workspace/willRenameFiles",
        "params": {
            "files": [{
                "oldUri": "file:///tmp/old_extra.rs",
                "newUri": "file:///tmp/new_extra.rs",
                "overwrite": true,
                "ignoreIfExists": false
            }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 223);
    assert_eq!(resp["result"], json!({}));
}

#[test]
fn test_will_rename_files_missing_new_uri_graceful() {
    let ts = create_temp_shard_with_symbol("p2_ren_missing", "SymMiss");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let req = json!({
        "jsonrpc": "2.0",
        "id": 224,
        "method": "workspace/willRenameFiles",
        "params": {
            "files": [{ "oldUri": "file:///tmp/only_old.rs" }]
        }
    });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 224);
    assert_eq!(resp["result"], json!({}));
}
