/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for workspace/didCreateFiles (Domain 10, Part 3).
//! Verifies PositionConverter cache invalidation, overlay preservation, idempotency, and URI encoding.

use ridx::lsp::protocol::{path_to_uri, PositionConverter};
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter, RawDefinition};
use serde_json::json;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static CREATE_COUNTER_P3: AtomicU64 = AtomicU64::new(200);

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
    let id = CREATE_COUNTER_P3.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_create_p3_{}_{}_{}", name, std::process::id(), id));
    std::fs::create_dir_all(dir.join("src")).unwrap();
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

#[test]
fn test_did_create_files_multiple_files_clears_multiple_conv_caches() {
    let ts = create_temp_shard_with_symbol("p3_multiclear", "ClearControl");
    let mut mr = MultiRootWorkspace::new();
    assert!(mr.add_root(ts.shard.clone()));
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());

    let file1 = ts.dir.join("src/mod1.rs");
    let file2 = ts.dir.join("src/mod2.rs");
    std::fs::write(&file1, "🦀abc\nline2").unwrap();
    std::fs::write(&file2, "🦀def\nline2").unwrap();

    let col1_warm = conv.char_col_to_utf16_col(file1.to_str().unwrap(), 1, 2);
    let col2_warm = conv.char_col_to_utf16_col(file2.to_str().unwrap(), 1, 2);
    assert_eq!(col1_warm, 2);
    assert_eq!(col2_warm, 2);

    std::fs::write(&file1, "xabc\nline2").unwrap();
    std::fs::write(&file2, "ydef\nline2").unwrap();
    assert_eq!(conv.char_col_to_utf16_col(file1.to_str().unwrap(), 1, 2), 2);
    assert_eq!(conv.char_col_to_utf16_col(file2.to_str().unwrap(), 1, 2), 2);

    let msg = json!({
        "params": {
            "files": [
                { "uri": to_uri(&file1) },
                { "uri": to_uri(&file2) }
            ]
        }
    });
    mr.handle_file_event("workspace/didCreateFiles", &msg, &mut overlay_mgr, &mut conv);

    let col1_fresh = conv.char_col_to_utf16_col(file1.to_str().unwrap(), 1, 2);
    let col2_fresh = conv.char_col_to_utf16_col(file2.to_str().unwrap(), 1, 2);
    assert_eq!(col1_fresh, 1);
    assert_eq!(col2_fresh, 1);
}

#[test]
fn test_did_create_files_clears_entire_converter_cache() {
    let ts = create_temp_shard_with_symbol("p3_unrelated", "UnrelatedControl");
    let mut mr = MultiRootWorkspace::new();
    assert!(mr.add_root(ts.shard.clone()));
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());

    let file_a = ts.dir.join("src/file_a.rs");
    let file_b = ts.dir.join("src/file_b.rs");
    std::fs::write(&file_a, "🦀alpha\n").unwrap();
    std::fs::write(&file_b, "🦀beta\n").unwrap();

    assert_eq!(conv.char_col_to_utf16_col(file_a.to_str().unwrap(), 1, 2), 2);
    assert_eq!(conv.char_col_to_utf16_col(file_b.to_str().unwrap(), 1, 2), 2);

    std::fs::write(&file_a, "xalpha\n").unwrap();
    std::fs::write(&file_b, "xbeta\n").unwrap();

    let msg = json!({
        "params": {
            "files": [{ "uri": to_uri(&file_a) }]
        }
    });
    mr.handle_file_event("workspace/didCreateFiles", &msg, &mut overlay_mgr, &mut conv);

    assert_eq!(conv.char_col_to_utf16_col(file_a.to_str().unwrap(), 1, 2), 1);
    assert_eq!(conv.char_col_to_utf16_col(file_b.to_str().unwrap(), 1, 2), 1);
}

#[test]
fn test_did_create_files_overlay_opened_before_notification() {
    let ts = create_temp_shard_with_symbol("p3_over_open", "OverSym");
    let mut mr = MultiRootWorkspace::new();
    assert!(mr.add_root(ts.shard.clone()));
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());

    let buf_path = "/tmp/pre_opened_buffer.rs";
    let buf_uri = "file:///tmp/pre_opened_buffer.rs";
    overlay_mgr.open_with_provenance(buf_uri, buf_path, 1, "pub fn created_pre() {}\n".to_string(), true);

    let msg = json!({ "params": { "files": [{ "uri": buf_uri }] } });
    mr.handle_file_event("workspace/didCreateFiles", &msg, &mut overlay_mgr, &mut conv);

    let doc = overlay_mgr.get(buf_path).unwrap();
    assert_eq!(doc.version, 1);
    assert_eq!(doc.text, "pub fn created_pre() {}\n");
}

#[test]
fn test_did_create_files_overlay_content_preserved_after_notification() {
    let mut overlay_mgr = OverlayManager::new();
    let dirty_path = "/tmp/dirty_verify_preserved.rs";
    let dirty_uri = "file:///tmp/dirty_verify_preserved.rs";
    overlay_mgr.open_with_provenance(dirty_uri, dirty_path, 1, "pub struct Initial;\n".to_string(), true);
    let edits = vec![json!({ "text": "pub struct Modified;\n" })];
    overlay_mgr.apply_content_changes(dirty_path, 2, &edits);

    let (mut mr, mut conv) = (MultiRootWorkspace::new(), PositionConverter::new());
    let msg = json!({ "params": { "files": [{ "uri": dirty_uri }] } });
    mr.handle_file_event("workspace/didCreateFiles", &msg, &mut overlay_mgr, &mut conv);

    let doc = overlay_mgr.get(dirty_path).unwrap();
    assert!(doc.is_dirty());
    assert_eq!(doc.version, 2);
    assert_eq!(doc.text, "pub struct Modified;\n");
}

#[test]
fn test_did_create_files_multiple_events_idempotent() {
    let ts = create_temp_shard_with_symbol("p3_idempotent", "IdemControl");
    let mut mr = MultiRootWorkspace::new();
    assert!(mr.add_root(ts.shard.clone()));
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());

    let test_file = ts.dir.join("src/idem.rs");
    let msg = json!({ "params": { "files": [{ "uri": to_uri(&test_file) }] } });

    mr.handle_file_event("workspace/didCreateFiles", &msg, &mut overlay_mgr, &mut conv);
    mr.handle_file_event("workspace/didCreateFiles", &msg, &mut overlay_mgr, &mut conv);
    mr.handle_file_event("workspace/didCreateFiles", &msg, &mut overlay_mgr, &mut conv);

    assert_eq!(mr.len(), 1);
    assert_eq!(mr.first_workspace().unwrap().readers()[0].crate_name(), "p3_idempotent");
}

#[test]
fn test_did_create_files_non_utf8_path_handling() {
    let ts = create_temp_shard_with_symbol("p3_raw_enc", "RawEncControl");
    let mut mr = MultiRootWorkspace::new();
    assert!(mr.add_root(ts.shard.clone()));
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());

    let msg = json!({ "params": { "files": [{ "uri": "file:///tmp/%FF%FE%AA_invalid.rs" }] } });
    mr.handle_file_event("workspace/didCreateFiles", &msg, &mut overlay_mgr, &mut conv);

    assert_eq!(mr.len(), 1);
    assert_eq!(mr.first_workspace().unwrap().readers()[0].crate_name(), "p3_raw_enc");
}

#[test]
fn test_did_create_files_crlf_file_conv_invalidation() {
    let ts = create_temp_shard_with_symbol("p3_crlf", "CrlfControl");
    let mut mr = MultiRootWorkspace::new();
    assert!(mr.add_root(ts.shard.clone()));
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());

    let crlf_file = ts.dir.join("src/crlf.rs");
    std::fs::write(&crlf_file, "🦀abc\r\nsecond_line\r\n").unwrap();
    assert_eq!(conv.char_col_to_utf16_col(crlf_file.to_str().unwrap(), 1, 2), 2);

    std::fs::write(&crlf_file, "zabc\r\nsecond_line\r\n").unwrap();
    assert_eq!(conv.char_col_to_utf16_col(crlf_file.to_str().unwrap(), 1, 2), 2);

    let msg = json!({ "params": { "files": [{ "uri": to_uri(&crlf_file) }] } });
    mr.handle_file_event("workspace/didCreateFiles", &msg, &mut overlay_mgr, &mut conv);

    assert_eq!(conv.char_col_to_utf16_col(crlf_file.to_str().unwrap(), 1, 2), 1);
}

#[test]
fn test_did_create_files_case_sensitivity_path() {
    let ts = create_temp_shard_with_symbol("p3_case", "CaseControl");
    let mut mr = MultiRootWorkspace::new();
    assert!(mr.add_root(ts.shard.clone()));
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());

    let mixed_file = ts.dir.join("src/MySpecialHandler.rs");
    std::fs::write(&mixed_file, "🦀data").unwrap();
    assert_eq!(conv.char_col_to_utf16_col(mixed_file.to_str().unwrap(), 1, 2), 2);

    std::fs::write(&mixed_file, "adata").unwrap();
    let msg = json!({ "params": { "files": [{ "uri": to_uri(&mixed_file) }] } });
    mr.handle_file_event("workspace/didCreateFiles", &msg, &mut overlay_mgr, &mut conv);

    assert_eq!(conv.char_col_to_utf16_col(mixed_file.to_str().unwrap(), 1, 2), 1);
}

#[test]
fn test_did_create_files_with_custom_uri_scheme() {
    let ts = create_temp_shard_with_symbol("p3_scheme", "SchemeControl");
    let mut mr = MultiRootWorkspace::new();
    assert!(mr.add_root(ts.shard.clone()));
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());

    let msg = json!({ "params": { "files": [{ "uri": "vscode-vfs://remote/virtual/doc.rs" }] } });
    mr.handle_file_event("workspace/didCreateFiles", &msg, &mut overlay_mgr, &mut conv);

    assert_eq!(mr.len(), 1);
    assert_eq!(mr.first_workspace().unwrap().readers()[0].crate_name(), "p3_scheme");
}

#[test]
fn test_did_create_files_dot_slash_path_normalization() {
    let ts = create_temp_shard_with_symbol("p3_dotslash", "DotSlashControl");
    let mut mr = MultiRootWorkspace::new();
    assert!(mr.add_root(ts.shard.clone()));
    let (mut overlay_mgr, mut conv) = (OverlayManager::new(), PositionConverter::new());

    let msg = json!({ "params": { "files": [{ "uri": "file:///tmp/./a/../b/test.rs" }] } });
    mr.handle_file_event("workspace/didCreateFiles", &msg, &mut overlay_mgr, &mut conv);

    assert_eq!(mr.len(), 1);
    assert_eq!(mr.first_workspace().unwrap().readers()[0].crate_name(), "p3_dotslash");
}
