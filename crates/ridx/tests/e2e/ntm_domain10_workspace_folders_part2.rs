/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for workspace/didChangeWorkspaceFolders part 2:
//! root re-addition, middle/head/tail removal, batches, and missing event fields.

use ridx::lsp::protocol::path_to_uri;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx_format::IndexWriter;
use serde_json::json;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static FOLDER_COUNTER_P2: AtomicU64 = AtomicU64::new(100);

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
    let id = FOLDER_COUNTER_P2.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_ws_p2_{}_{}_{}", name, std::process::id(), id));
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
fn test_did_change_workspace_folders_reopen_previously_removed_root() {
    let mut mr = MultiRootWorkspace::new();
    let ts1 = create_temp_shard("reopen_alpha");
    let uri = to_uri(&ts1.shard);

    let add_event = json!({
        "params": { "event": { "added": [{ "uri": uri, "name": "alpha" }], "removed": [] } }
    });
    mr.handle_did_change_workspace_folders(&add_event);
    assert_eq!(mr.len(), 1);

    let rm_event = json!({
        "params": { "event": { "added": [], "removed": [{ "uri": uri, "name": "alpha" }] } }
    });
    mr.handle_did_change_workspace_folders(&rm_event);
    assert_eq!(mr.len(), 0);

    // Re-add the same root
    mr.handle_did_change_workspace_folders(&add_event);
    assert_eq!(mr.len(), 1);
    assert_eq!(mr.first_workspace().unwrap().readers()[0].crate_name(), "reopen_alpha");
}

#[test]
fn test_did_change_workspace_folders_add_three_roots_distinct_order() {
    let mut mr = MultiRootWorkspace::new();
    let ts1 = create_temp_shard("first");
    let ts2 = create_temp_shard("second");
    let ts3 = create_temp_shard("third");

    let event = json!({
        "params": { "event": {
            "added": [
                { "uri": to_uri(&ts1.shard), "name": "1" },
                { "uri": to_uri(&ts2.shard), "name": "2" },
                { "uri": to_uri(&ts3.shard), "name": "3" }
            ],
            "removed": []
        } }
    });
    mr.handle_did_change_workspace_folders(&event);
    assert_eq!(mr.len(), 3);
    let names: Vec<String> = mr.all_workspaces().map(|ws| ws.readers()[0].crate_name().to_string()).collect();
    assert_eq!(names, vec!["first".to_string(), "second".to_string(), "third".to_string()]);
}

#[test]
fn test_did_change_workspace_folders_remove_middle_root_preserves_surrounding() {
    let mut mr = MultiRootWorkspace::new();
    let ts1 = create_temp_shard("head");
    let ts2 = create_temp_shard("middle");
    let ts3 = create_temp_shard("tail");

    let add_event = json!({
        "params": { "event": {
            "added": [
                { "uri": to_uri(&ts1.shard), "name": "h" },
                { "uri": to_uri(&ts2.shard), "name": "m" },
                { "uri": to_uri(&ts3.shard), "name": "t" }
            ],
            "removed": []
        } }
    });
    mr.handle_did_change_workspace_folders(&add_event);
    assert_eq!(mr.len(), 3);

    let rm_event = json!({
        "params": { "event": { "added": [], "removed": [{ "uri": to_uri(&ts2.shard), "name": "m" }] } }
    });
    mr.handle_did_change_workspace_folders(&rm_event);
    assert_eq!(mr.len(), 2);
    let names: Vec<String> = mr.all_workspaces().map(|ws| ws.readers()[0].crate_name().to_string()).collect();
    assert_eq!(names, vec!["head".to_string(), "tail".to_string()]);
}

#[test]
fn test_did_change_workspace_folders_remove_first_root_preserves_tail() {
    let mut mr = MultiRootWorkspace::new();
    let ts1 = create_temp_shard("head_to_drop");
    let ts2 = create_temp_shard("mid_kept");
    let ts3 = create_temp_shard("tail_kept");

    let add_event = json!({
        "params": { "event": {
            "added": [
                { "uri": to_uri(&ts1.shard), "name": "1" },
                { "uri": to_uri(&ts2.shard), "name": "2" },
                { "uri": to_uri(&ts3.shard), "name": "3" }
            ],
            "removed": []
        } }
    });
    mr.handle_did_change_workspace_folders(&add_event);

    let rm_event = json!({
        "params": { "event": { "added": [], "removed": [{ "uri": to_uri(&ts1.shard), "name": "1" }] } }
    });
    mr.handle_did_change_workspace_folders(&rm_event);
    assert_eq!(mr.len(), 2);
    let names: Vec<String> = mr.all_workspaces().map(|ws| ws.readers()[0].crate_name().to_string()).collect();
    assert_eq!(names, vec!["mid_kept".to_string(), "tail_kept".to_string()]);
}

#[test]
fn test_did_change_workspace_folders_remove_last_root_preserves_head() {
    let mut mr = MultiRootWorkspace::new();
    let ts1 = create_temp_shard("head_stay");
    let ts2 = create_temp_shard("mid_stay");
    let ts3 = create_temp_shard("tail_drop");

    let add_event = json!({
        "params": { "event": {
            "added": [
                { "uri": to_uri(&ts1.shard), "name": "1" },
                { "uri": to_uri(&ts2.shard), "name": "2" },
                { "uri": to_uri(&ts3.shard), "name": "3" }
            ],
            "removed": []
        } }
    });
    mr.handle_did_change_workspace_folders(&add_event);

    let rm_event = json!({
        "params": { "event": { "added": [], "removed": [{ "uri": to_uri(&ts3.shard), "name": "3" }] } }
    });
    mr.handle_did_change_workspace_folders(&rm_event);
    assert_eq!(mr.len(), 2);
    let names: Vec<String> = mr.all_workspaces().map(|ws| ws.readers()[0].crate_name().to_string()).collect();
    assert_eq!(names, vec!["head_stay".to_string(), "mid_stay".to_string()]);
}

#[test]
fn test_did_change_workspace_folders_duplicate_in_same_added_batch() {
    let mut mr = MultiRootWorkspace::new();
    let ts = create_temp_shard("dupe_batch");
    let uri = to_uri(&ts.shard);

    let event = json!({
        "params": { "event": {
            "added": [
                { "uri": uri, "name": "dupe1" },
                { "uri": uri, "name": "dupe2" }
            ],
            "removed": []
        } }
    });
    mr.handle_did_change_workspace_folders(&event);
    assert_eq!(mr.len(), 1);
}

#[test]
fn test_did_change_workspace_folders_add_and_remove_same_root_in_one_event() {
    let mut mr = MultiRootWorkspace::new();
    let ts = create_temp_shard("same_event");
    let uri = to_uri(&ts.shard);

    let event = json!({
        "params": { "event": {
            "added": [{ "uri": uri, "name": "add" }],
            "removed": [{ "uri": uri, "name": "rm" }]
        } }
    });
    // Removed is processed before added, so root remains added
    mr.handle_did_change_workspace_folders(&event);
    assert_eq!(mr.len(), 1);
    assert_eq!(mr.first_workspace().unwrap().readers()[0].crate_name(), "same_event");
}

#[test]
fn test_did_change_workspace_folders_missing_added_field_handled() {
    let mut mr = MultiRootWorkspace::new();
    let event = json!({
        "params": { "event": { "removed": [] } }
    });
    mr.handle_did_change_workspace_folders(&event);
    assert_eq!(mr.len(), 0);
}

#[test]
fn test_did_change_workspace_folders_missing_removed_field_handled() {
    let mut mr = MultiRootWorkspace::new();
    let ts = create_temp_shard("no_rm_field");
    let event = json!({
        "params": { "event": { "added": [{ "uri": to_uri(&ts.shard), "name": "add" }] } }
    });
    mr.handle_did_change_workspace_folders(&event);
    assert_eq!(mr.len(), 1);
}

#[test]
fn test_did_change_workspace_folders_null_event_handled() {
    let mut mr = MultiRootWorkspace::new();
    let event = json!({
        "params": { "event": serde_json::Value::Null }
    });
    mr.handle_did_change_workspace_folders(&event);
    assert_eq!(mr.len(), 0);
}
