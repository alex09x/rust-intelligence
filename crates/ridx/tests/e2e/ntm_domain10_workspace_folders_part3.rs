/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for workspace/didChangeWorkspaceFolders part 3:
//! longest prefix routing, fallback workspace, empty states, and trailing slash URIs.

use ridx::lsp::protocol::path_to_uri;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx_format::IndexWriter;
use serde_json::json;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static FOLDER_COUNTER_P3: AtomicU64 = AtomicU64::new(200);

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
    let id = FOLDER_COUNTER_P3.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_ws_p3_{}_{}_{}", name, std::process::id(), id));
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
fn test_did_change_workspace_folders_routing_longest_prefix() {
    let mut mr = MultiRootWorkspace::new();
    let ts_parent = create_temp_shard("parent_crate");
    let nested_dir = ts_parent.dir.join("sub_module");
    std::fs::create_dir_all(&nested_dir).unwrap();
    let nested_shard = nested_dir.join("index.ridx");
    let mut writer = IndexWriter::new("nested_crate", "0.1.0");
    writer.write_to(&mut File::create(&nested_shard).unwrap()).unwrap();

    let event = json!({
        "params": { "event": {
            "added": [
                { "uri": to_uri(&ts_parent.shard), "name": "parent" },
                { "uri": to_uri(&nested_shard), "name": "nested" }
            ],
            "removed": []
        } }
    });
    mr.handle_did_change_workspace_folders(&event);
    assert_eq!(mr.len(), 2);

    let target_in_nested = nested_dir.join("lib.rs");
    let ws = mr.find_workspace_for_path(&target_in_nested).unwrap();
    assert_eq!(ws.readers()[0].crate_name(), "nested_crate");

    let target_in_parent = ts_parent.dir.join("lib.rs");
    let ws_p = mr.find_workspace_for_path(&target_in_parent).unwrap();
    assert_eq!(ws_p.readers()[0].crate_name(), "parent_crate");
}

#[test]
fn test_did_change_workspace_folders_file_outside_all_roots_falls_back() {
    let mut mr = MultiRootWorkspace::new();
    let ts = create_temp_shard("fallback_crate");
    let event = json!({
        "params": { "event": {
            "added": [{ "uri": to_uri(&ts.shard), "name": "fb" }],
            "removed": []
        } }
    });
    mr.handle_did_change_workspace_folders(&event);
    assert_eq!(mr.len(), 1);

    let external_file = PathBuf::from("/completely/different/external/file.rs");
    let ws = mr.find_workspace_for_path(&external_file).unwrap();
    assert_eq!(ws.readers()[0].crate_name(), "fallback_crate");
}

#[test]
fn test_did_change_workspace_folders_remove_root_clears_first_workspace() {
    let mut mr = MultiRootWorkspace::new();
    let ts = create_temp_shard("to_drop");
    let uri = to_uri(&ts.shard);

    let add_event = json!({
        "params": { "event": { "added": [{ "uri": uri, "name": "drop" }], "removed": [] } }
    });
    mr.handle_did_change_workspace_folders(&add_event);
    assert_eq!(mr.len(), 1);

    let rm_event = json!({
        "params": { "event": { "added": [], "removed": [{ "uri": uri, "name": "drop" }] } }
    });
    mr.handle_did_change_workspace_folders(&rm_event);
    assert_eq!(mr.len(), 0);
}

#[test]
fn test_did_change_workspace_folders_uri_trailing_slash_handled() {
    let mut mr = MultiRootWorkspace::new();
    let ts = create_temp_shard("trailing_slash");
    let mut uri = to_uri(&ts.dir);
    uri.push('/');

    let event = json!({
        "params": { "event": { "added": [{ "uri": uri, "name": "ts" }], "removed": [] } }
    });
    mr.handle_did_change_workspace_folders(&event);
    assert_eq!(mr.len(), 1);
    assert_eq!(mr.first_workspace().unwrap().readers()[0].crate_name(), "trailing_slash");
}

#[test]
fn test_did_change_workspace_folders_clear_removes_all_tracked_roots() {
    let mut mr = MultiRootWorkspace::new();
    let ts1 = create_temp_shard("c1");
    let ts2 = create_temp_shard("c2");

    let event = json!({
        "params": { "event": {
            "added": [
                { "uri": to_uri(&ts1.shard), "name": "1" },
                { "uri": to_uri(&ts2.shard), "name": "2" }
            ],
            "removed": []
        } }
    });
    mr.handle_did_change_workspace_folders(&event);
    assert_eq!(mr.len(), 2);

    mr.clear();
    assert_eq!(mr.len(), 0);
}

#[test]
fn test_did_change_workspace_folders_find_workspace_on_empty_returns_none() {
    let mr = MultiRootWorkspace::new();
    let test_path = PathBuf::from("/test/path.rs");
    assert_eq!(mr.find_workspace_for_path(&test_path).map(|_| ()), None);
}

#[test]
fn test_did_change_workspace_folders_reopen_from_slice() {
    let ts1 = create_temp_shard("slice1");
    let ts2 = create_temp_shard("slice2");

    let paths = vec![ts1.shard.clone(), ts2.shard.clone()];
    let mut mr = MultiRootWorkspace::new();
    mr.open_from_folders(&paths);
    assert_eq!(mr.len(), 2);
    let names: Vec<String> = mr.all_workspaces().map(|ws| ws.readers()[0].crate_name().to_string()).collect();
    assert_eq!(names, vec!["slice1".to_string(), "slice2".to_string()]);
}

#[test]
fn test_did_change_workspace_folders_removed_entry_without_uri_skipped() {
    let mut mr = MultiRootWorkspace::new();
    let ts = create_temp_shard("kept");
    let event_add = json!({
        "params": { "event": { "added": [{ "uri": to_uri(&ts.shard), "name": "kept" }], "removed": [] } }
    });
    mr.handle_did_change_workspace_folders(&event_add);
    assert_eq!(mr.len(), 1);

    let event_rm = json!({
        "params": { "event": { "added": [], "removed": [{ "name": "no_uri_key" }] } }
    });
    mr.handle_did_change_workspace_folders(&event_rm);
    assert_eq!(mr.len(), 1);
}

#[test]
fn test_did_change_workspace_folders_remove_already_empty_is_noop() {
    let mut mr = MultiRootWorkspace::new();
    let event = json!({
        "params": { "event": { "added": [], "removed": [{ "uri": "file:///tmp/does_not_exist", "name": "fake" }] } }
    });
    mr.handle_did_change_workspace_folders(&event);
    assert_eq!(mr.len(), 0);
}

#[test]
fn test_did_change_workspace_folders_add_folder_path_discovers_shards() {
    let mut mr = MultiRootWorkspace::new();
    let ts = create_temp_shard("folder_scan");
    let folder_uri = to_uri(&ts.dir);

    let event = json!({
        "params": { "event": { "added": [{ "uri": folder_uri, "name": "folder" }], "removed": [] } }
    });
    mr.handle_did_change_workspace_folders(&event);
    assert_eq!(mr.len(), 1);
    assert_eq!(mr.first_workspace().unwrap().readers()[0].crate_name(), "folder_scan");
}
