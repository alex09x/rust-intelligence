/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for workspace/didChangeWorkspaceFolders part 4:
//! dispatch_lsp_message execution, lifecycle interaction, cancellation sets, and rapid transitions.

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

static FOLDER_COUNTER_P4: AtomicU64 = AtomicU64::new(300);

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
    let id = FOLDER_COUNTER_P4.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_ws_p4_{}_{}_{}", name, std::process::id(), id));
    std::fs::create_dir_all(&dir).unwrap();
    let shard = dir.join("index.ridx");
    let mut writer = IndexWriter::new(name, "0.1.0");
    writer.write_to(&mut File::create(&shard).unwrap()).unwrap();
    TempShard { dir, shard }
}

fn to_uri(p: &Path) -> String {
    path_to_uri(p.to_str().unwrap())
}

#[test]
fn test_did_change_workspace_folders_dispatch_via_notification() {
    let mut mr = MultiRootWorkspace::new();
    let (mut lifecycle, mut overlay_mgr) = (LifecycleManager::new(), OverlayManager::new());
    let (cancelled, mut writer) = (Arc::new(Mutex::new(HashSet::new())), Vec::new());
    let ts = create_temp_shard("notif_add");

    let notif = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWorkspaceFolders",
        "params": { "event": { "added": [{ "uri": to_uri(&ts.shard), "name": "na" }], "removed": [] } }
    });

    let res = dispatch_lsp_message(notif, &mut writer, &mut lifecycle, &mut mr, &mut overlay_mgr, &cancelled, None).unwrap();
    assert!(res);
    assert_eq!(mr.len(), 1);
}

#[test]
fn test_did_change_workspace_folders_dispatch_ignores_if_empty_event() {
    let mut mr = MultiRootWorkspace::new();
    let (mut lifecycle, mut overlay_mgr) = (LifecycleManager::new(), OverlayManager::new());
    let (cancelled, mut writer) = (Arc::new(Mutex::new(HashSet::new())), Vec::new());

    let notif = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWorkspaceFolders",
        "params": { "event": { "added": [], "removed": [] } }
    });

    let res = dispatch_lsp_message(notif, &mut writer, &mut lifecycle, &mut mr, &mut overlay_mgr, &cancelled, None).unwrap();
    assert!(res);
    assert_eq!(mr.len(), 0);
}

#[test]
fn test_did_change_workspace_folders_dispatch_with_uninitialized_state() {
    let mut mr = MultiRootWorkspace::new();
    let (mut lifecycle, mut overlay_mgr) = (LifecycleManager::new(), OverlayManager::new());
    let (cancelled, mut writer) = (Arc::new(Mutex::new(HashSet::new())), Vec::new());
    let ts = create_temp_shard("preinit_root");

    let notif = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWorkspaceFolders",
        "params": { "event": { "added": [{ "uri": to_uri(&ts.shard), "name": "pi" }], "removed": [] } }
    });

    let res = dispatch_lsp_message(notif, &mut writer, &mut lifecycle, &mut mr, &mut overlay_mgr, &cancelled, None).unwrap();
    assert!(res);
}

#[test]
fn test_did_change_workspace_folders_dispatch_with_cancelled_token() {
    let mut mr = MultiRootWorkspace::new();
    let (mut lifecycle, mut overlay_mgr) = (LifecycleManager::new(), OverlayManager::new());
    let mut cancel_set = HashSet::new();
    cancel_set.insert(json!(42));
    let (cancelled, mut writer) = (Arc::new(Mutex::new(cancel_set)), Vec::new());
    let ts = create_temp_shard("cancel_token");

    let notif = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWorkspaceFolders",
        "params": { "event": { "added": [{ "uri": to_uri(&ts.shard), "name": "ct" }], "removed": [] } }
    });

    let res = dispatch_lsp_message(notif, &mut writer, &mut lifecycle, &mut mr, &mut overlay_mgr, &cancelled, None).unwrap();
    assert!(res);
    assert_eq!(mr.len(), 1);
}

#[test]
fn test_did_change_workspace_folders_multiple_shards_same_event_dispatch() {
    let mut mr = MultiRootWorkspace::new();
    let (mut lifecycle, mut overlay_mgr) = (LifecycleManager::new(), OverlayManager::new());
    let (cancelled, mut writer) = (Arc::new(Mutex::new(HashSet::new())), Vec::new());
    let ts1 = create_temp_shard("batch_d1");
    let ts2 = create_temp_shard("batch_d2");

    let notif = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWorkspaceFolders",
        "params": { "event": {
            "added": [
                { "uri": to_uri(&ts1.shard), "name": "1" },
                { "uri": to_uri(&ts2.shard), "name": "2" }
            ],
            "removed": []
        } }
    });

    let res = dispatch_lsp_message(notif, &mut writer, &mut lifecycle, &mut mr, &mut overlay_mgr, &cancelled, None).unwrap();
    assert!(res);
    assert_eq!(mr.len(), 2);
}

#[test]
fn test_did_change_workspace_folders_remove_via_notification_dispatch() {
    let mut mr = MultiRootWorkspace::new();
    let (mut lifecycle, mut overlay_mgr) = (LifecycleManager::new(), OverlayManager::new());
    let (cancelled, mut writer) = (Arc::new(Mutex::new(HashSet::new())), Vec::new());
    let ts = create_temp_shard("to_remove_dsp");
    let uri = to_uri(&ts.shard);

    let notif_add = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWorkspaceFolders",
        "params": { "event": { "added": [{ "uri": uri, "name": "tr" }], "removed": [] } }
    });
    dispatch_lsp_message(notif_add, &mut writer, &mut lifecycle, &mut mr, &mut overlay_mgr, &cancelled, None).unwrap();
    assert_eq!(mr.len(), 1);

    let notif_rm = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWorkspaceFolders",
        "params": { "event": { "added": [], "removed": [{ "uri": uri, "name": "tr" }] } }
    });
    dispatch_lsp_message(notif_rm, &mut writer, &mut lifecycle, &mut mr, &mut overlay_mgr, &cancelled, None).unwrap();
    assert_eq!(mr.len(), 0);
}

#[test]
fn test_did_change_workspace_folders_rapid_add_remove_sequence() {
    let mut mr = MultiRootWorkspace::new();
    let ts_a = create_temp_shard("seq_a");
    let ts_b = create_temp_shard("seq_b");
    let ts_c = create_temp_shard("seq_c");

    // Add A and B
    let ev1 = json!({
        "params": { "event": {
            "added": [{ "uri": to_uri(&ts_a.shard), "name": "a" }, { "uri": to_uri(&ts_b.shard), "name": "b" }],
            "removed": []
        } }
    });
    mr.handle_did_change_workspace_folders(&ev1);
    assert_eq!(mr.len(), 2);

    // Remove A, add C
    let ev2 = json!({
        "params": { "event": {
            "added": [{ "uri": to_uri(&ts_c.shard), "name": "c" }],
            "removed": [{ "uri": to_uri(&ts_a.shard), "name": "a" }]
        } }
    });
    mr.handle_did_change_workspace_folders(&ev2);
    assert_eq!(mr.len(), 2);

    // Remove B
    let ev3 = json!({
        "params": { "event": {
            "added": [],
            "removed": [{ "uri": to_uri(&ts_b.shard), "name": "b" }]
        } }
    });
    mr.handle_did_change_workspace_folders(&ev3);
    assert_eq!(mr.len(), 1);
    assert_eq!(mr.first_workspace().unwrap().readers()[0].crate_name(), "seq_c");
}

#[test]
fn test_did_change_workspace_folders_non_existent_file_path_skipped_in_dispatch() {
    let mut mr = MultiRootWorkspace::new();
    let (mut lifecycle, mut overlay_mgr) = (LifecycleManager::new(), OverlayManager::new());
    let (cancelled, mut writer) = (Arc::new(Mutex::new(HashSet::new())), Vec::new());

    let notif = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWorkspaceFolders",
        "params": { "event": { "added": [{ "uri": "file:///non/existent/random_987654.ridx", "name": "x" }], "removed": [] } }
    });

    let res = dispatch_lsp_message(notif, &mut writer, &mut lifecycle, &mut mr, &mut overlay_mgr, &cancelled, None).unwrap();
    assert!(res);
    assert_eq!(mr.len(), 0);
}

#[test]
fn test_did_change_workspace_folders_http_uri_rejected() {
    let mut mr = MultiRootWorkspace::new();
    let event = json!({
        "params": { "event": { "added": [{ "uri": "http://example.com/shard.ridx", "name": "net" }], "removed": [] } }
    });
    mr.handle_did_change_workspace_folders(&event);
    assert_eq!(mr.len(), 0);
}

#[test]
fn test_did_change_workspace_folders_relative_file_uri_skipped() {
    let mut mr = MultiRootWorkspace::new();
    let event = json!({
        "params": { "event": { "added": [{ "uri": "file://relative_without_root", "name": "rel" }], "removed": [] } }
    });
    mr.handle_did_change_workspace_folders(&event);
    assert_eq!(mr.len(), 0);
}
