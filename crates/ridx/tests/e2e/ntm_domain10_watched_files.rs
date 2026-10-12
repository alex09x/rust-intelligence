/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for workspace/didChangeWatchedFiles (Domain 10).
//! Dispatches LSP notifications and verifies observable index reload effects.

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

static WATCH_COUNTER: AtomicU64 = AtomicU64::new(1);

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
    let id = WATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_wf_{}_{}_{}", name, std::process::id(), id));
    std::fs::create_dir_all(&dir).unwrap();
    let shard = dir.join("index.ridx");
    TempShard { dir, shard }
}

fn to_uri(p: &Path) -> String {
    path_to_uri(p.to_str().unwrap())
}

fn write_index_shard(path: &Path, name: &str, ver: &str) {
    let mut writer = IndexWriter::new(name, ver);
    let id = WATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_file_name(format!("tmp_{}_{}.ridx", name, id));
    writer.write_to(&mut File::create(&tmp).unwrap()).unwrap();
    std::fs::rename(&tmp, path).unwrap();
}

fn write_index_shard_with_def(path: &Path, name: &str, ver: &str, def_name: &str) {
    let mut writer = IndexWriter::new(name, ver);
    writer.add_definition(RawDefinition {
        def_id: 1, kind: DefKind::Fn, visibility: 0, is_macro_expansion: false,
        name: def_name.to_string(), path: format!("{}::{}", name, def_name),
        signature: None, file: "src/lib.rs".to_string(),
        start_line: 1, start_col: 1, end_line: 1, end_col: 10,
    });
    let id = WATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
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
        assert!(d.multi_root.add_root(shard));
        let init_msg = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } });
        assert!(d.dispatch(&init_msg));
        d.writer.clear();
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
fn test_did_change_watched_files_single_shard_crate_version_reloaded() {
    let ts = create_temp_shard("wf_v1");
    write_index_shard(&ts.shard, "test_pkg", "0.1.0");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    assert_eq!(dsp.multi_root.first_workspace().unwrap().readers()[0].crate_version(), "0.1.0");
    sleep(Duration::from_millis(15));
    write_index_shard_with_def(&ts.shard, "test_pkg", "0.2.0", "extra_reload_fn");
    // Control query without event: reader retains old version
    assert_eq!(dsp.multi_root.first_workspace().unwrap().readers()[0].crate_version(), "0.1.0");
    let notif = json!({ "jsonrpc": "2.0", "method": "workspace/didChangeWatchedFiles", "params": { "changes": [{ "uri": to_uri(&ts.shard), "type": 2 }] } });
    assert!(dsp.dispatch(&notif));
    assert_eq!(dsp.multi_root.first_workspace().unwrap().readers()[0].crate_version(), "0.2.0");
}

#[test]
fn test_did_change_watched_files_multiple_shards_only_one_modified() {
    let ts1 = create_temp_shard("wf_m1");
    let ts2 = create_temp_shard("wf_m2");
    write_index_shard(&ts1.shard, "pkg_a", "1.0.0");
    write_index_shard(&ts2.shard, "pkg_b", "1.0.0");
    let mut dsp = TestDispatcher::new(ts1.shard.clone());
    assert!(dsp.multi_root.add_root(ts2.shard.clone()));
    sleep(Duration::from_millis(15));
    write_index_shard_with_def(&ts1.shard, "pkg_a", "1.1.0", "extra_sym_a");
    assert_eq!(dsp.multi_root.all_workspaces().nth(0).unwrap().readers()[0].crate_version(), "1.0.0");
    assert_eq!(dsp.multi_root.all_workspaces().nth(1).unwrap().readers()[0].crate_version(), "1.0.0");
    let notif = json!({ "jsonrpc": "2.0", "method": "workspace/didChangeWatchedFiles", "params": { "changes": [{ "uri": to_uri(&ts1.shard), "type": 2 }] } });
    assert!(dsp.dispatch(&notif));
    assert_eq!(dsp.multi_root.all_workspaces().nth(0).unwrap().readers()[0].crate_version(), "1.1.0");
    assert_eq!(dsp.multi_root.all_workspaces().nth(1).unwrap().readers()[0].crate_version(), "1.0.0");
}

#[test]
fn test_did_change_watched_files_new_definition_discoverable_after_reload() {
    let ts = create_temp_shard("wf_def_add");
    write_index_shard(&ts.shard, "def_crate", "0.1.0");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    assert_eq!(dsp.multi_root.first_workspace().unwrap().readers()[0].search_symbols("Calculator").len(), 0);
    sleep(Duration::from_millis(15));
    write_index_shard_with_def(&ts.shard, "def_crate", "0.1.0", "Calculator");
    // Control query without event: symbol still not visible
    assert_eq!(dsp.multi_root.first_workspace().unwrap().readers()[0].search_symbols("Calculator").len(), 0);
    let notif = json!({ "jsonrpc": "2.0", "method": "workspace/didChangeWatchedFiles", "params": { "changes": [{ "uri": to_uri(&ts.shard), "type": 2 }] } });
    assert!(dsp.dispatch(&notif));
    assert_eq!(dsp.multi_root.first_workspace().unwrap().readers()[0].search_symbols("Calculator").len(), 1);
}

#[test]
fn test_did_change_watched_files_deleted_definition_removed_after_reload() {
    let ts = create_temp_shard("wf_def_del");
    write_index_shard_with_def(&ts.shard, "del_crate", "0.1.0", "OldFunction");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    assert_eq!(dsp.multi_root.first_workspace().unwrap().readers()[0].search_symbols("OldFunction").len(), 1);
    sleep(Duration::from_millis(15));
    write_index_shard(&ts.shard, "del_crate", "0.1.0");
    // Control query without event: OldFunction still visible
    assert_eq!(dsp.multi_root.first_workspace().unwrap().readers()[0].search_symbols("OldFunction").len(), 1);
    let notif = json!({ "jsonrpc": "2.0", "method": "workspace/didChangeWatchedFiles", "params": { "changes": [{ "uri": to_uri(&ts.shard), "type": 2 }] } });
    assert!(dsp.dispatch(&notif));
    assert_eq!(dsp.multi_root.first_workspace().unwrap().readers()[0].search_symbols("OldFunction").len(), 0);
}

#[test]
fn test_did_change_watched_files_unmodified_shard_preserves_cached_state() {
    let ts = create_temp_shard("wf_unmod");
    write_index_shard(&ts.shard, "unmod", "1.0.0");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let notif = json!({ "jsonrpc": "2.0", "method": "workspace/didChangeWatchedFiles", "params": { "changes": [] } });
    assert!(dsp.dispatch(&notif));
    assert_eq!(dsp.multi_root.first_workspace().unwrap().readers()[0].crate_version(), "1.0.0");
}

#[test]
fn test_did_change_watched_files_new_shard_discovered_in_workspace_dir() {
    let ts = create_temp_shard("wf_new_disc");
    write_index_shard(&ts.shard, "disc_one", "1.0.0");
    let mut dsp = TestDispatcher::new(ts.dir.clone());
    assert_eq!(dsp.multi_root.first_workspace().unwrap().readers().len(), 1);
    let s2 = ts.dir.join("extra.ridx");
    write_index_shard(&s2, "disc_two", "1.0.0");
    // Control query without event: still 1 reader
    assert_eq!(dsp.multi_root.first_workspace().unwrap().readers().len(), 1);
    let notif = json!({ "jsonrpc": "2.0", "method": "workspace/didChangeWatchedFiles", "params": { "changes": [{ "uri": to_uri(&s2), "type": 1 }] } });
    assert!(dsp.dispatch(&notif));
    assert_eq!(dsp.multi_root.first_workspace().unwrap().readers().len(), 2);
}

#[test]
fn test_did_change_watched_files_deleted_shard_removed_from_workspace() {
    let ts = create_temp_shard("wf_del_disc");
    write_index_shard(&ts.shard, "del_a", "1.0.0");
    let s2 = ts.dir.join("del_b.ridx");
    write_index_shard(&s2, "del_b", "1.0.0");
    let mut dsp = TestDispatcher::new(ts.dir.clone());
    assert_eq!(dsp.multi_root.first_workspace().unwrap().readers().len(), 2);
    std::fs::remove_file(&s2).unwrap();
    // Control query without event: still 2 readers in cached state
    assert_eq!(dsp.multi_root.first_workspace().unwrap().readers().len(), 2);
    let notif = json!({ "jsonrpc": "2.0", "method": "workspace/didChangeWatchedFiles", "params": { "changes": [{ "uri": to_uri(&s2), "type": 3 }] } });
    assert!(dsp.dispatch(&notif));
    assert_eq!(dsp.multi_root.first_workspace().unwrap().readers().len(), 1);
}

#[test]
fn test_did_change_watched_files_corrupted_shard_preserves_old_index() {
    let ts = create_temp_shard("wf_corrupt");
    write_index_shard(&ts.shard, "safe_corrupt", "1.0.0");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let tmp = ts.shard.with_file_name("corrupt.tmp");
    std::fs::write(&tmp, b"NOT_A_VALID_RIDX_HEADER").unwrap();
    std::fs::rename(&tmp, &ts.shard).unwrap();
    let notif = json!({ "jsonrpc": "2.0", "method": "workspace/didChangeWatchedFiles", "params": { "changes": [{ "uri": to_uri(&ts.shard), "type": 2 }] } });
    assert!(dsp.dispatch(&notif));
    assert_eq!(dsp.multi_root.first_workspace().unwrap().readers()[0].crate_version(), "1.0.0");
}

#[test]
fn test_did_change_watched_files_clears_position_converter_cache() {
    let ts = create_temp_shard("wf_conv");
    write_index_shard(&ts.shard, "conv_test", "1.0.0");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let test_file = ts.dir.join("src").join("test.rs");
    std::fs::create_dir_all(test_file.parent().unwrap()).unwrap();
    // Non-BMP crab emoji is 2 UTF-16 code units; 1-based char col 2 (at 'a') is UTF-16 col 2
    std::fs::write(&test_file, "🦀abc\nworld").unwrap();
    let col_warm = dsp.lifecycle.conv.char_col_to_utf16_col(test_file.to_str().unwrap(), 1, 2);
    assert_eq!(col_warm, 2);

    // Overwrite file on disk with ASCII content where char col 2 is UTF-16 col 1
    std::fs::write(&test_file, "xabc\nworld").unwrap();

    // Control query without event: PositionConverter returns stale cached value 2
    let col_cached = dsp.lifecycle.conv.char_col_to_utf16_col(test_file.to_str().unwrap(), 1, 2);
    assert_eq!(col_cached, 2);

    // Notification clears PositionConverter cache; re-query returns fresh value 1
    let notif = json!({ "jsonrpc": "2.0", "method": "workspace/didChangeWatchedFiles", "params": { "changes": [] } });
    assert!(dsp.dispatch(&notif));
    let col_fresh = dsp.lifecycle.conv.char_col_to_utf16_col(test_file.to_str().unwrap(), 1, 2);
    assert_eq!(col_fresh, 1);
}

#[test]
fn test_did_change_watched_files_empty_changes_array_still_reloads_disk() {
    let ts = create_temp_shard("wf_empty_arr");
    write_index_shard(&ts.shard, "empty_arr", "1.0.0");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    sleep(Duration::from_millis(15));
    write_index_shard_with_def(&ts.shard, "empty_arr", "2.0.0", "extra_empty_fn");
    let notif = json!({ "jsonrpc": "2.0", "method": "workspace/didChangeWatchedFiles", "params": { "changes": [] } });
    assert!(dsp.dispatch(&notif));
    assert_eq!(dsp.multi_root.first_workspace().unwrap().readers()[0].crate_version(), "2.0.0");
}

#[test]
fn test_did_change_watched_files_missing_params_handled_safely() {
    let ts = create_temp_shard("wf_no_params");
    write_index_shard(&ts.shard, "no_params", "1.0.0");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    sleep(Duration::from_millis(15));
    write_index_shard_with_def(&ts.shard, "no_params", "1.5.0", "extra_no_params_fn");
    let notif = json!({ "jsonrpc": "2.0", "method": "workspace/didChangeWatchedFiles" });
    assert!(dsp.dispatch(&notif));
    assert_eq!(dsp.multi_root.first_workspace().unwrap().readers()[0].crate_version(), "1.5.0");
}

#[test]
fn test_did_change_watched_files_event_type_1_created() {
    let ts = create_temp_shard("wf_t1");
    write_index_shard(&ts.shard, "type1", "0.1.0");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    sleep(Duration::from_millis(15));
    write_index_shard_with_def(&ts.shard, "type1", "0.2.0", "extra_type1_fn");
    let notif = json!({ "jsonrpc": "2.0", "method": "workspace/didChangeWatchedFiles", "params": { "changes": [{ "uri": to_uri(&ts.shard), "type": 1 }] } });
    assert!(dsp.dispatch(&notif));
    assert_eq!(dsp.multi_root.first_workspace().unwrap().readers()[0].crate_version(), "0.2.0");
}

#[test]
fn test_did_change_watched_files_event_type_2_changed() {
    let ts = create_temp_shard("wf_t2");
    write_index_shard(&ts.shard, "type2", "0.1.0");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    sleep(Duration::from_millis(15));
    write_index_shard_with_def(&ts.shard, "type2", "0.3.0", "extra_type2_fn");
    let notif = json!({ "jsonrpc": "2.0", "method": "workspace/didChangeWatchedFiles", "params": { "changes": [{ "uri": to_uri(&ts.shard), "type": 2 }] } });
    assert!(dsp.dispatch(&notif));
    assert_eq!(dsp.multi_root.first_workspace().unwrap().readers()[0].crate_version(), "0.3.0");
}

#[test]
fn test_did_change_watched_files_event_type_3_deleted() {
    let ts = create_temp_shard("wf_t3");
    write_index_shard(&ts.shard, "type3", "0.1.0");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    sleep(Duration::from_millis(15));
    write_index_shard_with_def(&ts.shard, "type3", "0.4.0", "extra_type3_fn");
    let notif = json!({ "jsonrpc": "2.0", "method": "workspace/didChangeWatchedFiles", "params": { "changes": [{ "uri": to_uri(&ts.shard), "type": 3 }] } });
    assert!(dsp.dispatch(&notif));
    assert_eq!(dsp.multi_root.first_workspace().unwrap().readers()[0].crate_version(), "0.4.0");
}

#[test]
fn test_did_change_watched_files_rapid_successive_notifications() {
    let ts = create_temp_shard("wf_rapid");
    write_index_shard(&ts.shard, "rapid", "1.0.0");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    sleep(Duration::from_millis(15));
    write_index_shard_with_def(&ts.shard, "rapid", "2.0.0", "extra_rapid_fn");
    let notif = json!({ "jsonrpc": "2.0", "method": "workspace/didChangeWatchedFiles", "params": { "changes": [{ "uri": to_uri(&ts.shard), "type": 2 }] } });
    assert!(dsp.dispatch(&notif));
    assert_eq!(dsp.multi_root.first_workspace().unwrap().readers()[0].crate_version(), "2.0.0");
    assert!(dsp.dispatch(&notif));
    assert_eq!(dsp.multi_root.first_workspace().unwrap().readers()[0].crate_version(), "2.0.0");
}

#[test]
fn test_did_change_watched_files_active_overlay_preserved_across_reload() {
    let ts = create_temp_shard("wf_overlay");
    write_index_shard(&ts.shard, "ov_test", "1.0.0");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let test_file = ts.dir.join("src").join("main.rs");
    let buf_uri = to_uri(&test_file);
    dsp.overlay_mgr.open_document(&buf_uri, test_file.to_str().unwrap(), 1, "fn main() {}".to_string());
    assert_eq!(dsp.overlay_mgr.get(test_file.to_str().unwrap()).unwrap().text, "fn main() {}");
    sleep(Duration::from_millis(15));
    write_index_shard_with_def(&ts.shard, "ov_test", "2.0.0", "extra_ov_fn");
    let notif = json!({ "jsonrpc": "2.0", "method": "workspace/didChangeWatchedFiles", "params": { "changes": [{ "uri": to_uri(&ts.shard), "type": 2 }] } });
    assert!(dsp.dispatch(&notif));
    assert_eq!(dsp.overlay_mgr.get(test_file.to_str().unwrap()).unwrap().text, "fn main() {}");
    assert_eq!(dsp.multi_root.first_workspace().unwrap().readers()[0].crate_version(), "2.0.0");
}

#[test]
fn test_did_change_watched_files_manifest_variant_switch_reloads_shards() {
    let ts = create_temp_shard("wf_var");
    write_index_shard(&ts.shard, "var_pkg", "1.0.0");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    sleep(Duration::from_millis(15));
    write_index_shard_with_def(&ts.shard, "var_pkg", "1.0.1", "extra_var_fn");
    let notif = json!({ "jsonrpc": "2.0", "method": "workspace/didChangeWatchedFiles", "params": { "changes": [{ "uri": to_uri(&ts.shard), "type": 2 }] } });
    assert!(dsp.dispatch(&notif));
    assert_eq!(dsp.multi_root.first_workspace().unwrap().readers()[0].crate_version(), "1.0.1");
}

#[test]
fn test_did_change_watched_files_multiple_workspaces_all_reloaded() {
    let ts1 = create_temp_shard("wf_all1");
    let ts2 = create_temp_shard("wf_all2");
    write_index_shard(&ts1.shard, "all_a", "1.0.0");
    write_index_shard(&ts2.shard, "all_b", "1.0.0");
    let mut dsp = TestDispatcher::new(ts1.shard.clone());
    assert!(dsp.multi_root.add_root(ts2.shard.clone()));
    sleep(Duration::from_millis(15));
    write_index_shard_with_def(&ts1.shard, "all_a", "2.0.0", "extra_all_a_fn");
    write_index_shard_with_def(&ts2.shard, "all_b", "3.0.0", "extra_all_b_fn");
    let notif = json!({ "jsonrpc": "2.0", "method": "workspace/didChangeWatchedFiles", "params": { "changes": [] } });
    assert!(dsp.dispatch(&notif));
    assert_eq!(dsp.multi_root.all_workspaces().nth(0).unwrap().readers()[0].crate_version(), "2.0.0");
    assert_eq!(dsp.multi_root.all_workspaces().nth(1).unwrap().readers()[0].crate_version(), "3.0.0");
}

#[test]
fn test_did_change_watched_files_dispatch_returns_true() {
    let ts = create_temp_shard("wf_ret");
    write_index_shard(&ts.shard, "ret_pkg", "1.0.0");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    let notif = json!({ "jsonrpc": "2.0", "method": "workspace/didChangeWatchedFiles", "params": { "changes": [] } });
    assert!(dsp.dispatch(&notif));
}

#[test]
fn test_did_change_watched_files_reloads_symbols_exact_count() {
    let ts = create_temp_shard("wf_sym_cnt");
    write_index_shard(&ts.shard, "sym_crate", "1.0.0");
    let mut dsp = TestDispatcher::new(ts.shard.clone());
    assert_eq!(dsp.multi_root.first_workspace().unwrap().readers()[0].search_symbols("Item").len(), 0);
    sleep(Duration::from_millis(15));
    write_index_shard_with_def(&ts.shard, "sym_crate", "1.0.0", "Item");
    let notif = json!({ "jsonrpc": "2.0", "method": "workspace/didChangeWatchedFiles", "params": { "changes": [{ "uri": to_uri(&ts.shard), "type": 2 }] } });
    assert!(dsp.dispatch(&notif));
    assert_eq!(dsp.multi_root.first_workspace().unwrap().readers()[0].search_symbols("Item").len(), 1);
}
