/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 3: `textDocument/didChange` synchronization tests (part 5: boundaries, caching, and fallback).

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::navigation::handle_definition;
use ridx::lsp::protocol::{uri_to_path, PositionConverter};
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use ridx_format::{DefKind, WorkspaceIndex};
use ridx_test_support::fixture::SyntheticWorkspaceFixture;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs;
use std::io::Cursor;
use std::sync::{Arc, Mutex};

fn setup_open_doc(uri: &str, initial_text: &str) -> (LifecycleManager, MultiRootWorkspace, OverlayManager) {
    let lifecycle = LifecycleManager::new();
    let multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let file = uri_to_path(uri);
    overlay_mgr.open_with_provenance(uri, &file, 1, initial_text.to_string(), true);
    (lifecycle, multi_root, overlay_mgr)
}

fn dispatch_did_change_with_converter(
    msg: Value,
    lifecycle: &mut LifecycleManager,
    multi_root: &mut MultiRootWorkspace,
    overlay_mgr: &mut OverlayManager,
    _converter: Option<&mut PositionConverter>,
) -> (bool, Vec<u8>) {
    let mut out = Cursor::new(Vec::new());
    let cancelled = Arc::new(Mutex::new(HashSet::new()));
    let res = dispatch_lsp_message(
        msg,
        &mut out,
        lifecycle,
        multi_root,
        overlay_mgr,
        &cancelled,
        None,
    );
    (res.unwrap_or(false), out.into_inner())
}

#[test]
fn test_did_change_out_of_bounds_range_clamped_safely() {
    let uri = "file:///tmp/oob_range.rs";
    let (mut lc, mut mr, mut om) = setup_open_doc(uri, "short");
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_change_with_converter(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": 2 },
                "contentChanges": [{
                    "range": {
                        "start": { "line": 100, "character": 50 },
                        "end": { "line": 200, "character": 80 }
                    },
                    "text": " app"
                }]
            }
        }),
        &mut lc, &mut mr, &mut om, None,
    );

    assert_eq!(ok, true);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.text, "short app");
}

#[test]
fn test_did_change_inverted_range_clamped_safely() {
    let uri = "file:///tmp/inverted_range.rs";
    let (mut lc, mut mr, mut om) = setup_open_doc(uri, "abcdef");
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_change_with_converter(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": 2 },
                "contentChanges": [{
                    "range": {
                        "start": { "line": 0, "character": 4 },
                        "end": { "line": 0, "character": 2 }
                    },
                    "text": "X"
                }]
            }
        }),
        &mut lc, &mut mr, &mut om, None,
    );

    assert_eq!(ok, true);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.text, "abcdXef");
}

#[test]
fn test_did_change_invalidates_position_converter_cache() {
    let temp_dir = std::env::temp_dir().join(format!("conv_change_{}", std::process::id()));
    let _ = fs::create_dir_all(&temp_dir);
    let path = temp_dir.join("conv.rs");
    let _ = fs::write(&path, "line1_initial\nline2\n");
    let uri = format!("file://{}", path.display());
    let file = uri_to_path(&uri);

    let (mut lc, mut mr, mut om) = setup_open_doc(&uri, "line1_initial\nline2\n");

    // Populate converter cache
    let cached = lc.conv.get_line(&file, 1);
    assert_eq!(cached, Some("line1_initial"));

    // Write updated content to disk behind converter cache
    let _ = fs::write(&path, "line1_disk_updated\nline2\n");
    assert_eq!(lc.conv.get_line(&file, 1), Some("line1_initial"));

    let (ok, _) = dispatch_did_change_with_converter(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": 2 },
                "contentChanges": [{ "text": "line1_overlay\n" }]
            }
        }),
        &mut lc, &mut mr, &mut om, None,
    );

    assert_eq!(ok, true);
    assert_eq!(lc.conv.get_line(&file, 1), Some("line1_disk_updated"));
    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_did_change_unopened_document_reads_from_disk_fallback() {
    let mut lc = LifecycleManager::new();
    let mut mr = MultiRootWorkspace::new();
    let mut om = OverlayManager::new();

    let path = std::env::temp_dir().join("disk_fallback_change.rs");
    let _ = fs::write(&path, "initial content\n");
    let uri = format!("file://{}", path.display());
    let file = uri_to_path(&uri);

    let (ok, _) = dispatch_did_change_with_converter(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": 2 },
                "contentChanges": [{
                    "range": {
                        "start": { "line": 0, "character": 0 },
                        "end": { "line": 0, "character": 7 }
                    },
                    "text": "updated"
                }]
            }
        }),
        &mut lc, &mut mr, &mut om, None,
    );

    let _ = fs::remove_file(&path);
    assert_eq!(ok, true);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.text, "updated content\n");
}

#[test]
fn test_did_change_unopened_document_missing_on_disk_uses_empty() {
    let mut lc = LifecycleManager::new();
    let mut mr = MultiRootWorkspace::new();
    let mut om = OverlayManager::new();

    let uri = "file:///tmp/nonexistent_change_doc.rs";
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_change_with_converter(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": 1 },
                "contentChanges": [{
                    "range": {
                        "start": { "line": 0, "character": 0 },
                        "end": { "line": 0, "character": 0 }
                    },
                    "text": "fresh text\n"
                }]
            }
        }),
        &mut lc, &mut mr, &mut om, None,
    );

    assert_eq!(ok, true);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.text, "fresh text\n");
}

#[test]
fn test_did_change_supports_new_text_key_variant() {
    let uri = "file:///tmp/new_text_key.rs";
    let (mut lc, mut mr, mut om) = setup_open_doc(uri, "aaa");
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_change_with_converter(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": 2 },
                "contentChanges": [{
                    "range": {
                        "start": { "line": 0, "character": 1 },
                        "end": { "line": 0, "character": 2 }
                    },
                    "newText": "Z"
                }]
            }
        }),
        &mut lc, &mut mr, &mut om, None,
    );

    assert_eq!(ok, true);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.text, "aZa");
}

#[test]
fn test_did_change_supports_text_key_variant() {
    let uri = "file:///tmp/text_key.rs";
    let (mut lc, mut mr, mut om) = setup_open_doc(uri, "aaa");
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_change_with_converter(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": 2 },
                "contentChanges": [{
                    "range": {
                        "start": { "line": 0, "character": 1 },
                        "end": { "line": 0, "character": 2 }
                    },
                    "text": "W"
                }]
            }
        }),
        &mut lc, &mut mr, &mut om, None,
    );

    assert_eq!(ok, true);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.text, "aWa");
}

#[test]
fn test_did_change_preserves_other_open_documents() {
    let uri1 = "file:///tmp/doc_one.rs";
    let uri2 = "file:///tmp/doc_two.rs";
    let (mut lc, mut mr, mut om) = setup_open_doc(uri1, "doc one text\n");
    let file2 = uri_to_path(uri2);
    om.open_with_provenance(uri2, &file2, 1, "doc two text\n".to_string(), true);

    let (ok, _) = dispatch_did_change_with_converter(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri1, "version": 2 },
                "contentChanges": [{ "text": "doc one modified\n" }]
            }
        }),
        &mut lc, &mut mr, &mut om, None,
    );

    assert_eq!(ok, true);
    let overlay2 = om.get(&file2).expect("doc2 must be untouched");
    assert_eq!(overlay2.text, "doc two text\n");
    assert_eq!(overlay2.version, 1);
}

#[test]
fn test_did_change_crlf_incremental_replacement() {
    let uri = "file:///tmp/crlf_inc.rs";
    let (mut lc, mut mr, mut om) = setup_open_doc(uri, "first\r\nsecond\r\n");
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_change_with_converter(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": 2 },
                "contentChanges": [{
                    "range": {
                        "start": { "line": 1, "character": 0 },
                        "end": { "line": 1, "character": 6 }
                    },
                    "text": "modified"
                }]
            }
        }),
        &mut lc, &mut mr, &mut om, None,
    );

    assert_eq!(ok, true);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.text, "first\r\nmodified\r\n");
}

#[test]
fn test_did_change_short_overlay_definition_on_past_eof_returns_null() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_trunc", "0.1.0").unwrap();

    let mut source_lines = String::new();
    for i in 0..50 {
        source_lines.push_str(&format!("// line {i}\n"));
    }
    source_lines.push_str("pub struct «TruncatedTarget»;\n");

    ws.add_source_file_with_defs("pkg_trunc", "src/lib.rs", &source_lines, DefKind::Struct)
        .unwrap();

    let shard = ws.build_crate_index("pkg_trunc").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let lib_uri = ws.file_uri("pkg_trunc", "src/lib.rs").unwrap();
    let lib_path = ws.file_path("pkg_trunc", "src/lib.rs").unwrap();
    let lib_str = lib_path.to_str().unwrap();

    let mut lc = LifecycleManager::new();
    let mut mr = MultiRootWorkspace::new();
    let mut om = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // 1. Initially open with the full 51-line text (clean overlay)
    om.open_with_provenance(&lib_uri, lib_str, 1, source_lines.clone(), true);

    let def_req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "textDocument/definition",
        "params": {
            "textDocument": { "uri": lib_uri },
            "position": { "line": 50, "character": 12 }
        }
    });

    // Clean overlay finds the definition at line 50
    let res_clean = handle_definition(&def_req, Some(&index), &om, &mut conv);
    assert_eq!(res_clean["uri"], lib_uri);
    assert_eq!(res_clean["range"]["start"]["line"], 50);

    // 2. didChange: document truncated to 1 line, becoming dirty
    let (ok, _) = dispatch_did_change_with_converter(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": lib_uri, "version": 2 },
                "contentChanges": [{ "text": "// truncated\n" }]
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
        Some(&mut conv),
    );
    assert_eq!(ok, true);

    // 3. Request definition at line 50 (which is now past EOF in live overlay)
    let res_dirty = handle_definition(&def_req, Some(&index), &om, &mut conv);
    assert_eq!(res_dirty, Value::Null);
}

