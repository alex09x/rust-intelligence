/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for workspace/didChangeWorkspaceFolders (Domain 10).

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

static FOLDER_COUNTER: AtomicU64 = AtomicU64::new(1);

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
    let id = FOLDER_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_ws_{}_{}_{}", name, std::process::id(), id));
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
fn test_did_change_workspace_folders_empty_added_and_removed_is_noop() {
    let mut mr = MultiRootWorkspace::new();
    let ts1 = create_temp_shard("alpha");
    assert!(mr.add_root(ts1.shard.clone()));
    assert_eq!(mr.len(), 1);

    let event = json!({ "params": { "event": { "added": [], "removed": [] } } });
    mr.handle_did_change_workspace_folders(&event);
    assert_eq!(mr.len(), 1);
    assert_eq!(mr.first_workspace().unwrap().readers()[0].crate_name(), "alpha");
}

#[test]
fn test_did_change_workspace_folders_missing_event_object_is_noop() {
    let mut mr = MultiRootWorkspace::new();
    let ts1 = create_temp_shard("beta");
    assert!(mr.add_root(ts1.shard.clone()));
    mr.handle_did_change_workspace_folders(&json!({ "params": {} }));
    assert_eq!(mr.len(), 1);
}

#[test]
fn test_did_change_workspace_folders_add_single_root() {
    let mut mr = MultiRootWorkspace::new();
    let ts1 = create_temp_shard("root1");
    let event = json!({
        "params": { "event": { "added": [{ "uri": to_uri(&ts1.shard), "name": "root1" }], "removed": [] } }
    });
    mr.handle_did_change_workspace_folders(&event);
    assert_eq!(mr.len(), 1);
    assert_eq!(mr.first_workspace().unwrap().readers()[0].crate_name(), "root1");
}

#[test]
fn test_did_change_workspace_folders_remove_single_root() {
    let mut mr = MultiRootWorkspace::new();
    let ts1 = create_temp_shard("r1");
    let ts2 = create_temp_shard("r2");
    assert!(mr.add_root(ts1.shard.clone()));
    assert!(mr.add_root(ts2.shard.clone()));
    assert_eq!(mr.len(), 2);

    let event = json!({
        "params": { "event": { "added": [], "removed": [{ "uri": to_uri(&ts1.shard), "name": "r1" }] } }
    });
    mr.handle_did_change_workspace_folders(&event);
    assert_eq!(mr.len(), 1);
    assert_eq!(mr.first_workspace().unwrap().readers()[0].crate_name(), "r2");
}

#[test]
fn test_did_change_workspace_folders_simultaneous_add_and_remove() {
    let mut mr = MultiRootWorkspace::new();
    let ts1 = create_temp_shard("old_root");
    let ts2 = create_temp_shard("new_root");
    assert!(mr.add_root(ts1.shard.clone()));
    let event = json!({
        "params": { "event": {
            "added": [{ "uri": to_uri(&ts2.shard), "name": "new_root" }],
            "removed": [{ "uri": to_uri(&ts1.shard), "name": "old_root" }]
        } }
    });
    mr.handle_did_change_workspace_folders(&event);
    assert_eq!(mr.len(), 1);
    assert_eq!(mr.first_workspace().unwrap().readers()[0].crate_name(), "new_root");
}

#[test]
fn test_did_change_workspace_folders_remove_nonexistent_root_is_noop() {
    let mut mr = MultiRootWorkspace::new();
    let ts1 = create_temp_shard("persist");
    assert!(mr.add_root(ts1.shard.clone()));

    let fake_uri = to_uri(Path::new("/nonexistent/ridx/dir"));
    let event = json!({
        "params": { "event": { "added": [], "removed": [{ "uri": fake_uri, "name": "ghost" }] } }
    });
    mr.handle_did_change_workspace_folders(&event);
    assert_eq!(mr.len(), 1);
}

#[test]
fn test_did_change_workspace_folders_duplicate_addition_ignored() {
    let mut mr = MultiRootWorkspace::new();
    let ts1 = create_temp_shard("dup");
    assert!(mr.add_root(ts1.shard.clone()));

    let event = json!({
        "params": { "event": { "added": [{ "uri": to_uri(&ts1.shard), "name": "dup" }], "removed": [] } }
    });
    mr.handle_did_change_workspace_folders(&event);
    assert_eq!(mr.len(), 1);
}

#[test]
fn test_did_change_workspace_folders_invalid_file_uri_skipped() {
    let mut mr = MultiRootWorkspace::new();
    let ts1 = create_temp_shard("valid");
    assert!(mr.add_root(ts1.shard.clone()));

    let event = json!({
        "params": { "event": { "added": [{ "uri": "not a uri at all", "name": "bad" }], "removed": [] } }
    });
    mr.handle_did_change_workspace_folders(&event);
    assert_eq!(mr.len(), 1);
}

#[test]
fn test_did_change_workspace_folders_non_file_scheme_skipped() {
    let mut mr = MultiRootWorkspace::new();
    let ts1 = create_temp_shard("scheme_test");
    assert!(mr.add_root(ts1.shard.clone()));

    let event = json!({
        "params": { "event": { "added": [{ "uri": "http://example.com/shard", "name": "remote" }], "removed": [] } }
    });
    mr.handle_did_change_workspace_folders(&event);
    assert_eq!(mr.len(), 1);
}

#[test]
fn test_did_change_workspace_folders_nonexistent_path_skipped() {
    let mut mr = MultiRootWorkspace::new();
    let uri = to_uri(Path::new("/tmp/certainly_does_not_exist_ridx_9999"));
    let event = json!({
        "params": { "event": { "added": [{ "uri": uri, "name": "ghost" }], "removed": [] } }
    });
    mr.handle_did_change_workspace_folders(&event);
    assert_eq!(mr.len(), 0);
}

#[test]
fn test_did_change_workspace_folders_empty_params_safe() {
    let mut mr = MultiRootWorkspace::new();
    let msg = json!({});
    mr.handle_did_change_workspace_folders(&msg);
    assert_eq!(mr.len(), 0);
}

#[test]
fn test_did_change_workspace_folders_add_multiple_roots_batch() {
    let mut mr = MultiRootWorkspace::new();
    let ts1 = create_temp_shard("batch1");
    let ts2 = create_temp_shard("batch2");
    let ts3 = create_temp_shard("batch3");

    let event = json!({
        "params": { "event": {
            "added": [
                { "uri": to_uri(&ts1.shard), "name": "b1" },
                { "uri": to_uri(&ts2.shard), "name": "b2" },
                { "uri": to_uri(&ts3.shard), "name": "b3" }
            ],
            "removed": []
        } }
    });
    mr.handle_did_change_workspace_folders(&event);
    assert_eq!(mr.len(), 3);
}

#[test]
fn test_did_change_workspace_folders_remove_multiple_roots_batch() {
    let mut mr = MultiRootWorkspace::new();
    let ts1 = create_temp_shard("rem1");
    let ts2 = create_temp_shard("rem2");
    let ts3 = create_temp_shard("rem3");
    assert!(mr.add_root(ts1.shard.clone()));
    assert!(mr.add_root(ts2.shard.clone()));
    assert!(mr.add_root(ts3.shard.clone()));
    assert_eq!(mr.len(), 3);

    let event = json!({
        "params": { "event": {
            "added": [],
            "removed": [
                { "uri": to_uri(&ts1.shard), "name": "rem1" },
                { "uri": to_uri(&ts2.shard), "name": "rem2" }
            ]
        } }
    });
    mr.handle_did_change_workspace_folders(&event);
    assert_eq!(mr.len(), 1);
    assert_eq!(mr.first_workspace().unwrap().readers()[0].crate_name(), "rem3");
}

#[test]
fn test_did_change_workspace_folders_dispatched_via_lsp_message() {
    let mut mr = MultiRootWorkspace::new();
    let (mut lifecycle, mut overlay_mgr) = (LifecycleManager::new(), OverlayManager::new());
    let (cancelled, mut writer) = (Arc::new(Mutex::new(HashSet::new())), Vec::new());
    let ts1 = create_temp_shard("dispatched_root");
    let notif = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWorkspaceFolders",
        "params": { "event": { "added": [{ "uri": to_uri(&ts1.shard), "name": "dsp" }], "removed": [] } }
    });

    let res = dispatch_lsp_message(notif, &mut writer, &mut lifecycle, &mut mr, &mut overlay_mgr, &cancelled, None).unwrap();
    assert!(res);
    assert_eq!(mr.len(), 1);
}

#[test]
fn test_did_change_workspace_folders_routing_switches_after_removal() {
    let mut mr = MultiRootWorkspace::new();
    let ts1 = create_temp_shard("rt1");
    let ts2 = create_temp_shard("rt2");
    assert!(mr.add_root(ts1.shard.clone()));
    assert!(mr.add_root(ts2.shard.clone()));

    let target_file = ts2.dir.join("src/lib.rs");
    assert_eq!(mr.find_workspace_for_path(&target_file).unwrap().readers()[0].crate_name(), "rt2");

    let event = json!({
        "params": { "event": { "added": [], "removed": [{ "uri": to_uri(&ts2.shard), "name": "rt2" }] } }
    });
    mr.handle_did_change_workspace_folders(&event);

    assert_eq!(mr.find_workspace_for_path(&target_file).unwrap().readers()[0].crate_name(), "rt1");
}

#[test]
fn test_did_change_workspace_folders_all_workspaces_order_preserved() {
    let mut mr = MultiRootWorkspace::new();
    let ts1 = create_temp_shard("order_a");
    let ts2 = create_temp_shard("order_b");

    let event = json!({
        "params": { "event": {
            "added": [
                { "uri": to_uri(&ts1.shard), "name": "ord_a" },
                { "uri": to_uri(&ts2.shard), "name": "ord_b" }
            ],
            "removed": []
        } }
    });
    mr.handle_did_change_workspace_folders(&event);
    let names: Vec<String> = mr.all_workspaces().map(|ws| ws.readers()[0].crate_name().to_string()).collect();
    assert_eq!(names, vec!["order_a".to_string(), "order_b".to_string()]);
}

#[test]
fn test_did_change_workspace_folders_empty_uri_entry_skipped() {
    let mut mr = MultiRootWorkspace::new();
    let event = json!({
        "params": { "event": { "added": [{ "uri": "", "name": "empty" }], "removed": [] } }
    });
    mr.handle_did_change_workspace_folders(&event);
    assert_eq!(mr.len(), 0);
}

#[test]
fn test_did_change_workspace_folders_malformed_added_entry_not_object() {
    let mut mr = MultiRootWorkspace::new();
    let event = json!({
        "params": { "event": { "added": ["not-an-object", 42], "removed": [] } }
    });
    mr.handle_did_change_workspace_folders(&event);
    assert_eq!(mr.len(), 0);
}

#[test]
fn test_did_change_workspace_folders_malformed_removed_entry_not_object() {
    let mut mr = MultiRootWorkspace::new();
    let ts1 = create_temp_shard("safe_malformed");
    assert!(mr.add_root(ts1.shard.clone()));

    let event = json!({
        "params": { "event": { "added": [], "removed": [null, 123] } }
    });
    mr.handle_did_change_workspace_folders(&event);
    assert_eq!(mr.len(), 1);
}

#[test]
fn test_did_change_workspace_folders_remove_all_roots_via_event() {
    let mut mr = MultiRootWorkspace::new();
    let ts1 = create_temp_shard("all1");
    let ts2 = create_temp_shard("all2");
    assert!(mr.add_root(ts1.shard.clone()));
    assert!(mr.add_root(ts2.shard.clone()));
    assert_eq!(mr.len(), 2);

    let event = json!({
        "params": { "event": {
            "added": [],
            "removed": [
                { "uri": to_uri(&ts1.shard), "name": "all1" },
                { "uri": to_uri(&ts2.shard), "name": "all2" }
            ]
        } }
    });
    mr.handle_did_change_workspace_folders(&event);
    assert_eq!(mr.len(), 0);
}

#[test]
fn test_multiroot_api_clear_removes_all_roots() {
    let mut mr = MultiRootWorkspace::new();
    let ts1 = create_temp_shard("c1");
    let ts2 = create_temp_shard("c2");
    assert!(mr.add_root(ts1.shard.clone()));
    assert!(mr.add_root(ts2.shard.clone()));
    assert_eq!(mr.len(), 2);
    mr.clear();
    assert_eq!(mr.len(), 0);
}

#[test]
fn test_multiroot_api_find_workspace_on_empty_is_none() {
    let mr = MultiRootWorkspace::new();
    assert!(mr.find_workspace_for_path(Path::new("/tmp/foo")).is_none());
}

#[test]
fn test_multiroot_api_doc_ws_matches_root_file() {
    let mut mr = MultiRootWorkspace::new();
    let ts1 = create_temp_shard("doc_root");
    assert!(mr.add_root(ts1.shard.clone()));
    let target = ts1.dir.join("src/lib.rs");
    let msg = json!({ "params": { "textDocument": { "uri": to_uri(&target) } } });
    let ws = mr.doc_ws(&msg);
    assert_eq!(ws.unwrap().readers()[0].crate_name(), "doc_root");
}

#[test]
fn test_multiroot_api_doc_ws_empty_uri_falls_back() {
    let mut mr = MultiRootWorkspace::new();
    let ts1 = create_temp_shard("fallback_root");
    assert!(mr.add_root(ts1.shard.clone()));
    let msg = json!({ "params": { "textDocument": { "uri": "" } } });
    assert_eq!(mr.doc_ws(&msg).unwrap().readers()[0].crate_name(), "fallback_root");
}

#[test]
fn test_multiroot_api_reopen_from_slice() {
    let ts1 = create_temp_shard("slice1");
    let ts2 = create_temp_shard("slice2");
    let roots = vec![ts1.shard.clone(), ts2.shard.clone()];
    let mut mr = MultiRootWorkspace::new();
    mr.open_from_folders(&roots);
    assert_eq!(mr.len(), 2);
}
