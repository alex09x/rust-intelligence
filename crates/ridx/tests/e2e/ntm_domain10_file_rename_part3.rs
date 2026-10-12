/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for workspace/didRenameFiles (Domain 10, Part 3).
//! Verifies overlay rebinding, dirty state preservation, full PositionConverter cache invalidation,
//! file swapping, and CRLF preservation.

use ridx::lsp::protocol::{path_to_uri, PositionConverter};
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter, RawDefinition};
use serde_json::json;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static RENAME_COUNTER_P3: AtomicU64 = AtomicU64::new(200);

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
    let id = RENAME_COUNTER_P3.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_ren_p3_{}_{}_{}", name, std::process::id(), id));
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
fn test_did_rename_files_rebinding_clears_old_overlay_entry() {
    let ts = create_temp_shard_with_symbol("p3_rebind", "SymRebind");
    let mut mr = MultiRootWorkspace::new();
    assert!(mr.add_root(ts.shard.clone()));
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());

    let old_p = "/tmp/rebind_old.rs";
    let new_p = "/tmp/rebind_new.rs";
    let old_u = "file:///tmp/rebind_old.rs";
    let new_u = "file:///tmp/rebind_new.rs";

    overlay_mgr.open_with_provenance(old_u, old_p, 1, "pub fn original() {}\n".to_string(), true);
    assert_eq!(overlay_mgr.get(old_p).map(|d| d.version), Some(1));

    let msg = json!({
        "params": {
            "files": [{ "oldUri": old_u, "newUri": new_u }]
        }
    });
    mr.handle_file_event("workspace/didRenameFiles", &msg, &mut overlay_mgr, &mut conv);

    assert_eq!(overlay_mgr.get(old_p).map(|d| d.version), None);
    assert_eq!(overlay_mgr.get(new_p).map(|d| d.version), Some(1));
    assert_eq!(overlay_mgr.get(new_p).unwrap().text, "pub fn original() {}\n");
}

#[test]
fn test_did_rename_files_dirty_overlay_keeps_dirty_state_after_rebind() {
    let ts = create_temp_shard_with_symbol("p3_dirty", "SymDirty");
    let mut mr = MultiRootWorkspace::new();
    assert!(mr.add_root(ts.shard.clone()));
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());

    let old_p = "/tmp/dirty_old.rs";
    let new_p = "/tmp/dirty_new.rs";
    let old_u = "file:///tmp/dirty_old.rs";
    let new_u = "file:///tmp/dirty_new.rs";

    overlay_mgr.open_with_provenance(old_u, old_p, 1, "pub fn v1() {}\n".to_string(), true);
    let edits = vec![json!({ "text": "pub fn v2_dirty() {}\n" })];
    overlay_mgr.apply_content_changes(old_p, 2, &edits);
    assert!(overlay_mgr.get(old_p).unwrap().is_dirty());

    let msg = json!({
        "params": {
            "files": [{ "oldUri": old_u, "newUri": new_u }]
        }
    });
    mr.handle_file_event("workspace/didRenameFiles", &msg, &mut overlay_mgr, &mut conv);

    let doc = overlay_mgr.get(new_p).unwrap();
    assert!(doc.is_dirty());
    assert_eq!(doc.version, 2);
    assert_eq!(doc.text, "pub fn v2_dirty() {}\n");
}

#[test]
fn test_did_rename_files_clears_conv_for_both_old_and_new_paths() {
    let ts = create_temp_shard_with_symbol("p3_clear_conv", "SymClearConv");
    let mut mr = MultiRootWorkspace::new();
    assert!(mr.add_root(ts.shard.clone()));
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());

    let f1 = ts.dir.join("src/conv1.rs");
    let f2 = ts.dir.join("src/conv2.rs");
    std::fs::write(&f1, "🦀abc\nsecond").unwrap();
    std::fs::write(&f2, "🦀def\nsecond").unwrap();

    assert_eq!(conv.char_col_to_utf16_col(f1.to_str().unwrap(), 1, 2), 2);
    assert_eq!(conv.char_col_to_utf16_col(f2.to_str().unwrap(), 1, 2), 2);

    std::fs::write(&f1, "xabc\nsecond").unwrap();
    std::fs::write(&f2, "ydef\nsecond").unwrap();

    let msg = json!({
        "params": {
            "files": [{ "oldUri": to_uri(&f1), "newUri": to_uri(&f2) }]
        }
    });
    mr.handle_file_event("workspace/didRenameFiles", &msg, &mut overlay_mgr, &mut conv);

    assert_eq!(conv.char_col_to_utf16_col(f1.to_str().unwrap(), 1, 2), 1);
    assert_eq!(conv.char_col_to_utf16_col(f2.to_str().unwrap(), 1, 2), 1);
}

#[test]
fn test_did_rename_files_swap_two_files() {
    let ts = create_temp_shard_with_symbol("p3_swap", "SymSwap");
    let mut mr = MultiRootWorkspace::new();
    assert!(mr.add_root(ts.shard.clone()));
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());

    let p_a = "/tmp/swap_a.rs";
    let p_b = "/tmp/swap_b.rs";
    let _p_tmp = "/tmp/swap_tmp.rs";
    let u_a = "file:///tmp/swap_a.rs";
    let u_b = "file:///tmp/swap_b.rs";
    let u_tmp = "file:///tmp/swap_tmp.rs";

    overlay_mgr.open_with_provenance(u_a, p_a, 1, "Content A\n".to_string(), true);
    overlay_mgr.open_with_provenance(u_b, p_b, 1, "Content B\n".to_string(), true);

    let msg = json!({
        "params": {
            "files": [
                { "oldUri": u_a, "newUri": u_tmp },
                { "oldUri": u_b, "newUri": u_a },
                { "oldUri": u_tmp, "newUri": u_b }
            ]
        }
    });
    mr.handle_file_event("workspace/didRenameFiles", &msg, &mut overlay_mgr, &mut conv);

    assert_eq!(overlay_mgr.get(p_a).unwrap().text, "Content B\n");
    assert_eq!(overlay_mgr.get(p_b).unwrap().text, "Content A\n");
}

#[test]
fn test_did_rename_files_extension_change_rs_to_bak() {
    let ts = create_temp_shard_with_symbol("p3_ext", "SymExt");
    let mut mr = MultiRootWorkspace::new();
    assert!(mr.add_root(ts.shard.clone()));
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());

    let old_p = "/tmp/code.rs";
    let new_p = "/tmp/code.rs.bak";
    let old_u = "file:///tmp/code.rs";
    let new_u = "file:///tmp/code.rs.bak";

    overlay_mgr.open_with_provenance(old_u, old_p, 1, "fn backup_me() {}\n".to_string(), true);
    let msg = json!({
        "params": {
            "files": [{ "oldUri": old_u, "newUri": new_u }]
        }
    });
    mr.handle_file_event("workspace/didRenameFiles", &msg, &mut overlay_mgr, &mut conv);

    assert_eq!(overlay_mgr.get(old_p).map(|d| d.version), None);
    assert_eq!(overlay_mgr.get(new_p).unwrap().text, "fn backup_me() {}\n");
}

#[test]
fn test_did_rename_files_deeply_nested_directory_rename() {
    let ts = create_temp_shard_with_symbol("p3_deep", "SymDeep");
    let mut mr = MultiRootWorkspace::new();
    assert!(mr.add_root(ts.shard.clone()));
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());

    let old_p = "/tmp/shallow.rs";
    let new_p = "/tmp/a/b/c/d/e/deep.rs";
    let old_u = "file:///tmp/shallow.rs";
    let new_u = "file:///tmp/a/b/c/d/e/deep.rs";

    overlay_mgr.open_with_provenance(old_u, old_p, 5, "fn deep_code() {}\n".to_string(), true);
    let msg = json!({
        "params": {
            "files": [{ "oldUri": old_u, "newUri": new_u }]
        }
    });
    mr.handle_file_event("workspace/didRenameFiles", &msg, &mut overlay_mgr, &mut conv);

    let doc = overlay_mgr.get(new_p).unwrap();
    assert_eq!(doc.version, 5);
    assert_eq!(doc.text, "fn deep_code() {}\n");
}

#[test]
fn test_did_rename_files_null_params_handled() {
    let ts = create_temp_shard_with_symbol("p3_null_par", "SymNullPar");
    let mut mr = MultiRootWorkspace::new();
    assert!(mr.add_root(ts.shard.clone()));
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());

    let msg = json!({ "params": serde_json::Value::Null });
    mr.handle_file_event("workspace/didRenameFiles", &msg, &mut overlay_mgr, &mut conv);
    assert_eq!(mr.len(), 1);
}

#[test]
fn test_did_rename_files_empty_uris_handled() {
    let ts = create_temp_shard_with_symbol("p3_empty_uri", "SymEmptyUri");
    let mut mr = MultiRootWorkspace::new();
    assert!(mr.add_root(ts.shard.clone()));
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());

    let msg = json!({
        "params": {
            "files": [{ "oldUri": "", "newUri": "" }]
        }
    });
    mr.handle_file_event("workspace/didRenameFiles", &msg, &mut overlay_mgr, &mut conv);
    assert_eq!(mr.len(), 1);
}

#[test]
fn test_did_rename_files_idempotent_duplicate_events() {
    let ts = create_temp_shard_with_symbol("p3_idem", "SymIdem");
    let mut mr = MultiRootWorkspace::new();
    assert!(mr.add_root(ts.shard.clone()));
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());

    let old_p = "/tmp/idem_src.rs";
    let new_p = "/tmp/idem_dst.rs";
    let old_u = "file:///tmp/idem_src.rs";
    let new_u = "file:///tmp/idem_dst.rs";

    overlay_mgr.open_with_provenance(old_u, old_p, 1, "pub fn idem() {}\n".to_string(), true);
    let msg = json!({
        "params": {
            "files": [{ "oldUri": old_u, "newUri": new_u }]
        }
    });

    mr.handle_file_event("workspace/didRenameFiles", &msg, &mut overlay_mgr, &mut conv);
    mr.handle_file_event("workspace/didRenameFiles", &msg, &mut overlay_mgr, &mut conv);

    assert_eq!(overlay_mgr.get(new_p).map(|d| d.version), Some(1));
    assert_eq!(overlay_mgr.get(old_p).map(|d| d.version), None);
}

#[test]
fn test_did_rename_files_crlf_file_rebound_cleanly() {
    let ts = create_temp_shard_with_symbol("p3_crlf", "SymCrlf");
    let mut mr = MultiRootWorkspace::new();
    assert!(mr.add_root(ts.shard.clone()));
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());

    let old_p = "/tmp/crlf_old.rs";
    let new_p = "/tmp/crlf_new.rs";
    let old_u = "file:///tmp/crlf_old.rs";
    let new_u = "file:///tmp/crlf_new.rs";

    let crlf_content = "pub fn windows() {\r\n    let val = 42;\r\n}\r\n";
    overlay_mgr.open_with_provenance(old_u, old_p, 1, crlf_content.to_string(), true);

    let msg = json!({
        "params": {
            "files": [{ "oldUri": old_u, "newUri": new_u }]
        }
    });
    mr.handle_file_event("workspace/didRenameFiles", &msg, &mut overlay_mgr, &mut conv);

    let doc = overlay_mgr.get(new_p).unwrap();
    assert_eq!(doc.text, crlf_content);
    assert_eq!(doc.version, 1);
}
