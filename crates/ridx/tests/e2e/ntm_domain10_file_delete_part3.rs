/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for workspace/didDeleteFiles (Domain 10, Part 3).
//! Verifies overlay eviction, dirty buffer discard, full PositionConverter cache invalidation,
//! reopen after deletion, and CRLF handling.

use ridx::lsp::protocol::{path_to_uri, PositionConverter};
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter, RawDefinition};
use serde_json::json;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static DELETE_COUNTER_P3: AtomicU64 = AtomicU64::new(200);

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
    let id = DELETE_COUNTER_P3.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_del_p3_{}_{}_{}", name, std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
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

#[test]
fn test_did_delete_files_multiple_overlays_evicted_simultaneously() {
    let ts = create_temp_shard_with_symbol("p3_del_multi", "SymDelMulti");
    let mut mr = MultiRootWorkspace::new();
    assert!(mr.add_root(ts.shard.clone()));
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());

    let p1 = "/tmp/del_mod1.rs";
    let p2 = "/tmp/del_mod2.rs";
    let p3 = "/tmp/del_mod3.rs";
    let u1 = "file:///tmp/del_mod1.rs";
    let u2 = "file:///tmp/del_mod2.rs";
    let u3 = "file:///tmp/del_mod3.rs";

    overlay_mgr.open_with_provenance(u1, p1, 1, "fn m1() {}\n".to_string(), true);
    overlay_mgr.open_with_provenance(u2, p2, 1, "fn m2() {}\n".to_string(), true);
    overlay_mgr.open_with_provenance(u3, p3, 1, "fn m3() {}\n".to_string(), true);

    let msg = json!({
        "params": {
            "files": [{ "uri": u1 }, { "uri": u2 }, { "uri": u3 }]
        }
    });
    mr.handle_file_event("workspace/didDeleteFiles", &msg, &mut overlay_mgr, &mut conv);

    assert_eq!(overlay_mgr.get(p1).map(|d| d.version), None);
    assert_eq!(overlay_mgr.get(p2).map(|d| d.version), None);
    assert_eq!(overlay_mgr.get(p3).map(|d| d.version), None);
}

#[test]
fn test_did_delete_files_unrelated_overlay_remains_intact() {
    let ts = create_temp_shard_with_symbol("p3_del_unrel", "SymDelUnrel");
    let mut mr = MultiRootWorkspace::new();
    assert!(mr.add_root(ts.shard.clone()));
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());

    let p_a = "/tmp/delete_target.rs";
    let p_b = "/tmp/preserve_target.rs";
    let u_a = "file:///tmp/delete_target.rs";
    let u_b = "file:///tmp/preserve_target.rs";

    overlay_mgr.open_with_provenance(u_a, p_a, 1, "fn doomed() {}\n".to_string(), true);
    overlay_mgr.open_with_provenance(u_b, p_b, 1, "fn survivor() {}\n".to_string(), true);

    let msg = json!({
        "params": {
            "files": [{ "uri": u_a }]
        }
    });
    mr.handle_file_event("workspace/didDeleteFiles", &msg, &mut overlay_mgr, &mut conv);

    assert_eq!(overlay_mgr.get(p_a).map(|d| d.version), None);
    assert_eq!(overlay_mgr.get(p_b).map(|d| d.version), Some(1));
    assert_eq!(overlay_mgr.get(p_b).unwrap().text, "fn survivor() {}\n");
}

#[test]
fn test_did_delete_files_clears_conv_cache_for_workspace() {
    let ts = create_temp_shard_with_symbol("p3_del_conv", "SymDelConv");
    let mut mr = MultiRootWorkspace::new();
    assert!(mr.add_root(ts.shard.clone()));
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());

    let f1 = ts.dir.join("src/del_c1.rs");
    let f2 = ts.dir.join("src/del_c2.rs");
    std::fs::write(&f1, "🦀abc\nline2").unwrap();
    std::fs::write(&f2, "🦀def\nline2").unwrap();

    assert_eq!(conv.char_col_to_utf16_col(f1.to_str().unwrap(), 1, 2), 2);
    assert_eq!(conv.char_col_to_utf16_col(f2.to_str().unwrap(), 1, 2), 2);

    std::fs::write(&f1, "xabc\nline2").unwrap();
    std::fs::write(&f2, "ydef\nline2").unwrap();

    let msg = json!({
        "params": {
            "files": [{ "uri": to_uri(&f1) }]
        }
    });
    mr.handle_file_event("workspace/didDeleteFiles", &msg, &mut overlay_mgr, &mut conv);

    assert_eq!(conv.char_col_to_utf16_col(f1.to_str().unwrap(), 1, 2), 1);
    assert_eq!(conv.char_col_to_utf16_col(f2.to_str().unwrap(), 1, 2), 1);
}

#[test]
fn test_did_delete_files_dirty_overlay_with_unsaved_changes_evicted() {
    let ts = create_temp_shard_with_symbol("p3_del_dirty", "SymDelDirty");
    let mut mr = MultiRootWorkspace::new();
    assert!(mr.add_root(ts.shard.clone()));
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());

    let p = "/tmp/dirty_doomed.rs";
    let u = "file:///tmp/dirty_doomed.rs";

    overlay_mgr.open_with_provenance(u, p, 1, "pub fn initial() {}\n".to_string(), true);
    let edits = vec![json!({ "text": "pub fn modified_unsaved() {}\n" })];
    overlay_mgr.apply_content_changes(p, 2, &edits);
    assert!(overlay_mgr.get(p).unwrap().is_dirty());

    let msg = json!({
        "params": {
            "files": [{ "uri": u }]
        }
    });
    mr.handle_file_event("workspace/didDeleteFiles", &msg, &mut overlay_mgr, &mut conv);

    assert_eq!(overlay_mgr.get(p).map(|d| d.version), None);
}

#[test]
fn test_did_delete_files_null_params_handled() {
    let ts = create_temp_shard_with_symbol("p3_del_null", "SymDelNull");
    let mut mr = MultiRootWorkspace::new();
    assert!(mr.add_root(ts.shard.clone()));
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());

    let msg = json!({ "params": serde_json::Value::Null });
    mr.handle_file_event("workspace/didDeleteFiles", &msg, &mut overlay_mgr, &mut conv);
    assert_eq!(mr.len(), 1);
}

#[test]
fn test_did_delete_files_empty_uri_entry_handled() {
    let ts = create_temp_shard_with_symbol("p3_del_empty", "SymDelEmpty");
    let mut mr = MultiRootWorkspace::new();
    assert!(mr.add_root(ts.shard.clone()));
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());

    let msg = json!({
        "params": {
            "files": [{ "uri": "" }]
        }
    });
    mr.handle_file_event("workspace/didDeleteFiles", &msg, &mut overlay_mgr, &mut conv);
    assert_eq!(mr.len(), 1);
}

#[test]
fn test_did_delete_files_file_outside_workspace_roots() {
    let ts = create_temp_shard_with_symbol("p3_del_outside", "SymDelOutside");
    let mut mr = MultiRootWorkspace::new();
    assert!(mr.add_root(ts.shard.clone()));
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());

    let ext_p = "/tmp/system_unrelated.rs";
    let ext_u = "file:///tmp/system_unrelated.rs";
    overlay_mgr.open_with_provenance(ext_u, ext_p, 1, "pub fn ext() {}\n".to_string(), true);

    let msg = json!({
        "params": {
            "files": [{ "uri": ext_u }]
        }
    });
    mr.handle_file_event("workspace/didDeleteFiles", &msg, &mut overlay_mgr, &mut conv);

    assert_eq!(overlay_mgr.get(ext_p).map(|d| d.version), None);
    assert_eq!(mr.len(), 1);
}

#[test]
fn test_did_delete_files_idempotent_eviction() {
    let ts = create_temp_shard_with_symbol("p3_del_idem", "SymDelIdem");
    let mut mr = MultiRootWorkspace::new();
    assert!(mr.add_root(ts.shard.clone()));
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());

    let p = "/tmp/idem_delete.rs";
    let u = "file:///tmp/idem_delete.rs";

    overlay_mgr.open_with_provenance(u, p, 1, "pub fn idem() {}\n".to_string(), true);
    let msg = json!({
        "params": {
            "files": [{ "uri": u }]
        }
    });

    mr.handle_file_event("workspace/didDeleteFiles", &msg, &mut overlay_mgr, &mut conv);
    mr.handle_file_event("workspace/didDeleteFiles", &msg, &mut overlay_mgr, &mut conv);

    assert_eq!(overlay_mgr.get(p).map(|d| d.version), None);
}

#[test]
fn test_did_delete_files_reopen_with_new_version_after_delete() {
    let ts = create_temp_shard_with_symbol("p3_del_reopen", "SymDelReopen");
    let mut mr = MultiRootWorkspace::new();
    assert!(mr.add_root(ts.shard.clone()));
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());

    let p = "/tmp/reopen_after_delete.rs";
    let u = "file:///tmp/reopen_after_delete.rs";

    overlay_mgr.open_with_provenance(u, p, 1, "pub fn first() {}\n".to_string(), true);
    let msg = json!({
        "params": {
            "files": [{ "uri": u }]
        }
    });
    mr.handle_file_event("workspace/didDeleteFiles", &msg, &mut overlay_mgr, &mut conv);
    assert_eq!(overlay_mgr.get(p).map(|d| d.version), None);

    overlay_mgr.open_with_provenance(u, p, 1, "pub fn recreated() {}\n".to_string(), true);
    assert_eq!(overlay_mgr.get(p).map(|d| d.version), Some(1));
    assert_eq!(overlay_mgr.get(p).unwrap().text, "pub fn recreated() {}\n");
}

#[test]
fn test_did_delete_files_crlf_overlay_evicted_cleanly() {
    let ts = create_temp_shard_with_symbol("p3_del_crlf", "SymDelCrlf");
    let mut mr = MultiRootWorkspace::new();
    assert!(mr.add_root(ts.shard.clone()));
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());

    let p = "/tmp/crlf_delete.rs";
    let u = "file:///tmp/crlf_delete.rs";

    overlay_mgr.open_with_provenance(u, p, 1, "pub fn crlf() {\r\n    1\r\n}\r\n".to_string(), true);
    let msg = json!({
        "params": {
            "files": [{ "uri": u }]
        }
    });
    mr.handle_file_event("workspace/didDeleteFiles", &msg, &mut overlay_mgr, &mut conv);

    assert_eq!(overlay_mgr.get(p).map(|d| d.version), None);
}
