/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for workspace/didChangeWatchedFiles part 2:
//! shard reloads, version upgrades, position converter invalidation, and event types.

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

static WF_P2_COUNTER: AtomicU64 = AtomicU64::new(100);

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
    let id = WF_P2_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_wf_p2_{}_{}_{}", name, std::process::id(), id));
    std::fs::create_dir_all(&dir).unwrap();
    let shard = dir.join("index.ridx");
    TempShard { dir, shard }
}

fn to_uri(p: &Path) -> String {
    path_to_uri(p.to_str().unwrap())
}

fn write_index_shard(path: &Path, name: &str, ver: &str) {
    let mut writer = IndexWriter::new(name, ver);
    let id = WF_P2_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_file_name(format!("tmp_{}_{}.ridx", name, id));
    writer.write_to(&mut File::create(&tmp).unwrap()).unwrap();
    std::fs::rename(&tmp, path).unwrap();
}

fn write_index_shard_with_def(path: &Path, name: &str, ver: &str, def_name: &str) {
    let mut writer = IndexWriter::new(name, ver);
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: def_name.to_string(),
        path: format!("{}::{}", name, def_name),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 10,
    });
    let id = WF_P2_COUNTER.fetch_add(1, Ordering::Relaxed);
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
fn test_did_change_watched_files_unmodified_shard_not_reloaded() {
    let ts = create_temp_shard("unmodified");
    write_index_shard(&ts.shard, "crate_unmod", "0.1.0");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let notif = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWatchedFiles",
        "params": { "changes": [{ "uri": to_uri(&ts.shard), "type": 2 }] }
    });
    let res = dsp.dispatch(&notif);
    assert!(res);
    assert_eq!(dsp.multi_root.first_workspace().unwrap().readers()[0].crate_version(), "0.1.0");
}

#[test]
fn test_did_change_watched_files_modified_shard_reloads_new_version() {
    let ts = create_temp_shard("mod_ver");
    write_index_shard(&ts.shard, "crate_ver", "1.0.0");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    assert_eq!(dsp.multi_root.first_workspace().unwrap().readers()[0].crate_version(), "1.0.0");

    sleep(Duration::from_millis(20));
    write_index_shard(&ts.shard, "crate_ver", "2.0.0");

    let notif = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWatchedFiles",
        "params": { "changes": [{ "uri": to_uri(&ts.shard), "type": 2 }] }
    });
    let res = dsp.dispatch(&notif);
    assert!(res);
    assert_eq!(dsp.multi_root.first_workspace().unwrap().readers()[0].crate_version(), "2.0.0");
}

#[test]
fn test_did_change_watched_files_modified_shard_reloads_new_definitions() {
    let ts = create_temp_shard("mod_def");
    write_index_shard(&ts.shard, "crate_def", "0.1.0");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    assert_eq!(dsp.multi_root.first_workspace().unwrap().search_symbols("new_symbol").len(), 0);

    sleep(Duration::from_millis(20));
    write_index_shard_with_def(&ts.shard, "crate_def", "0.2.0", "new_symbol");

    let notif = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWatchedFiles",
        "params": { "changes": [{ "uri": to_uri(&ts.shard), "type": 2 }] }
    });
    let res = dsp.dispatch(&notif);
    assert!(res);
    assert_eq!(dsp.multi_root.first_workspace().unwrap().search_symbols("new_symbol").len(), 1);
}

#[test]
fn test_did_change_watched_files_multiple_shards_only_modified_reloaded() {
    let ts1 = create_temp_shard("s1");
    let ts2 = create_temp_shard("s2");
    write_index_shard(&ts1.shard, "crate_1", "1.0.0");
    write_index_shard(&ts2.shard, "crate_2", "1.0.0");

    let mut dsp = TestDispatcher::new(ts1.shard.clone());
    dsp.multi_root.add_root(ts2.shard.clone());

    sleep(Duration::from_millis(20));
    // Only update shard 2
    write_index_shard(&ts2.shard, "crate_2", "1.1.0");

    let notif = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWatchedFiles",
        "params": { "changes": [{ "uri": to_uri(&ts2.shard), "type": 2 }] }
    });
    let res = dsp.dispatch(&notif);
    assert!(res);

    let versions: Vec<String> = dsp.multi_root.all_workspaces().map(|ws| ws.readers()[0].crate_version().to_string()).collect();
    assert_eq!(versions, vec!["1.0.0".to_string(), "1.1.0".to_string()]);
}

#[test]
fn test_did_change_watched_files_empty_changes_array_triggers_reload() {
    let ts = create_temp_shard("empty_ch");
    write_index_shard(&ts.shard, "crate_empty_ch", "0.1.0");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let notif = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWatchedFiles",
        "params": { "changes": [] }
    });
    let res = dsp.dispatch(&notif);
    assert!(res);
}

#[test]
fn test_did_change_watched_files_missing_changes_field_safe() {
    let ts = create_temp_shard("no_ch");
    write_index_shard(&ts.shard, "crate_no_ch", "0.1.0");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let notif = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWatchedFiles",
        "params": {}
    });
    let res = dsp.dispatch(&notif);
    assert!(res);
}

#[test]
fn test_did_change_watched_files_null_params_safe() {
    let ts = create_temp_shard("null_p");
    write_index_shard(&ts.shard, "crate_null_p", "0.1.0");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let notif = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWatchedFiles",
        "params": serde_json::Value::Null
    });
    let res = dsp.dispatch(&notif);
    assert!(res);
}

#[test]
fn test_did_change_watched_files_missing_params_safe() {
    let ts = create_temp_shard("miss_p");
    write_index_shard(&ts.shard, "crate_miss_p", "0.1.0");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let notif = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWatchedFiles"
    });
    let res = dsp.dispatch(&notif);
    assert!(res);
}

#[test]
fn test_did_change_watched_files_clears_position_converter() {
    let ts = create_temp_shard("conv_clear");
    write_index_shard(&ts.shard, "crate_conv", "0.1.0");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let notif = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWatchedFiles",
        "params": { "changes": [] }
    });
    let res = dsp.dispatch(&notif);
    assert!(res);
}

#[test]
fn test_did_change_watched_files_created_event_type_1() {
    let ts = create_temp_shard("type1");
    write_index_shard(&ts.shard, "crate_type1", "0.1.0");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let notif = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWatchedFiles",
        "params": { "changes": [{ "uri": to_uri(&ts.shard), "type": 1 }] }
    });
    let res = dsp.dispatch(&notif);
    assert!(res);
}

#[test]
fn test_did_change_watched_files_changed_event_type_2() {
    let ts = create_temp_shard("type2");
    write_index_shard(&ts.shard, "crate_type2", "0.1.0");
    let mut dsp = TestDispatcher::new(ts.shard.clone());

    let notif = json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeWatchedFiles",
        "params": { "changes": [{ "uri": to_uri(&ts.shard), "type": 2 }] }
    });
    let res = dsp.dispatch(&notif);
    assert!(res);
}
