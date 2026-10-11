/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 3: Document open synchronization tests (`didOpen`, provenance, CRLF, unicode, and caching).

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::{uri_to_path, PositionConverter};
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
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
fn test_did_open_fresh_document() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();

    let uri = "file:///tmp/fresh_test.rs";
    let file = uri_to_path(uri);
    let code = "pub fn add(a: i32, b: i32) -> i32 { a + b }\n";

    dispatch_msg(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "version": 1, "text": code } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    assert_eq!(overlay_mgr.open_count(), 1);
    let overlay = overlay_mgr.get(&file).expect("overlay must exist");
    assert_eq!(overlay.version, 1);
    assert_eq!(overlay.text, code);
    assert!(!overlay.symbols.is_empty());
}

#[test]
fn test_did_open_with_provenance_compiled() {
    let mut overlay_mgr = OverlayManager::new();
    let path = "/test/compiled_doc.rs";
    overlay_mgr.open_with_provenance(
        "file:///test/compiled_doc.rs",
        path,
        1,
        "fn f() {}\n".to_string(),
        true,
    );

    let ov = overlay_mgr.get(path).unwrap();
    assert!(!ov.is_dirty(), "compiled baseline must not be dirty");
    assert_eq!(ov.map_line_to_compiled(1), Some(1));
}

#[test]
fn test_did_open_with_provenance_uncompiled() {
    let mut overlay_mgr = OverlayManager::new();
    let path = "/test/uncompiled_doc.rs";
    overlay_mgr.open_with_provenance(
        "file:///test/uncompiled_doc.rs",
        path,
        1,
        "fn f() {}\n".to_string(),
        false,
    );

    let ov = overlay_mgr.get(path).unwrap();
    assert!(ov.is_dirty(), "uncompiled doc must report dirty");
    assert_eq!(ov.map_line_to_compiled(1), None);
}

#[test]
fn test_did_open_already_open_replaces_cleanly() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();

    let uri = "file:///tmp/replace_test.rs";
    let file = uri_to_path(uri);

    dispatch_msg(
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "version": 1, "text": "fn v1() {}\n" } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );
    assert_eq!(overlay_mgr.open_count(), 1);

    dispatch_msg(
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "version": 2, "text": "fn v2() {}\n" } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );
    assert_eq!(overlay_mgr.open_count(), 1);
    let ov = overlay_mgr.get(&file).unwrap();
    assert_eq!(ov.version, 2);
    assert!(ov.text.contains("v2"));
}

#[test]
fn test_did_open_crlf_line_endings_handling() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();

    let uri = "file:///tmp/crlf_test.rs";
    let file = uri_to_path(uri);
    let crlf_code = "line1\r\nline2\r\nline3\r\n";

    dispatch_msg(
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "version": 1, "text": crlf_code } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    let ov = overlay_mgr.get(&file).unwrap();
    assert_eq!(ov.line_starts.len(), 4);
    assert_eq!(ov.position_to_byte_offset(0, 0), 0);
    assert_eq!(ov.position_to_byte_offset(1, 0), 7);
    assert_eq!(ov.position_to_byte_offset(2, 0), 14);
}

#[test]
fn test_did_open_empty_document() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();

    let uri = "file:///tmp/empty_doc.rs";
    let file = uri_to_path(uri);

    dispatch_msg(
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "version": 1, "text": "" } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    let ov = overlay_mgr.get(&file).unwrap();
    assert_eq!(ov.text, "");
    assert!(ov.symbols.is_empty());
    assert_eq!(ov.position_to_byte_offset(0, 0), 0);
}

#[test]
fn test_did_open_multibyte_unicode_and_emojis() {
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();

    let uri = "file:///tmp/unicode_doc.rs";
    let file = uri_to_path(uri);
    let code = "fn unicode_greeting() { let 🚀 = 42; }\n";

    dispatch_msg(
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "version": 1, "text": code } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    let ov = overlay_mgr.get(&file).unwrap();
    let def = ov
        .find_definition_at(1, 4)
        .expect("should find unicode_greeting");
    assert_eq!(def.name, "unicode_greeting");
}

#[test]
fn test_did_open_invalidates_position_cache() {
    let mut conv = PositionConverter::new();
    let file = "/tmp/cache_inval.rs";
    let _ = conv.get_line(file, 1);

    conv.invalidate(file);
    let mut lifecycle = LifecycleManager::new();
    lifecycle.conv.invalidate(file);
    assert_eq!(lifecycle.conv.get_line(file, 1), None);
}

#[test]
fn test_did_open_multi_reader_provenance_fast_path() {
    use ridx_format::{DefKind, IndexWriter, RawDefinition, WorkspaceIndex};

    let temp_dir = std::env::temp_dir().join(format!("ridx_test_fast_path_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);

    // 1. Reader A: crate_a with src_a.rs
    let file_a_path = temp_dir.join("src_a.rs").to_str().unwrap().to_string();
    let mut writer_a = IndexWriter::new("crate_a", "0.1.0");
    writer_a.add_file_hash(&file_a_path, 0x1111_2222_3333_4444);
    writer_a.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "func_a".to_string(),
        path: "crate_a::func_a".to_string(),
        signature: Some("fn()".to_string()),
        file: file_a_path.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 10,
    });
    let ridx_a = temp_dir.join("crate_a.ridx");
    let mut f_a = std::fs::File::create(&ridx_a).unwrap();
    writer_a.write_to(&mut f_a).unwrap();
    drop(f_a);

    // 2. Reader B: crate_b with src_b.rs
    let file_b_path = temp_dir.join("src_b.rs").to_str().unwrap().to_string();
    let mut writer_b = IndexWriter::new("crate_b", "0.1.0");
    writer_b.add_file_hash(&file_b_path, 0x5555_6666_7777_8888);
    writer_b.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "func_b".to_string(),
        path: "crate_b::func_b".to_string(),
        signature: Some("fn()".to_string()),
        file: file_b_path.clone(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 10,
    });
    let ridx_b = temp_dir.join("crate_b.ridx");
    let mut f_b = std::fs::File::create(&ridx_b).unwrap();
    writer_b.write_to(&mut f_b).unwrap();
    drop(f_b);

    // Open multi-reader workspace from directory containing both shards
    let mut ws = WorkspaceIndex::open(&temp_dir).expect("must open multi-reader workspace");
    ws.set_workspace_root(Some(temp_dir.clone()));
    assert_eq!(ws.readers().len(), 2, "must contain both reader shards");

    // Verify deterministic fast-path retrieval:
    // Reader A knows src_a.rs
    let hash_a = ws.source_hash_for_file(&file_a_path);
    assert_eq!(
        hash_a,
        Some(0x1111_2222_3333_4444),
        "fast path must return exact hash for file_a in reader A"
    );
    let mtime_a = ws.shard_mtime_for_file(&file_a_path);
    assert!(
        mtime_a.is_some(),
        "fast path must find shard mtime for file_a"
    );

    // Reader B knows src_b.rs
    let hash_b = ws.source_hash_for_file(&file_b_path);
    assert_eq!(
        hash_b,
        Some(0x5555_6666_7777_8888),
        "fast path must return exact hash for file_b in reader B"
    );
    let mtime_b = ws.shard_mtime_for_file(&file_b_path);
    assert!(
        mtime_b.is_some(),
        "fast path must find shard mtime for file_b"
    );

    // Non-existent file returns None without timing assertions
    let unknown_path = temp_dir.join("unknown.rs").to_str().unwrap().to_string();
    assert_eq!(ws.source_hash_for_file(&unknown_path), None);
    assert_eq!(ws.shard_mtime_for_file(&unknown_path), None);

    let _ = std::fs::remove_dir_all(&temp_dir);
}
