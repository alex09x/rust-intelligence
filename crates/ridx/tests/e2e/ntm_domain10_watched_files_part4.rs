/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for workspace/didChangeWatchedFiles part 4:
//! symbol query updates, multiple shard updates, and edge URI schemes.

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
use std::thread::sleep;
use std::time::Duration;

static WF_P4_COUNTER: AtomicU64 = AtomicU64::new(300);

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
    let id = WF_P4_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_wf_p4_{}_{}_{}", name, std::process::id(), id));
    std::fs::create_dir_all(&dir).unwrap();
    let shard = dir.join("index.ridx");
    TempShard { dir, shard }
}

fn to_uri(p: &Path) -> String {
    path_to_uri(p.to_str().unwrap())
}

fn write_index_shard(path: &Path, name: &str, ver: &str) {
    let mut writer = IndexWriter::new(name, ver);
    let id = WF_P4_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_file_name(format!("tmp_{}_{}.ridx", name, id));
    writer.write_to(&mut File::create(&tmp).unwrap()).unwrap();
    std::fs::rename(&tmp, path).unwrap();
}

fn write_index_shard_with_defs(path: &Path, name: &str, ver: &str, def_names: &[&str]) {
    let mut writer = IndexWriter::new(name, ver);
    for (i, def_name) in def_names.iter().enumerate() {
        writer.add_definition(RawDefinition {
            def_id: (i + 1) as u32,
            kind: DefKind::Struct,
            visibility: 0,
            is_macro_expansion: false,
            name: def_name.to_string(),
            path: format!("{}::{}", name, def_name),
            signature: None,
            file: "src/lib.rs".to_string(),
            start_line: (i + 1) as u32,
            start_col: 1,
            end_line: (i + 1) as u32,
            end_col: 10,
        });
    }
    let id = WF_P4_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_file_name(format!("tmp_{}_{}.ridx", name, id));
    writer.write_to(&mut File::create(&tmp).unwrap()).unwrap();
    std::fs::rename(&tmp, path).unwrap();
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
        dispatch_lsp_message(msg.clone(), &mut self.writer, &mut self.lifecycle, &mut self.multi_root, &mut self.overlay_mgr, &self.cancelled, None).unwrap()
    }
}

#[test]
fn test_did_change_watched_files_reload_updates_workspace_symbol_search() {
    let ts = create_temp_shard("ws_sym_up");
    write_index_shard_with_defs(&ts.shard, "pkg_sym", "0.1.0", &["FirstOnly"]);
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    assert_eq!(dsp.multi_root.first_workspace().unwrap().search_symbols("SecondAdded").len(), 0);

    sleep(Duration::from_millis(20));
    write_index_shard_with_defs(&ts.shard, "pkg_sym", "0.2.0", &["FirstOnly", "SecondAdded"]);

    let notif = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWatchedFiles",
        "params": { "changes": [{ "uri": to_uri(&ts.shard), "type": 2 }] }
    });
    let res = dsp.dispatch(&notif);
    assert!(res);
    assert_eq!(dsp.multi_root.first_workspace().unwrap().search_symbols("SecondAdded").len(), 1);
}

#[test]
fn test_did_change_watched_files_reload_removes_deleted_definitions() {
    let ts = create_temp_shard("del_def");
    write_index_shard_with_defs(&ts.shard, "pkg_del", "0.1.0", &["WillBeDeleted"]);
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    assert_eq!(dsp.multi_root.first_workspace().unwrap().search_symbols("WillBeDeleted").len(), 1);

    sleep(Duration::from_millis(20));
    write_index_shard(&ts.shard, "pkg_del", "0.2.0");

    let notif = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWatchedFiles",
        "params": { "changes": [{ "uri": to_uri(&ts.shard), "type": 2 }] }
    });
    let res = dsp.dispatch(&notif);
    assert!(res);
    assert_eq!(dsp.multi_root.first_workspace().unwrap().search_symbols("WillBeDeleted").len(), 0);
}

#[test]
fn test_did_change_watched_files_multiple_shards_both_reloaded_simultaneously() {
    let ts1 = create_temp_shard("both1");
    let ts2 = create_temp_shard("both2");
    write_index_shard(&ts1.shard, "pkg_1", "1.0.0");
    write_index_shard(&ts2.shard, "pkg_2", "1.0.0");

    let mut dsp = TestDispatcher::new(ts1.shard.clone());
    dsp.multi_root.add_root(ts2.shard.clone());

    sleep(Duration::from_millis(20));
    write_index_shard(&ts1.shard, "pkg_1", "2.0.0");
    write_index_shard(&ts2.shard, "pkg_2", "2.0.0");

    let notif = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWatchedFiles",
        "params": { "changes": [
            { "uri": to_uri(&ts1.shard), "type": 2 },
            { "uri": to_uri(&ts2.shard), "type": 2 }
        ] }
    });
    let res = dsp.dispatch(&notif);
    assert!(res);

    let versions: Vec<String> = dsp.multi_root.all_workspaces().map(|ws| ws.readers()[0].crate_version().to_string()).collect();
    assert_eq!(versions, vec!["2.0.0".to_string(), "2.0.0".to_string()]);
}

#[test]
fn test_did_change_watched_files_duplicate_uri_in_changes_array() {
    let ts = create_temp_shard("dupe_uri");
    write_index_shard(&ts.shard, "pkg_dupe", "0.1.0");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let notif = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWatchedFiles",
        "params": { "changes": [
            { "uri": to_uri(&ts.shard), "type": 2 },
            { "uri": to_uri(&ts.shard), "type": 2 }
        ] }
    });
    let res = dsp.dispatch(&notif);
    assert!(res);
}

#[test]
fn test_did_change_watched_files_ignores_directory_change() {
    let ts = create_temp_shard("dir_ch");
    write_index_shard(&ts.shard, "pkg_dir", "0.1.0");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let notif = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWatchedFiles",
        "params": { "changes": [{ "uri": to_uri(&ts.dir), "type": 2 }] }
    });
    let res = dsp.dispatch(&notif);
    assert!(res);
}

#[test]
fn test_did_change_watched_files_relative_path_in_changes() {
    let ts = create_temp_shard("rel_p");
    write_index_shard(&ts.shard, "pkg_rel", "0.1.0");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let notif = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWatchedFiles",
        "params": { "changes": [{ "uri": "file://relative/path/shard.ridx", "type": 2 }] }
    });
    let res = dsp.dispatch(&notif);
    assert!(res);
}

#[test]
fn test_did_change_watched_files_uppercase_scheme_handled() {
    let ts = create_temp_shard("up_scheme");
    write_index_shard(&ts.shard, "pkg_up", "0.1.0");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let notif = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWatchedFiles",
        "params": { "changes": [{ "uri": "FILE:///tmp/up_shard.ridx", "type": 2 }] }
    });
    let res = dsp.dispatch(&notif);
    assert!(res);
}

#[test]
fn test_did_change_watched_files_reload_all_returns_false_when_empty_multi_root() {
    let mut mr = MultiRootWorkspace::new();
    let reloaded = mr.reload_all_if_modified();
    assert!(!reloaded);
}

#[test]
fn test_did_change_watched_files_multi_root_len_preserved_after_reload() {
    let ts1 = create_temp_shard("len1");
    let ts2 = create_temp_shard("len2");
    write_index_shard(&ts1.shard, "len_1", "0.1.0");
    write_index_shard(&ts2.shard, "len_2", "0.1.0");

    let mut dsp = TestDispatcher::new(ts1.shard.clone());
    dsp.multi_root.add_root(ts2.shard.clone());
    assert_eq!(dsp.multi_root.len(), 2);

    let notif = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWatchedFiles",
        "params": { "changes": [{ "uri": to_uri(&ts1.shard), "type": 2 }] }
    });
    dsp.dispatch(&notif);
    assert_eq!(dsp.multi_root.len(), 2);
}

#[test]
fn test_did_change_watched_files_first_workspace_preserved_after_reload() {
    let ts = create_temp_shard("first_ws");
    write_index_shard(&ts.shard, "first_crate", "0.1.0");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let notif = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWatchedFiles",
        "params": { "changes": [{ "uri": to_uri(&ts.shard), "type": 2 }] }
    });
    dsp.dispatch(&notif);
    assert_eq!(dsp.multi_root.first_workspace().unwrap().readers()[0].crate_name(), "first_crate");
}
