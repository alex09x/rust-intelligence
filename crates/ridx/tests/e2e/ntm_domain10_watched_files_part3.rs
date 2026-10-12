/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for workspace/didChangeWatchedFiles part 3:
//! deleted events (type 3), non-ridx file filtering, malformed entries, and rapid sequences.

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::path_to_uri;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use ridx_format::IndexWriter;
use serde_json::json;
use std::collections::HashSet;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

static WF_P3_COUNTER: AtomicU64 = AtomicU64::new(200);

struct TempShard {
    dir: PathBuf,
    shard: PathBuf,
}

impl Drop for TempShard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn create_temp_shard(name: &str) -> TempShard {
    let id = WF_P3_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_wf_p3_{}_{}_{}", name, std::process::id(), id));
    std::fs::create_dir_all(&dir).unwrap();
    let shard = dir.join("index.ridx");
    let mut writer = IndexWriter::new(name, "0.1.0");
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
        d.multi_root.add_root(shard);
        d
    }

    fn dispatch(&mut self, msg: &serde_json::Value) -> bool {
        dispatch_lsp_message(
            msg.clone(),
            &mut self.writer,
            &mut self.lifecycle,
            &mut self.multi_root,
            &mut self.overlay_mgr,
            &self.cancelled,
            None,
        )
        .unwrap()
    }
}

#[test]
fn test_did_change_watched_files_deleted_event_type_3() {
    let ts = create_temp_shard("del3");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let notif = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWatchedFiles",
        "params": { "changes": [{ "uri": to_uri(&ts.shard), "type": 3 }] }
    });
    let res = dsp.dispatch(&notif);
    assert!(res);
}

#[test]
fn test_did_change_watched_files_source_rs_file_in_changes_ignored() {
    let ts = create_temp_shard("src_rs");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let notif = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWatchedFiles",
        "params": { "changes": [{ "uri": "file:///workspace/src/lib.rs", "type": 2 }] }
    });
    let res = dsp.dispatch(&notif);
    assert!(res);
}

#[test]
fn test_did_change_watched_files_toml_file_in_changes_ignored() {
    let ts = create_temp_shard("toml");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let notif = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWatchedFiles",
        "params": { "changes": [{ "uri": "file:///workspace/Cargo.toml", "type": 2 }] }
    });
    let res = dsp.dispatch(&notif);
    assert!(res);
}

#[test]
fn test_did_change_watched_files_mixed_change_types_batch() {
    let ts = create_temp_shard("mixed");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let notif = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWatchedFiles",
        "params": { "changes": [
            { "uri": "file:///tmp/a.rs", "type": 1 },
            { "uri": to_uri(&ts.shard), "type": 2 },
            { "uri": "file:///tmp/b.rs", "type": 3 }
        ] }
    });
    let res = dsp.dispatch(&notif);
    assert!(res);
}

#[test]
fn test_did_change_watched_files_consecutive_rapid_notifications() {
    let ts = create_temp_shard("rapid");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let notif = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWatchedFiles",
        "params": { "changes": [{ "uri": to_uri(&ts.shard), "type": 2 }] }
    });
    let r1 = dsp.dispatch(&notif);
    let r2 = dsp.dispatch(&notif);
    assert!(r1);
    assert!(r2);
}

#[test]
fn test_did_change_watched_files_pre_init_lifecycle_safe() {
    let ts = create_temp_shard("preinit");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let notif = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWatchedFiles",
        "params": { "changes": [{ "uri": to_uri(&ts.shard), "type": 2 }] }
    });
    let res = dsp.dispatch(&notif);
    assert!(res);
}

#[test]
fn test_did_change_watched_files_cancelled_request_set_safe() {
    let ts = create_temp_shard("canc_set");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    dsp.cancelled.lock().unwrap().insert(json!(999));

    let notif = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWatchedFiles",
        "params": { "changes": [{ "uri": to_uri(&ts.shard), "type": 2 }] }
    });
    let res = dsp.dispatch(&notif);
    assert!(res);
}

#[test]
fn test_did_change_watched_files_non_file_uri_skipped() {
    let ts = create_temp_shard("nonfile");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let notif = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWatchedFiles",
        "params": { "changes": [{ "uri": "untitled:Untitled-1", "type": 2 }] }
    });
    let res = dsp.dispatch(&notif);
    assert!(res);
}

#[test]
fn test_did_change_watched_files_malformed_change_entry_not_object() {
    let ts = create_temp_shard("malformed");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let notif = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWatchedFiles",
        "params": { "changes": ["not_an_object", 12345] }
    });
    let res = dsp.dispatch(&notif);
    assert!(res);
}

#[test]
fn test_did_change_watched_files_empty_uri_entry_skipped() {
    let ts = create_temp_shard("empty_uri");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let notif = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWatchedFiles",
        "params": { "changes": [{ "uri": "", "type": 2 }] }
    });
    let res = dsp.dispatch(&notif);
    assert!(res);
}
