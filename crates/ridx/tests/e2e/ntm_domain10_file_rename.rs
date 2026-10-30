/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for workspace/didRenameFiles and workspace/willRenameFiles (Domain 10).
//! Verifies pre-rename LSP negotiation, active overlay rebinding, dirty buffer preservation,
//! PositionConverter invalidation, cyclic and chained renames, and multi-root isolation.

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

static RENAME_COUNTER: AtomicU64 = AtomicU64::new(1);

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
    let id = RENAME_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_rename_{}_{}_{}", name, std::process::id(), id));
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
fn test_will_rename_files_valid_returns_empty_edit() {
    let ts1 = create_temp_shard_with_symbol("ren_a", "SymA");
    let mut dsp = TestDispatcher::new(ts1.shard.clone());
    let req = json!({ "jsonrpc": "2.0", "id": 201, "method": "workspace/willRenameFiles", "params": { "files": [{ "oldUri": "file:///tmp/old_mod.rs", "newUri": "file:///tmp/new_mod.rs" }] } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 201);
    assert_eq!(resp["result"], json!({}));
}

#[test]
fn test_will_rename_files_multi_file_returns_empty_edit() {
    let ts1 = create_temp_shard_with_symbol("ren_multi", "MultiSym");
    let mut dsp = TestDispatcher::new(ts1.shard.clone());
    let req = json!({ "jsonrpc": "2.0", "id": 202, "method": "workspace/willRenameFiles", "params": { "files": [
        { "oldUri": "file:///tmp/a1.rs", "newUri": "file:///tmp/a2.rs" }, { "oldUri": "file:///tmp/b1.rs", "newUri": "file:///tmp/b2.rs" }
    ] } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 202);
    assert_eq!(resp["result"], json!({}));
}

#[test]
fn test_will_rename_files_empty_files_array() {
    let ts1 = create_temp_shard_with_symbol("ren_empty", "EmptySym");
    let mut dsp = TestDispatcher::new(ts1.shard.clone());
    let req = json!({ "jsonrpc": "2.0", "id": 203, "method": "workspace/willRenameFiles", "params": { "files": [] } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 203);
    assert_eq!(resp["result"], json!({}));
}

#[test]
fn test_will_rename_files_numeric_id_preserved() {
    let ts1 = create_temp_shard_with_symbol("ren_num", "NumSym");
    let mut dsp = TestDispatcher::new(ts1.shard.clone());
    let req = json!({ "jsonrpc": "2.0", "id": 777, "method": "workspace/willRenameFiles", "params": { "files": [] } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 777);
    assert_eq!(resp["result"], json!({}));
}

#[test]
fn test_will_rename_files_string_id_preserved() {
    let ts1 = create_temp_shard_with_symbol("ren_str", "StrSym");
    let mut dsp = TestDispatcher::new(ts1.shard.clone());
    let req = json!({ "jsonrpc": "2.0", "id": "rename-uuid-88", "method": "workspace/willRenameFiles", "params": { "files": [] } });
    assert!(dsp.dispatch(&req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], "rename-uuid-88");
    assert_eq!(resp["result"], json!({}));
}

#[test]
fn test_will_rename_files_lifecycle_sequence_with_did_rename() {
    let ts1 = create_temp_shard_with_symbol("ren_seq", "SeqSym");
    let mut dsp = TestDispatcher::new(ts1.shard.clone());
    let will_req = json!({
        "jsonrpc": "2.0", "id": 555, "method": "workspace/willRenameFiles",
        "params": { "files": [{ "oldUri": "file:///tmp/seq_old.rs", "newUri": "file:///tmp/seq_new.rs" }] }
    });
    assert!(dsp.dispatch(&will_req));
    let resp = dsp.parse_response();
    assert_eq!(resp["id"], 555);
    assert_eq!(resp["result"], json!({}));
    dsp.overlay_mgr.open_with_provenance("file:///tmp/seq_old.rs", "/tmp/seq_old.rs", 1, "fn seq() {}\n".to_string(), true);
    let did_msg = json!({ "params": { "files": [{ "oldUri": "file:///tmp/seq_old.rs", "newUri": "file:///tmp/seq_new.rs" }] } });
    dsp.multi_root.handle_file_event("workspace/didRenameFiles", &did_msg, &mut dsp.overlay_mgr, &mut dsp.lifecycle.conv);
    assert!(dsp.overlay_mgr.get("/tmp/seq_old.rs").is_none());
    assert_eq!(dsp.overlay_mgr.get("/tmp/seq_new.rs").unwrap().text, "fn seq() {}\n");
}

#[test]
fn test_did_rename_files_active_overlay_rebound() {
    let mut overlay_mgr = OverlayManager::new();
    let (old_file, old_uri) = ("/tmp/rename_old_doc.rs", "file:///tmp/rename_old_doc.rs");
    let (new_file, new_uri) = ("/tmp/rename_new_doc.rs", "file:///tmp/rename_new_doc.rs");
    let (ctrl_file, ctrl_uri) = ("/tmp/rename_ctrl_doc.rs", "file:///tmp/rename_ctrl_doc.rs");
    overlay_mgr.open_with_provenance(old_uri, old_file, 1, "pub fn foo() {}\n".to_string(), true);
    overlay_mgr.open_with_provenance(ctrl_uri, ctrl_file, 1, "pub fn control() {}\n".to_string(), true);
    let (mut mr, mut conv) = (MultiRootWorkspace::new(), PositionConverter::new());
    let msg = json!({ "params": { "files": [{ "oldUri": old_uri, "newUri": new_uri }] } });
    mr.handle_file_event("workspace/didRenameFiles", &msg, &mut overlay_mgr, &mut conv);
    assert!(overlay_mgr.get(old_file).is_none());
    assert_eq!(overlay_mgr.get(new_file).unwrap().text, "pub fn foo() {}\n");
    assert_eq!(overlay_mgr.get(ctrl_file).unwrap().text, "pub fn control() {}\n");
}

#[test]
fn test_did_rename_files_dirty_overlay_preserves_text_and_version() {
    let mut overlay_mgr = OverlayManager::new();
    let (old_file, old_uri) = ("/tmp/dirty_old.rs", "file:///tmp/dirty_old.rs");
    let (new_file, new_uri) = ("/tmp/dirty_new.rs", "file:///tmp/dirty_new.rs");
    overlay_mgr.open_with_provenance(old_uri, old_file, 1, "pub fn pristine() {}\n".to_string(), true);
    let edits = vec![json!({ "text": "pub fn uncommitted() {}\n" })];
    overlay_mgr.apply_content_changes(old_file, 5, &edits);
    assert!(overlay_mgr.get(old_file).unwrap().is_dirty());
    let (mut mr, mut conv) = (MultiRootWorkspace::new(), PositionConverter::new());
    let msg = json!({ "params": { "files": [{ "oldUri": old_uri, "newUri": new_uri }] } });
    mr.handle_file_event("workspace/didRenameFiles", &msg, &mut overlay_mgr, &mut conv);
    assert!(overlay_mgr.get(old_file).is_none());
    assert!(overlay_mgr.get(new_file).unwrap().is_dirty());
    assert_eq!(overlay_mgr.get(new_file).unwrap().version, 5);
    assert_eq!(overlay_mgr.get(new_file).unwrap().text, "pub fn uncommitted() {}\n");
}

#[test]
fn test_did_rename_files_unopened_file_noop() {
    let mut overlay_mgr = OverlayManager::new();
    let (mut mr, mut conv) = (MultiRootWorkspace::new(), PositionConverter::new());
    let msg = json!({ "params": { "files": [{ "oldUri": "file:///tmp/unopened_old.rs", "newUri": "file:///tmp/unopened_new.rs" }] } });
    mr.handle_file_event("workspace/didRenameFiles", &msg, &mut overlay_mgr, &mut conv);
    assert!(overlay_mgr.get("/tmp/unopened_new.rs").is_none());
}

#[test]
fn test_did_rename_files_multi_rename_simultaneous() {
    let mut overlay_mgr = OverlayManager::new();
    overlay_mgr.open_with_provenance("file:///tmp/m1.rs", "/tmp/m1.rs", 1, "fn m1() {}\n".to_string(), true);
    overlay_mgr.open_with_provenance("file:///tmp/m2.rs", "/tmp/m2.rs", 1, "fn m2() {}\n".to_string(), true);
    let (mut mr, mut conv) = (MultiRootWorkspace::new(), PositionConverter::new());
    let msg = json!({ "params": { "files": [
        { "oldUri": "file:///tmp/m1.rs", "newUri": "file:///tmp/m1_renamed.rs" },
        { "oldUri": "file:///tmp/m2.rs", "newUri": "file:///tmp/m2_renamed.rs" }
    ] } });
    mr.handle_file_event("workspace/didRenameFiles", &msg, &mut overlay_mgr, &mut conv);
    assert!(overlay_mgr.get("/tmp/m1.rs").is_none());
    assert!(overlay_mgr.get("/tmp/m2.rs").is_none());
    assert_eq!(overlay_mgr.get("/tmp/m1_renamed.rs").unwrap().text, "fn m1() {}\n");
    assert_eq!(overlay_mgr.get("/tmp/m2_renamed.rs").unwrap().text, "fn m2() {}\n");
}

#[test]
fn test_did_rename_files_empty_files_noop() {
    let mut overlay_mgr = OverlayManager::new();
    overlay_mgr.open_with_provenance("file:///tmp/empty_ctrl.rs", "/tmp/empty_ctrl.rs", 1, "fn ec() {}\n".to_string(), true);
    let (mut mr, mut conv) = (MultiRootWorkspace::new(), PositionConverter::new());
    let msg = json!({ "params": { "files": [] } });
    mr.handle_file_event("workspace/didRenameFiles", &msg, &mut overlay_mgr, &mut conv);
    assert_eq!(overlay_mgr.get("/tmp/empty_ctrl.rs").unwrap().text, "fn ec() {}\n");
}

#[test]
fn test_did_rename_files_missing_files_key_noop() {
    let mut overlay_mgr = OverlayManager::new();
    overlay_mgr.open_with_provenance("file:///tmp/miss_ctrl.rs", "/tmp/miss_ctrl.rs", 1, "fn mc() {}\n".to_string(), true);
    let (mut mr, mut conv) = (MultiRootWorkspace::new(), PositionConverter::new());
    let msg = json!({ "params": {} });
    mr.handle_file_event("workspace/didRenameFiles", &msg, &mut overlay_mgr, &mut conv);
    assert_eq!(overlay_mgr.get("/tmp/miss_ctrl.rs").unwrap().text, "fn mc() {}\n");
}

#[test]
fn test_did_rename_files_missing_fields_skipped() {
    let mut overlay_mgr = OverlayManager::new();
    overlay_mgr.open_with_provenance("file:///tmp/incomp.rs", "/tmp/incomp.rs", 1, "fn ic() {}\n".to_string(), true);
    let (mut mr, mut conv) = (MultiRootWorkspace::new(), PositionConverter::new());
    let msg = json!({ "params": { "files": [{ "oldUri": "file:///tmp/orphan.rs" }, { "newUri": "file:///tmp/dest.rs" }] } });
    mr.handle_file_event("workspace/didRenameFiles", &msg, &mut overlay_mgr, &mut conv);
    assert_eq!(overlay_mgr.get("/tmp/incomp.rs").unwrap().text, "fn ic() {}\n");
}

#[test]
fn test_did_rename_files_clears_position_converter() {
    let ts1 = create_temp_shard_with_symbol("ren_conv", "ConvControl");
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
    let msg = json!({ "params": { "files": [{ "oldUri": "file:///tmp/old.rs", "newUri": "file:///tmp/new.rs" }] } });
    mr.handle_file_event("workspace/didRenameFiles", &msg, &mut overlay_mgr, &mut conv);
    let col_fresh = conv.char_col_to_utf16_col(test_file.to_str().unwrap(), 1, 2);
    assert_eq!(col_fresh, 1);
}

#[test]
fn test_did_rename_files_cyclic_rename_roundtrip() {
    let mut overlay_mgr = OverlayManager::new();
    let (file_a, uri_a) = ("/tmp/cycle_a.rs", "file:///tmp/cycle_a.rs");
    let (file_b, uri_b) = ("/tmp/cycle_b.rs", "file:///tmp/cycle_b.rs");
    overlay_mgr.open_with_provenance(uri_a, file_a, 1, "fn cycle() {}\n".to_string(), true);
    let (mut mr, mut conv) = (MultiRootWorkspace::new(), PositionConverter::new());
    let step1 = json!({ "params": { "files": [{ "oldUri": uri_a, "newUri": uri_b }] } });
    mr.handle_file_event("workspace/didRenameFiles", &step1, &mut overlay_mgr, &mut conv);
    assert!(overlay_mgr.get(file_a).is_none());
    assert_eq!(overlay_mgr.get(file_b).unwrap().text, "fn cycle() {}\n");
    let step2 = json!({ "params": { "files": [{ "oldUri": uri_b, "newUri": uri_a }] } });
    mr.handle_file_event("workspace/didRenameFiles", &step2, &mut overlay_mgr, &mut conv);
    assert!(overlay_mgr.get(file_b).is_none());
    assert_eq!(overlay_mgr.get(file_a).unwrap().text, "fn cycle() {}\n");
}

#[test]
fn test_did_rename_files_chain_rename() {
    let mut overlay_mgr = OverlayManager::new();
    let (f1, u1) = ("/tmp/chain_1.rs", "file:///tmp/chain_1.rs");
    let (f2, u2) = ("/tmp/chain_2.rs", "file:///tmp/chain_2.rs");
    let (f3, u3) = ("/tmp/chain_3.rs", "file:///tmp/chain_3.rs");
    overlay_mgr.open_with_provenance(u1, f1, 1, "fn chain() {}\n".to_string(), true);
    let (mut mr, mut conv) = (MultiRootWorkspace::new(), PositionConverter::new());
    let msg1 = json!({ "params": { "files": [{ "oldUri": u1, "newUri": u2 }] } });
    mr.handle_file_event("workspace/didRenameFiles", &msg1, &mut overlay_mgr, &mut conv);
    assert!(overlay_mgr.get(f1).is_none());
    assert_eq!(overlay_mgr.get(f2).unwrap().text, "fn chain() {}\n");
    let msg2 = json!({ "params": { "files": [{ "oldUri": u2, "newUri": u3 }] } });
    mr.handle_file_event("workspace/didRenameFiles", &msg2, &mut overlay_mgr, &mut conv);
    assert!(overlay_mgr.get(f2).is_none());
    assert_eq!(overlay_mgr.get(f3).unwrap().text, "fn chain() {}\n");
}

#[test]
fn test_did_rename_files_followed_by_did_change() {
    let mut overlay_mgr = OverlayManager::new();
    let (f_old, u_old) = ("/tmp/ren_chg_old.rs", "file:///tmp/ren_chg_old.rs");
    let (f_new, u_new) = ("/tmp/ren_chg_new.rs", "file:///tmp/ren_chg_new.rs");
    overlay_mgr.open_with_provenance(u_old, f_old, 1, "fn initial() {}\n".to_string(), true);
    let (mut mr, mut conv) = (MultiRootWorkspace::new(), PositionConverter::new());
    let msg = json!({ "params": { "files": [{ "oldUri": u_old, "newUri": u_new }] } });
    mr.handle_file_event("workspace/didRenameFiles", &msg, &mut overlay_mgr, &mut conv);
    let edits = vec![json!({ "text": "fn modified_after_rename() {}\n" })];
    assert!(overlay_mgr.apply_content_changes(f_new, 2, &edits));
    assert_eq!(overlay_mgr.get(f_new).unwrap().text, "fn modified_after_rename() {}\n");
    assert_eq!(overlay_mgr.get(f_new).unwrap().version, 2);
}

#[test]
fn test_did_rename_files_followed_by_did_close() {
    let mut overlay_mgr = OverlayManager::new();
    let (f_old, u_old) = ("/tmp/ren_close_old.rs", "file:///tmp/ren_close_old.rs");
    let (f_new, u_new) = ("/tmp/ren_close_new.rs", "file:///tmp/ren_close_new.rs");
    overlay_mgr.open_with_provenance(u_old, f_old, 1, "fn close_target() {}\n".to_string(), true);
    let (mut mr, mut conv) = (MultiRootWorkspace::new(), PositionConverter::new());
    let msg = json!({ "params": { "files": [{ "oldUri": u_old, "newUri": u_new }] } });
    mr.handle_file_event("workspace/didRenameFiles", &msg, &mut overlay_mgr, &mut conv);
    assert!(overlay_mgr.get(f_new).is_some());
    overlay_mgr.close(f_new);
    assert!(overlay_mgr.get(f_new).is_none());
}

#[test]
fn test_did_rename_files_percent_encoded_uri() {
    let mut overlay_mgr = OverlayManager::new();
    let old_uri = "file:///tmp/old%20percent.rs";
    let new_uri = "file:///tmp/new%20percent.rs";
    let old_path = uri_to_path(old_uri);
    let new_path = uri_to_path(new_uri);
    overlay_mgr.open_with_provenance(old_uri, &old_path, 1, "fn pct() {}\n".to_string(), true);
    let (mut mr, mut conv) = (MultiRootWorkspace::new(), PositionConverter::new());
    let msg = json!({ "params": { "files": [{ "oldUri": old_uri, "newUri": new_uri }] } });
    mr.handle_file_event("workspace/didRenameFiles", &msg, &mut overlay_mgr, &mut conv);
    assert!(overlay_mgr.get(&old_path).is_none());
    assert_eq!(overlay_mgr.get(&new_path).unwrap().text, "fn pct() {}\n");
}

#[test]
fn test_did_rename_files_multi_root_secondary_workspace() {
    let mut mr = MultiRootWorkspace::new();
    let ts1 = create_temp_shard_with_symbol("ren_mr_1", "R1");
    let ts2 = create_temp_shard_with_symbol("ren_mr_2", "R2");
    assert!(mr.add_root(ts1.shard.clone()));
    assert!(mr.add_root(ts2.shard.clone()));
    let mut overlay_mgr = OverlayManager::new();
    let sec_old = ts2.dir.join("src/sec_old.rs");
    let sec_new = ts2.dir.join("src/sec_new.rs");
    let (u_old, u_new) = (to_uri(&sec_old), to_uri(&sec_new));
    overlay_mgr.open_with_provenance(&u_old, sec_old.to_str().unwrap(), 1, "fn sec() {}\n".to_string(), true);
    let mut conv = PositionConverter::new();
    let msg = json!({ "params": { "files": [{ "oldUri": u_old, "newUri": u_new }] } });
    mr.handle_file_event("workspace/didRenameFiles", &msg, &mut overlay_mgr, &mut conv);
    assert!(overlay_mgr.get(sec_old.to_str().unwrap()).is_none());
    assert_eq!(overlay_mgr.get(sec_new.to_str().unwrap()).unwrap().text, "fn sec() {}\n");
    assert_eq!(mr.len(), 2);
}
