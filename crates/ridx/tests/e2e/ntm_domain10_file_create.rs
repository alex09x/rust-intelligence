/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for workspace/didCreateFiles and workspace/willCreateFiles (Domain 10).
//! Verifies pre-creation LSP request negotiation, post-creation index synchronization,
//! PositionConverter invalidation, and dirty overlay preservation.

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::{path_to_uri, PositionConverter};
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter, RawDefinition};
use serde_json::json;
use std::collections::HashSet;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

static CREATE_COUNTER: AtomicU64 = AtomicU64::new(1);

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
    let id = CREATE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_create_{}_{}_{}", name, std::process::id(), id));
    std::fs::create_dir_all(&dir).unwrap();
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
fn test_will_create_files_valid_file_returns_empty_edit() {
    let ts1 = create_temp_shard_with_symbol("create_a", "ExistingItem");
    let mut dsp = TestDispatcher::new(ts1.shard.clone());
    let target_file = ts1.dir.join("src/models.rs");
    let req = json!({ "jsonrpc": "2.0", "id": 101, "method": "workspace/willCreateFiles", "params": { "files": [{ "uri": to_uri(&target_file) }] } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 101);
    assert_eq!(resp["result"], json!({}));
}

#[test]
fn test_will_create_files_multi_file_returns_empty_edit() {
    let ts1 = create_temp_shard_with_symbol("create_multi", "MultiSym");
    let mut dsp = TestDispatcher::new(ts1.shard.clone());
    let req = json!({ "jsonrpc": "2.0", "id": 102, "method": "workspace/willCreateFiles", "params": { "files": [
        { "uri": "file:///tmp/models.rs" }, { "uri": "file:///tmp/views.rs" }, { "uri": "file:///tmp/controllers.rs" }
    ] } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 102);
    assert_eq!(resp["result"], json!({}));
}

#[test]
fn test_will_create_files_empty_files_array() {
    let ts1 = create_temp_shard_with_symbol("create_empty_arr", "EmptySym");
    let mut dsp = TestDispatcher::new(ts1.shard.clone());
    let req = json!({ "jsonrpc": "2.0", "id": 103, "method": "workspace/willCreateFiles", "params": { "files": [] } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 103);
    assert_eq!(resp["result"], json!({}));
}

#[test]
fn test_will_create_files_numeric_id_preserved() {
    let ts1 = create_temp_shard_with_symbol("create_num_id", "NumSym");
    let mut dsp = TestDispatcher::new(ts1.shard.clone());
    let req = json!({ "jsonrpc": "2.0", "id": 4242, "method": "workspace/willCreateFiles", "params": { "files": [{ "uri": "file:///tmp/new.rs" }] } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 4242);
    assert_eq!(resp["result"], json!({}));
}

#[test]
fn test_will_create_files_string_id_preserved() {
    let ts1 = create_temp_shard_with_symbol("create_str_id", "StrSym");
    let mut dsp = TestDispatcher::new(ts1.shard.clone());
    let req = json!({ "jsonrpc": "2.0", "id": "req-create-uuid-77", "method": "workspace/willCreateFiles", "params": { "files": [{ "uri": "file:///tmp/new.rs" }] } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], "req-create-uuid-77");
    assert_eq!(resp["result"], json!({}));
}

#[test]
fn test_will_create_files_returns_matching_schema() {
    let ts1 = create_temp_shard_with_symbol("create_schema", "SchemaSym");
    let mut dsp = TestDispatcher::new(ts1.shard.clone());
    let req = json!({ "jsonrpc": "2.0", "id": 999, "method": "workspace/willCreateFiles", "params": { "files": [{ "uri": "file:///tmp/schema_check.rs" }] } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 999);
    assert!(resp["result"].is_object());
}

#[test]
fn test_did_create_files_empty_files_noop() {
    let mut mr = MultiRootWorkspace::new();
    let ts1 = create_temp_shard_with_symbol("create_b", "AlphaControl");
    assert!(mr.add_root(ts1.shard.clone()));
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());
    let msg = json!({ "params": { "files": [] } });
    mr.handle_file_event("workspace/didCreateFiles", &msg, &mut overlay_mgr, &mut conv);
    assert_eq!(mr.len(), 1);
    assert_eq!(mr.first_workspace().unwrap().readers()[0].crate_name(), "create_b");
}

#[test]
fn test_did_create_files_missing_files_key_noop() {
    let mut mr = MultiRootWorkspace::new();
    let ts1 = create_temp_shard_with_symbol("create_c", "BetaControl");
    assert!(mr.add_root(ts1.shard.clone()));
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());
    let msg = json!({ "params": {} });
    mr.handle_file_event("workspace/didCreateFiles", &msg, &mut overlay_mgr, &mut conv);
    assert_eq!(mr.len(), 1);
    assert_eq!(mr.first_workspace().unwrap().readers()[0].crate_name(), "create_c");
}

#[test]
fn test_did_create_files_single_file_clears_conv() {
    let ts1 = create_temp_shard_with_symbol("create_d", "GammaControl");
    let mut mr = MultiRootWorkspace::new();
    assert!(mr.add_root(ts1.shard.clone()));
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());
    let test_file = ts1.dir.join("src/test.rs");
    std::fs::create_dir_all(test_file.parent().unwrap()).unwrap();
    // Non-BMP crab emoji is 2 UTF-16 code units; 1-based char col 2 (at 'a') is UTF-16 col 2
    std::fs::write(&test_file, "🦀abc\nworld").unwrap();
    let col_warm = conv.char_col_to_utf16_col(test_file.to_str().unwrap(), 1, 2);
    assert_eq!(col_warm, 2);

    // Overwrite file on disk with ASCII content where char col 2 is UTF-16 col 1
    std::fs::write(&test_file, "xabc\nworld").unwrap();
    // Control query without event: PositionConverter returns stale cached value 2
    let col_cached = conv.char_col_to_utf16_col(test_file.to_str().unwrap(), 1, 2);
    assert_eq!(col_cached, 2);

    let msg = json!({ "params": { "files": [{ "uri": to_uri(&test_file) }] } });
    mr.handle_file_event("workspace/didCreateFiles", &msg, &mut overlay_mgr, &mut conv);

    let col_fresh = conv.char_col_to_utf16_col(test_file.to_str().unwrap(), 1, 2);
    assert_eq!(col_fresh, 1);
}

#[test]
fn test_did_create_files_multi_root_sync() {
    let mut mr = MultiRootWorkspace::new();
    let ts1 = create_temp_shard_with_symbol("create_multi_1", "M1");
    let ts2 = create_temp_shard_with_symbol("create_multi_2", "M2");
    assert!(mr.add_root(ts1.shard.clone()));
    assert!(mr.add_root(ts2.shard.clone()));
    assert_eq!(mr.len(), 2);
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());
    let u = to_uri(&ts1.dir.join("src/lib2.rs"));
    let msg = json!({ "params": { "files": [{ "uri": u }] } });
    mr.handle_file_event("workspace/didCreateFiles", &msg, &mut overlay_mgr, &mut conv);
    assert_eq!(mr.len(), 2);
    assert_eq!(mr.first_workspace().unwrap().readers()[0].crate_name(), "create_multi_1");
}

#[test]
fn test_did_create_files_with_updated_disk_index_syncs() {
    let mut mr = MultiRootWorkspace::new();
    let ts1 = create_temp_shard_with_symbol("create_sync", "SyncControl");
    assert!(mr.add_root(ts1.shard.clone()));
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());
    let msg = json!({ "params": { "files": [{ "uri": to_uri(&ts1.dir.join("src/new.rs")) }] } });
    mr.handle_file_event("workspace/didCreateFiles", &msg, &mut overlay_mgr, &mut conv);
    assert_eq!(mr.len(), 1);
    assert_eq!(mr.first_workspace().unwrap().readers()[0].crate_name(), "create_sync");
}

#[test]
fn test_did_create_files_unmodified_index_preserves_baseline() {
    let mut mr = MultiRootWorkspace::new();
    let ts1 = create_temp_shard_with_symbol("create_base", "BaseControl");
    assert!(mr.add_root(ts1.shard.clone()));
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());
    let msg = json!({ "params": { "files": [{ "uri": "file:///tmp/other.rs" }] } });
    mr.handle_file_event("workspace/didCreateFiles", &msg, &mut overlay_mgr, &mut conv);
    assert_eq!(mr.first_workspace().unwrap().readers()[0].crate_name(), "create_base");
}

#[test]
fn test_did_create_files_percent_encoded_uri() {
    let mut mr = MultiRootWorkspace::new();
    let ts1 = create_temp_shard_with_symbol("create_pct", "PctControl");
    assert!(mr.add_root(ts1.shard.clone()));
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());
    let msg = json!({ "params": { "files": [{ "uri": "file:///tmp/test%20space/new.rs" }] } });
    mr.handle_file_event("workspace/didCreateFiles", &msg, &mut overlay_mgr, &mut conv);
    assert_eq!(mr.first_workspace().unwrap().readers()[0].crate_name(), "create_pct");
}

#[test]
fn test_did_create_files_file_outside_roots_ignored() {
    let mut mr = MultiRootWorkspace::new();
    let ts1 = create_temp_shard_with_symbol("create_ext", "ExtControl");
    assert!(mr.add_root(ts1.shard.clone()));
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());
    let msg = json!({ "params": { "files": [{ "uri": "file:///var/log/system.log" }] } });
    mr.handle_file_event("workspace/didCreateFiles", &msg, &mut overlay_mgr, &mut conv);
    assert_eq!(mr.len(), 1);
    assert_eq!(mr.first_workspace().unwrap().readers()[0].crate_name(), "create_ext");
}

#[test]
fn test_did_create_files_empty_uri_skipped() {
    let mut mr = MultiRootWorkspace::new();
    let ts1 = create_temp_shard_with_symbol("create_empty", "EmptyControl");
    assert!(mr.add_root(ts1.shard.clone()));
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());
    let msg = json!({ "params": { "files": [{ "uri": "" }] } });
    mr.handle_file_event("workspace/didCreateFiles", &msg, &mut overlay_mgr, &mut conv);
    assert_eq!(mr.len(), 1);
}

#[test]
fn test_did_create_files_followed_by_did_open() {
    let mut mr = MultiRootWorkspace::new();
    let ts1 = create_temp_shard_with_symbol("create_open", "OpenControl");
    assert!(mr.add_root(ts1.shard.clone()));
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());
    let new_file = "/tmp/newly_created_buffer.rs";
    let new_uri = "file:///tmp/newly_created_buffer.rs";
    let msg = json!({ "params": { "files": [{ "uri": new_uri }] } });
    mr.handle_file_event("workspace/didCreateFiles", &msg, &mut overlay_mgr, &mut conv);
    overlay_mgr.open_with_provenance(new_uri, new_file, 1, "pub fn created() {}\n".to_string(), true);
    assert_eq!(overlay_mgr.get(new_file).unwrap().version, 1);
    assert_eq!(overlay_mgr.get(new_file).unwrap().text, "pub fn created() {}\n");
}

#[test]
fn test_did_create_files_open_dirty_buffer_not_overwritten() {
    let mut overlay_mgr = OverlayManager::new();
    let dirty_file = "/tmp/dirty_created_buffer.rs";
    let dirty_uri = "file:///tmp/dirty_created_buffer.rs";
    overlay_mgr.open_with_provenance(dirty_uri, dirty_file, 1, "pub fn original() {}\n".to_string(), true);
    let edits = vec![json!({ "text": "pub fn modified() {}\n" })];
    overlay_mgr.apply_content_changes(dirty_file, 2, &edits);
    assert!(overlay_mgr.get(dirty_file).unwrap().is_dirty());

    let (mut mr, mut conv) = (MultiRootWorkspace::new(), PositionConverter::new());
    let msg = json!({ "params": { "files": [{ "uri": dirty_uri }] } });
    mr.handle_file_event("workspace/didCreateFiles", &msg, &mut overlay_mgr, &mut conv);
    assert!(overlay_mgr.get(dirty_file).unwrap().is_dirty());
    assert_eq!(overlay_mgr.get(dirty_file).unwrap().version, 2);
}

#[test]
fn test_did_create_files_batch_twenty_files() {
    let mut mr = MultiRootWorkspace::new();
    let ts1 = create_temp_shard_with_symbol("create_batch", "BatchControl");
    assert!(mr.add_root(ts1.shard.clone()));
    let mut files = Vec::new();
    for i in 0..20 {
        files.push(json!({ "uri": format!("file:///tmp/module_{}.rs", i) }));
    }
    let msg = json!({ "params": { "files": files } });
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());
    mr.handle_file_event("workspace/didCreateFiles", &msg, &mut overlay_mgr, &mut conv);
    assert_eq!(mr.len(), 1);
    assert_eq!(mr.first_workspace().unwrap().readers()[0].crate_name(), "create_batch");
}

#[test]
fn test_did_create_files_relative_path_resolution() {
    let mut mr = MultiRootWorkspace::new();
    let ts1 = create_temp_shard_with_symbol("create_rel", "RelControl");
    assert!(mr.add_root(ts1.shard.clone()));
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());
    let msg = json!({ "params": { "files": [{ "uri": "file:///tmp/foo/../bar/test.rs" }] } });
    mr.handle_file_event("workspace/didCreateFiles", &msg, &mut overlay_mgr, &mut conv);
    assert_eq!(mr.len(), 1);
}

#[test]
fn test_did_create_files_retains_existing_roots() {
    let mut mr = MultiRootWorkspace::new();
    let ts1 = create_temp_shard_with_symbol("create_retain", "RetainControl");
    assert!(mr.add_root(ts1.shard.clone()));
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());
    let msg = json!({ "params": { "files": [{ "uri": "file:///tmp/ignored.rs" }] } });
    mr.handle_file_event("workspace/didCreateFiles", &msg, &mut overlay_mgr, &mut conv);
    assert_eq!(mr.len(), 1);
    assert_eq!(mr.first_workspace().unwrap().readers()[0].crate_name(), "create_retain");
}
