/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! E2E regression tests for independent `compiled_text` and `disk_text` baselines on `textDocument/didOpen`.

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::path_to_uri;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use ridx_format::{normalized_source_hash, IndexWriter};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::Cursor;
use std::sync::{Arc, Mutex};

fn dispatch_msg(
    msg: Value,
    lifecycle: &mut LifecycleManager,
    multi_root: &mut MultiRootWorkspace,
    overlay_mgr: &mut OverlayManager,
) {
    let mut out = Cursor::new(Vec::new());
    let cancelled = Arc::new(Mutex::new(HashSet::new()));
    let _ = dispatch_lsp_message(
        msg,
        &mut out,
        lifecycle,
        multi_root,
        overlay_mgr,
        &cancelled,
        None,
    );
}

#[test]
fn test_did_open_buffer_matches_index_while_disk_diverged_preserves_disk_text() {
    let temp_dir =
        std::env::temp_dir().join(format!("ridx_did_open_buf_a_disk_b_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let src_file = temp_dir.join("lib.rs");
    let file_str = src_file.to_str().unwrap().to_string();
    let text_a = "pub fn a() {}\n";
    let text_b = "pub fn b() {}\n";

    std::fs::write(&src_file, text_b).unwrap();

    let mut writer = IndexWriter::new("baseline_crate", "0.1.0");
    writer.get_or_insert_file(&file_str);
    writer.add_file_hash(&file_str, normalized_source_hash(text_a.as_bytes()));
    writer
        .write_to(&mut std::fs::File::create(temp_dir.join("baseline_crate.ridx")).unwrap())
        .unwrap();

    let mut lifecycle = LifecycleManager::new();
    let _ = lifecycle.handle_initialize(&json!({ "params": {} }));
    let mut multi_root = MultiRootWorkspace::new();
    assert!(multi_root.add_root(temp_dir.clone()));
    let mut overlay_mgr = OverlayManager::new();
    let uri = path_to_uri(&file_str);

    dispatch_msg(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": &uri,
                    "version": 1,
                    "text": text_a
                }
            }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    let ov = overlay_mgr.get(&file_str).expect("overlay must exist");
    let actual = (
        ov.compiled_text.clone(),
        ov.disk_text.clone(),
        ov.is_synced_with_disk(),
    );
    let _ = std::fs::remove_dir_all(&temp_dir);

    assert_eq!(
        (actual.0.as_deref(), actual.1.as_deref()),
        (Some(text_a), Some(text_b)),
        "index=A, disk=B, buffer=A must yield (compiled_text=Some(A), disk_text=Some(B))"
    );
    assert!(
        !actual.2,
        "buffer=A must not be reported synced with disk=B"
    );
}

#[test]
fn test_did_open_stale_index_buffer_matches_disk_preserves_disk_text() {
    let temp_dir =
        std::env::temp_dir().join(format!("ridx_did_open_buf_b_disk_b_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let src_file = temp_dir.join("lib.rs");
    let file_str = src_file.to_str().unwrap().to_string();
    let text_a = "pub fn a() {}\n";
    let text_b = "pub fn b() {}\n";

    std::fs::write(&src_file, text_b).unwrap();

    let mut writer = IndexWriter::new("stale_index_crate", "0.1.0");
    writer.get_or_insert_file(&file_str);
    writer.add_file_hash(&file_str, normalized_source_hash(text_a.as_bytes()));
    writer
        .write_to(&mut std::fs::File::create(temp_dir.join("stale_index_crate.ridx")).unwrap())
        .unwrap();

    let mut lifecycle = LifecycleManager::new();
    let _ = lifecycle.handle_initialize(&json!({ "params": {} }));
    let mut multi_root = MultiRootWorkspace::new();
    assert!(multi_root.add_root(temp_dir.clone()));
    let mut overlay_mgr = OverlayManager::new();
    let uri = path_to_uri(&file_str);

    dispatch_msg(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": &uri,
                    "version": 1,
                    "text": text_b
                }
            }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    let ov = overlay_mgr.get(&file_str).expect("overlay must exist");
    let actual = (
        ov.compiled_text.clone(),
        ov.disk_text.clone(),
        ov.is_synced_with_disk(),
    );
    let _ = std::fs::remove_dir_all(&temp_dir);

    assert_eq!(
        (actual.0.as_deref(), actual.1.as_deref()),
        (None, Some(text_b)),
        "index=A, disk=B, buffer=B must yield (compiled_text=None, disk_text=Some(B))"
    );
    assert!(actual.2, "buffer=B must be reported synced with disk=B");
}

#[test]
fn test_reopen_without_close_shifted_buffer_preserves_compiled_and_disk_text() {
    let temp_dir =
        std::env::temp_dir().join(format!("ridx_reopen_shifted_buf_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let src_file = temp_dir.join("caller.rs");
    let target_file = temp_dir.join("target.rs");
    let file_str = src_file.to_str().unwrap().to_string();
    let target_str = target_file.to_str().unwrap().to_string();

    let text_a = "pub fn run() -> i32 {\n    compute_answer()\n}\n";
    let target_text = "pub fn compute_answer() -> i32 {\n    42\n}\n";
    std::fs::write(&src_file, text_a).unwrap();
    std::fs::write(&target_file, target_text).unwrap();

    let mut writer = IndexWriter::new("reopen_crate", "0.1.0");
    writer.add_file_hash(&file_str, normalized_source_hash(text_a.as_bytes()));
    writer.add_file_hash(&target_str, normalized_source_hash(target_text.as_bytes()));
    writer.add_definition(ridx_format::RawDefinition {
        def_id: 1,
        kind: ridx_format::DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "compute_answer".to_string(),
        path: "reopen_crate::compute_answer".to_string(),
        signature: Some("pub fn compute_answer() -> i32".to_string()),
        file: target_str.clone(),
        start_line: 1,
        start_col: 8,
        end_line: 1,
        end_col: 22,
    });
    writer.add_reference(ridx_format::RawReference {
        file: file_str.clone(),
        start_line: 2,
        start_col: 5,
        end_line: 2,
        end_col: 19,
        target_def_id: Some(1),
        target_path: "reopen_crate::compute_answer".to_string(),
        target_file: Some(target_str.clone()),
        target_unit: Some("reopen_crate".to_string()),
        target_line: 1,
        target_col: 8,
        kind: ridx_format::DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer
        .write_to(&mut std::fs::File::create(temp_dir.join("reopen_crate.ridx")).unwrap())
        .unwrap();

    let mut lifecycle = LifecycleManager::new();
    let _ = lifecycle.handle_initialize(&json!({ "params": {} }));
    let mut multi_root = MultiRootWorkspace::new();
    assert!(multi_root.add_root(temp_dir.clone()));
    let mut overlay_mgr = OverlayManager::new();
    let uri = path_to_uri(&file_str);

    dispatch_msg(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": { "uri": &uri, "version": 1, "text": text_a }
            }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    let shifted_a = format!("{}{}", "\n".repeat(10), text_a);
    dispatch_msg(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": { "uri": &uri, "version": 2, "text": &shifted_a }
            }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    let ov = overlay_mgr.get(&file_str).expect("overlay must exist");
    let actual_baselines = (
        ov.compiled_text.clone(),
        ov.disk_text.clone(),
        ov.is_synced_with_disk(),
    );

    let mut out = Cursor::new(Vec::new());
    let cancelled = Arc::new(Mutex::new(HashSet::new()));
    let _ = dispatch_lsp_message(
        json!({
            "jsonrpc": "2.0",
            "id": 201,
            "method": "textDocument/definition",
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 11, "character": 4 }
            }
        }),
        &mut out,
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        None,
    );
    let raw = String::from_utf8(out.into_inner()).unwrap();
    let body = raw.split("\r\n\r\n").nth(1).expect("LSP response body");
    let resp: Value = serde_json::from_str(body).unwrap();
    let _ = std::fs::remove_dir_all(&temp_dir);

    assert_eq!(
        (actual_baselines.0.as_deref(), actual_baselines.1.as_deref()),
        (Some(text_a), Some(text_a)),
        "repeated didOpen with +10 shifted buffer while index=A, disk=A must yield (Some(A), Some(A))"
    );
    assert!(
        !actual_baselines.2,
        "shifted buffer must not be synced with disk=A"
    );
    assert_eq!(
        resp["result"],
        json!({
            "uri": path_to_uri(&target_str),
            "range": {
                "start": { "line": 0, "character": 7 },
                "end": { "line": 0, "character": 21 }
            }
        })
    );
}

#[test]
fn test_reopen_without_close_refreshes_disk_text_from_disk() {
    let temp_dir =
        std::env::temp_dir().join(format!("ridx_reopen_disk_refresh_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let src_file = temp_dir.join("lib.rs");
    let file_str = src_file.to_str().unwrap().to_string();
    let text_a = "pub fn a() {}\n";
    let text_b = "pub fn b() {}\n";

    std::fs::write(&src_file, text_a).unwrap();

    let mut writer = IndexWriter::new("reopen_refresh_crate", "0.1.0");
    writer.get_or_insert_file(&file_str);
    writer.add_file_hash(&file_str, normalized_source_hash(text_a.as_bytes()));
    writer
        .write_to(&mut std::fs::File::create(temp_dir.join("reopen_refresh_crate.ridx")).unwrap())
        .unwrap();

    let mut lifecycle = LifecycleManager::new();
    let _ = lifecycle.handle_initialize(&json!({ "params": {} }));
    let mut multi_root = MultiRootWorkspace::new();
    assert!(multi_root.add_root(temp_dir.clone()));
    let mut overlay_mgr = OverlayManager::new();
    let uri = path_to_uri(&file_str);

    dispatch_msg(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": { "uri": &uri, "version": 1, "text": text_a }
            }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    std::fs::write(&src_file, text_b).unwrap();
    dispatch_msg(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": { "uri": &uri, "version": 2, "text": text_a }
            }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    let ov = overlay_mgr.get(&file_str).expect("overlay must exist");
    let actual = (
        ov.compiled_text.clone(),
        ov.disk_text.clone(),
        ov.is_synced_with_disk(),
    );
    let _ = std::fs::remove_dir_all(&temp_dir);

    assert_eq!(
        (actual.0.as_deref(), actual.1.as_deref()),
        (Some(text_a), Some(text_b)),
        "repeated didOpen must refresh disk_text from disk (Some(B)) while keeping compiled_text=Some(A)"
    );
    assert!(
        !actual.2,
        "buffer=A must not be reported synced after disk changed to B"
    );
}
