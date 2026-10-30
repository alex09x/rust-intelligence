/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for workspace/didDeleteFiles and workspace/willDeleteFiles (Domain 10).
//! Verifies pre-deletion LSP negotiation, overlay eviction, dirty buffer discard,
//! PositionConverter invalidation, multi-root isolation, and idempotent deletion.

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::{path_to_uri, uri_to_path, PositionConverter};
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter, RawDefinition};
use serde_json::json;
use std::collections::HashSet;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

static DELETE_COUNTER: AtomicU64 = AtomicU64::new(1);

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
    let id = DELETE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_delete_{}_{}_{}", name, std::process::id(), id));
    std::fs::create_dir_all(&dir).unwrap();
    let shard = dir.join("index.ridx");
    let mut writer = IndexWriter::new(name, "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: sym_name.to_string(),
        path: format!("{}::{}", name, sym_name),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 10,
        start_col: 5,
        end_line: 10,
        end_col: 15,
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
        dispatch_lsp_message(msg.clone(), &mut self.writer, &mut self.lifecycle, &mut self.multi_root, &mut self.overlay_mgr, &self.cancelled, None).unwrap()
    }

    fn parse_response(&mut self) -> serde_json::Value {
        let text = String::from_utf8(self.writer.clone()).unwrap();
        self.writer.clear();
        let body = text.split("\r\n\r\n").nth(1).unwrap();
        serde_json::from_str(body).unwrap()
    }
}

#[test]
fn test_will_delete_files_valid_file_returns_empty_edit() {
    let ts1 = create_temp_shard_with_symbol("del_a", "SymA");
    let mut dsp = TestDispatcher::new(ts1.shard.clone());
    let req = json!({ "jsonrpc": "2.0", "id": 301, "method": "workspace/willDeleteFiles", "params": { "files": [{ "uri": "file:///tmp/dead_file.rs" }] } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 301);
    assert_eq!(resp["result"], json!({}));
}

#[test]
fn test_will_delete_files_multi_file_returns_empty_edit() {
    let ts1 = create_temp_shard_with_symbol("del_multi", "MultiSym");
    let mut dsp = TestDispatcher::new(ts1.shard.clone());
    let req = json!({ "jsonrpc": "2.0", "id": 302, "method": "workspace/willDeleteFiles", "params": { "files": [
        { "uri": "file:///tmp/del_a.rs" }, { "uri": "file:///tmp/del_b.rs" }
    ] } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 302);
    assert_eq!(resp["result"], json!({}));
}

#[test]
fn test_will_delete_files_empty_files_array() {
    let ts1 = create_temp_shard_with_symbol("del_empty", "EmptySym");
    let mut dsp = TestDispatcher::new(ts1.shard.clone());
    let req = json!({ "jsonrpc": "2.0", "id": 303, "method": "workspace/willDeleteFiles", "params": { "files": [] } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 303);
    assert_eq!(resp["result"], json!({}));
}

#[test]
fn test_will_delete_files_numeric_id_preserved() {
    let ts1 = create_temp_shard_with_symbol("del_num", "NumSym");
    let mut dsp = TestDispatcher::new(ts1.shard.clone());
    let req = json!({ "jsonrpc": "2.0", "id": 8888, "method": "workspace/willDeleteFiles", "params": { "files": [] } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 8888);
    assert_eq!(resp["result"], json!({}));
}

#[test]
fn test_will_delete_files_string_id_preserved() {
    let ts1 = create_temp_shard_with_symbol("del_str", "StrSym");
    let mut dsp = TestDispatcher::new(ts1.shard.clone());
    let req = json!({ "jsonrpc": "2.0", "id": "del-uuid-99", "method": "workspace/willDeleteFiles", "params": { "files": [] } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], "del-uuid-99");
    assert_eq!(resp["result"], json!({}));
}

#[test]
fn test_will_delete_files_lifecycle_sequence_with_did_delete() {
    let ts1 = create_temp_shard_with_symbol("del_seq", "SeqSym");
    let mut dsp = TestDispatcher::new(ts1.shard.clone());
    let will_req = json!({ "jsonrpc": "2.0", "id": 9999, "method": "workspace/willDeleteFiles", "params": { "files": [{ "uri": "file:///tmp/seq_del.rs" }] } });
    assert!(dsp.dispatch(&will_req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 9999);
    assert_eq!(resp["result"], json!({}));

    dsp.overlay_mgr.open_with_provenance("file:///tmp/seq_del.rs", "/tmp/seq_del.rs", 1, "fn to_del() {}\n".to_string(), true);
    let did_msg = json!({ "params": { "files": [{ "uri": "file:///tmp/seq_del.rs" }] } });
    dsp.multi_root.handle_file_event("workspace/didDeleteFiles", &did_msg, &mut dsp.overlay_mgr, &mut dsp.lifecycle.conv);
    assert!(dsp.overlay_mgr.get("/tmp/seq_del.rs").is_none());
}

#[test]
fn test_did_delete_files_active_overlay_evicted() {
    let mut overlay_mgr = OverlayManager::new();
    let (target_file, target_uri) = ("/tmp/del_active.rs", "file:///tmp/del_active.rs");
    let (ctrl_file, ctrl_uri) = ("/tmp/del_ctrl.rs", "file:///tmp/del_ctrl.rs");
    overlay_mgr.open_with_provenance(target_uri, target_file, 1, "fn active() {}\n".to_string(), true);
    overlay_mgr.open_with_provenance(ctrl_uri, ctrl_file, 1, "fn keep() {}\n".to_string(), true);

    let (mut mr, mut conv) = (MultiRootWorkspace::new(), PositionConverter::new());
    let msg = json!({ "params": { "files": [{ "uri": target_uri }] } });
    mr.handle_file_event("workspace/didDeleteFiles", &msg, &mut overlay_mgr, &mut conv);

    assert!(overlay_mgr.get(target_file).is_none());
    assert_eq!(overlay_mgr.get(ctrl_file).unwrap().text, "fn keep() {}\n");
}

#[test]
fn test_did_delete_files_dirty_overlay_discarded() {
    let mut overlay_mgr = OverlayManager::new();
    let (file, uri) = ("/tmp/del_dirty.rs", "file:///tmp/del_dirty.rs");
    overlay_mgr.open_with_provenance(uri, file, 1, "fn pristine() {}\n".to_string(), true);
    let edits = vec![json!({ "text": "fn modified() {}\n" })];
    overlay_mgr.apply_content_changes(file, 2, &edits);
    assert!(overlay_mgr.get(file).unwrap().is_dirty());

    let (mut mr, mut conv) = (MultiRootWorkspace::new(), PositionConverter::new());
    let msg = json!({ "params": { "files": [{ "uri": uri }] } });
    mr.handle_file_event("workspace/didDeleteFiles", &msg, &mut overlay_mgr, &mut conv);
    assert!(overlay_mgr.get(file).is_none());
}

#[test]
fn test_did_delete_files_unopened_file_noop() {
    let mut overlay_mgr = OverlayManager::new();
    let (mut mr, mut conv) = (MultiRootWorkspace::new(), PositionConverter::new());
    let msg = json!({ "params": { "files": [{ "uri": "file:///tmp/not_opened_del.rs" }] } });
    mr.handle_file_event("workspace/didDeleteFiles", &msg, &mut overlay_mgr, &mut conv);
    assert!(overlay_mgr.get("/tmp/not_opened_del.rs").is_none());
}

#[test]
fn test_did_delete_files_multi_file_simultaneous() {
    let mut overlay_mgr = OverlayManager::new();
    overlay_mgr.open_with_provenance("file:///tmp/d1.rs", "/tmp/d1.rs", 1, "fn d1() {}\n".to_string(), true);
    overlay_mgr.open_with_provenance("file:///tmp/d2.rs", "/tmp/d2.rs", 1, "fn d2() {}\n".to_string(), true);
    overlay_mgr.open_with_provenance("file:///tmp/d3.rs", "/tmp/d3.rs", 1, "fn d3() {}\n".to_string(), true);

    let (mut mr, mut conv) = (MultiRootWorkspace::new(), PositionConverter::new());
    let msg = json!({ "params": { "files": [
        { "uri": "file:///tmp/d1.rs" }, { "uri": "file:///tmp/d2.rs" }
    ] } });
    mr.handle_file_event("workspace/didDeleteFiles", &msg, &mut overlay_mgr, &mut conv);

    assert!(overlay_mgr.get("/tmp/d1.rs").is_none());
    assert!(overlay_mgr.get("/tmp/d2.rs").is_none());
    assert_eq!(overlay_mgr.get("/tmp/d3.rs").unwrap().text, "fn d3() {}\n");
}

#[test]
fn test_did_delete_files_empty_files_noop() {
    let mut overlay_mgr = OverlayManager::new();
    let control = "/tmp/del_empty_ctrl.rs";
    overlay_mgr.open_with_provenance("file:///tmp/del_empty_ctrl.rs", control, 1, "fn ec() {}\n".to_string(), true);
    let (mut mr, mut conv) = (MultiRootWorkspace::new(), PositionConverter::new());
    let msg = json!({ "params": { "files": [] } });
    mr.handle_file_event("workspace/didDeleteFiles", &msg, &mut overlay_mgr, &mut conv);
    assert_eq!(overlay_mgr.get(control).unwrap().text, "fn ec() {}\n");
}

#[test]
fn test_did_delete_files_missing_files_key_noop() {
    let mut overlay_mgr = OverlayManager::new();
    let control = "/tmp/del_missing_key_control.rs";
    overlay_mgr.open_with_provenance("file:///tmp/del_missing_key_control.rs", control, 1, "fn mk() {}\n".to_string(), true);
    let (mut mr, mut conv) = (MultiRootWorkspace::new(), PositionConverter::new());
    let msg = json!({ "params": {} });
    mr.handle_file_event("workspace/didDeleteFiles", &msg, &mut overlay_mgr, &mut conv);
    assert_eq!(overlay_mgr.get(control).unwrap().text, "fn mk() {}\n");
}

#[test]
fn test_did_delete_files_missing_uri_field_skipped() {
    let mut overlay_mgr = OverlayManager::new();
    let control = "/tmp/del_missing_uri_control.rs";
    overlay_mgr.open_with_provenance("file:///tmp/del_missing_uri_control.rs", control, 1, "fn mu() {}\n".to_string(), true);
    let (mut mr, mut conv) = (MultiRootWorkspace::new(), PositionConverter::new());
    let msg = json!({ "params": { "files": [{ "other_field": 123 }] } });
    mr.handle_file_event("workspace/didDeleteFiles", &msg, &mut overlay_mgr, &mut conv);
    assert_eq!(overlay_mgr.get(control).unwrap().text, "fn mu() {}\n");
}

#[test]
fn test_did_delete_files_clears_position_converter() {
    let ts1 = create_temp_shard_with_symbol("del_conv", "ConvControl");
    let (mut mr, mut overlay_mgr, mut conv) = (MultiRootWorkspace::new(), OverlayManager::new(), PositionConverter::new());
    assert!(mr.add_root(ts1.shard.clone()));

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

    let msg = json!({ "params": { "files": [{ "uri": "file:///tmp/del_conv.rs" }] } });
    mr.handle_file_event("workspace/didDeleteFiles", &msg, &mut overlay_mgr, &mut conv);

    let col_fresh = conv.char_col_to_utf16_col(test_file.to_str().unwrap(), 1, 2);
    assert_eq!(col_fresh, 1);
}

#[test]
fn test_did_delete_files_reopen_after_deletion() {
    let mut overlay_mgr = OverlayManager::new();
    let (file, uri) = ("/tmp/reopen_file.rs", "file:///tmp/reopen_file.rs");
    overlay_mgr.open_with_provenance(uri, file, 1, "fn v1() {}\n".to_string(), true);
    let (mut mr, mut conv) = (MultiRootWorkspace::new(), PositionConverter::new());
    let msg = json!({ "params": { "files": [{ "uri": uri }] } });
    mr.handle_file_event("workspace/didDeleteFiles", &msg, &mut overlay_mgr, &mut conv);
    assert!(overlay_mgr.get(file).is_none());

    overlay_mgr.open_with_provenance(uri, file, 1, "fn v2_fresh() {}\n".to_string(), true);
    assert_eq!(overlay_mgr.get(file).unwrap().text, "fn v2_fresh() {}\n");
    assert_eq!(overlay_mgr.get(file).unwrap().version, 1);
}

#[test]
fn test_did_delete_files_idempotent_multiple_deletions() {
    let mut overlay_mgr = OverlayManager::new();
    let (file, uri) = ("/tmp/idem_file.rs", "file:///tmp/idem_file.rs");
    overlay_mgr.open_with_provenance(uri, file, 1, "fn idem() {}\n".to_string(), true);
    let (mut mr, mut conv) = (MultiRootWorkspace::new(), PositionConverter::new());
    let msg = json!({ "params": { "files": [{ "uri": uri }] } });
    mr.handle_file_event("workspace/didDeleteFiles", &msg, &mut overlay_mgr, &mut conv);
    assert!(overlay_mgr.get(file).is_none());
    mr.handle_file_event("workspace/didDeleteFiles", &msg, &mut overlay_mgr, &mut conv);
    assert!(overlay_mgr.get(file).is_none());
}

#[test]
fn test_did_delete_files_nonexistent_path_noop() {
    let mut overlay_mgr = OverlayManager::new();
    let control = "/tmp/del_nonexist_control.rs";
    overlay_mgr.open_with_provenance("file:///tmp/del_nonexist_control.rs", control, 1, "fn nec() {}\n".to_string(), true);
    let (mut mr, mut conv) = (MultiRootWorkspace::new(), PositionConverter::new());
    let msg = json!({ "params": { "files": [{ "uri": "file:///path/never/heard/of.rs" }] } });
    mr.handle_file_event("workspace/didDeleteFiles", &msg, &mut overlay_mgr, &mut conv);
    assert_eq!(overlay_mgr.get(control).unwrap().text, "fn nec() {}\n");
}

#[test]
fn test_did_delete_files_percent_encoded_uri() {
    let mut overlay_mgr = OverlayManager::new();
    let raw_uri = "file:///tmp/delete%20me.rs";
    let path = uri_to_path(raw_uri);
    overlay_mgr.open_with_provenance(raw_uri, &path, 1, "fn encoded() {}\n".to_string(), true);
    let (mut mr, mut conv) = (MultiRootWorkspace::new(), PositionConverter::new());
    let msg = json!({ "params": { "files": [{ "uri": raw_uri }] } });
    mr.handle_file_event("workspace/didDeleteFiles", &msg, &mut overlay_mgr, &mut conv);
    assert!(overlay_mgr.get(&path).is_none());
}

#[test]
fn test_did_delete_files_multi_root_secondary_workspace() {
    let mut mr = MultiRootWorkspace::new();
    let ts1 = create_temp_shard_with_symbol("del_mr_1", "D1");
    let ts2 = create_temp_shard_with_symbol("del_mr_2", "D2");
    assert!(mr.add_root(ts1.shard.clone()));
    assert!(mr.add_root(ts2.shard.clone()));

    let mut overlay_mgr = OverlayManager::new();
    let sec_file = ts2.dir.join("src/sec_del.rs");
    let u_sec = to_uri(&sec_file);
    overlay_mgr.open_with_provenance(&u_sec, sec_file.to_str().unwrap(), 1, "fn sec_del() {}\n".to_string(), true);

    let mut conv = PositionConverter::new();
    let msg = json!({ "params": { "files": [{ "uri": u_sec }] } });
    mr.handle_file_event("workspace/didDeleteFiles", &msg, &mut overlay_mgr, &mut conv);

    assert!(overlay_mgr.get(sec_file.to_str().unwrap()).is_none());
    assert_eq!(mr.len(), 2);
}

#[test]
fn test_did_delete_files_batch_ten_files_partial_open() {
    let mut overlay_mgr = OverlayManager::new();
    overlay_mgr.open_with_provenance("file:///tmp/part_1.rs", "/tmp/part_1.rs", 1, "fn p1() {}\n".to_string(), true);
    overlay_mgr.open_with_provenance("file:///tmp/part_2.rs", "/tmp/part_2.rs", 1, "fn p2() {}\n".to_string(), true);
    overlay_mgr.open_with_provenance("file:///tmp/part_keep.rs", "/tmp/part_keep.rs", 1, "fn keep() {}\n".to_string(), true);

    let mut files = vec![json!({ "uri": "file:///tmp/part_1.rs" }), json!({ "uri": "file:///tmp/part_2.rs" })];
    for i in 3..11 {
        files.push(json!({ "uri": format!("file:///tmp/unopened_{}.rs", i) }));
    }
    let msg = json!({ "params": { "files": files } });

    let (mut mr, mut conv) = (MultiRootWorkspace::new(), PositionConverter::new());
    mr.handle_file_event("workspace/didDeleteFiles", &msg, &mut overlay_mgr, &mut conv);

    assert!(overlay_mgr.get("/tmp/part_1.rs").is_none());
    assert!(overlay_mgr.get("/tmp/part_2.rs").is_none());
    assert_eq!(overlay_mgr.get("/tmp/part_keep.rs").unwrap().text, "fn keep() {}\n");
}
