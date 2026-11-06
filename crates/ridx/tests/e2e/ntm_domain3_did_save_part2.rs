/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 3: `textDocument/didSave` synchronization tests (part 2: text params, flycheck, and baselines).

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::{uri_to_path, PositionConverter};
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs;
use std::io::Cursor;
use std::sync::{Arc, Mutex};

fn setup_server_with_doc(uri: &str, text: &str) -> (LifecycleManager, MultiRootWorkspace, OverlayManager) {
    let lc = LifecycleManager::new();
    let mr = MultiRootWorkspace::new();
    let mut om = OverlayManager::new();
    let file = uri_to_path(uri);
    om.open_with_provenance(uri, &file, 1, text.to_string(), true);
    (lc, mr, om)
}

fn dispatch_did_save_with_converter(
    msg: Value,
    lifecycle: &mut LifecycleManager,
    multi_root: &mut MultiRootWorkspace,
    overlay_mgr: &mut OverlayManager,
    _converter: Option<&mut PositionConverter>,
) -> (bool, Vec<u8>) {
    let mut out = Cursor::new(Vec::new());
    let cancelled = Arc::new(Mutex::new(HashSet::new()));
    let res = dispatch_lsp_message(msg, &mut out, lifecycle, multi_root, overlay_mgr, &cancelled, None);
    (res.unwrap_or(false), out.into_inner())
}

#[test]
fn test_did_save_with_included_text_param_safe() {
    let uri = "file:///tmp/save_with_text.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn f() {}\n");
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_save_with_converter(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didSave",
            "params": {
                "textDocument": { "uri": uri },
                "text": "fn f_saved() {}\n"
            }
        }),
        &mut lc,
        &mut mr,
        &mut om,
        None,
    );

    assert!(ok);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.disk_text, Some("fn f() {}\n".to_string()));
}

#[test]
fn test_did_save_triggers_flycheck_when_cargo_toml_present() {
    let temp_dir = std::env::temp_dir().join("did_save_fc_test");
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let _ = fs::write(temp_dir.join("Cargo.toml"), "[package]\nname = \"fc_test\"\n");
    let file_path = temp_dir.join("src").join("lib.rs");
    let uri = format!("file://{}", file_path.display());

    let (mut lc, mut mr, mut om) = setup_server_with_doc(&uri, "pub fn check_me() {}\n");

    let (ok, _) = dispatch_did_save_with_converter(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didSave",
            "params": { "textDocument": { "uri": uri } }
        }),
        &mut lc,
        &mut mr,
        &mut om,
        None,
    );

    let _ = fs::remove_dir_all(&temp_dir);
    assert!(ok);
    assert_eq!(lc.flycheck.current_generation(), 1);
}

#[test]
fn test_did_save_does_not_trigger_flycheck_when_no_cargo_toml() {
    let uri = "file:///tmp/no_cargo_toml_save/lib.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "fn no_fc() {}\n");

    let (ok, _) = dispatch_did_save_with_converter(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didSave",
            "params": { "textDocument": { "uri": uri } }
        }),
        &mut lc,
        &mut mr,
        &mut om,
        None,
    );

    assert!(ok);
    assert_eq!(lc.flycheck.current_generation(), 0);
}

#[test]
fn test_did_save_multiple_successive_saves_on_same_file() {
    let uri = "file:///tmp/multi_save.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "version 1\n");
    let file = uri_to_path(uri);

    for _i in 1..=3 {
        let (ok, _) = dispatch_did_save_with_converter(
            json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didSave",
                "params": { "textDocument": { "uri": uri } }
            }),
            &mut lc,
            &mut mr,
            &mut om,
            None,
        );
        assert!(ok);
        let overlay = om.get(&file).expect("overlay must exist");
        assert_eq!(overlay.disk_text, Some("version 1\n".to_string()));
    }
}

#[test]
fn test_did_save_after_did_change_updates_disk_baseline() {
    let uri = "file:///tmp/change_then_save.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "v1 text\n");
    let file = uri_to_path(uri);

    // Apply incremental change
    om.apply_content_changes(&file, 2, &[json!({ "text": "v2 saved text\n" })]);
    assert_eq!(om.get(&file).unwrap().disk_text, None);

    let (ok, _) = dispatch_did_save_with_converter(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didSave",
            "params": { "textDocument": { "uri": uri } }
        }),
        &mut lc,
        &mut mr,
        &mut om,
        None,
    );

    assert!(ok);
    assert_eq!(om.get(&file).unwrap().disk_text, Some("v2 saved text\n".to_string()));
}

#[test]
fn test_did_save_preserves_line_starts() {
    let uri = "file:///tmp/ls_save.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "a\nb\nc\n");
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_save_with_converter(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didSave",
            "params": { "textDocument": { "uri": uri } }
        }),
        &mut lc,
        &mut mr,
        &mut om,
        None,
    );

    assert!(ok);
    let overlay = om.get(&file).expect("overlay must exist");
    assert_eq!(overlay.line_starts, vec![0, 2, 4, 6]);
}

#[test]
fn test_did_save_preserves_other_open_documents() {
    let uri1 = "file:///tmp/save_doc1.rs";
    let uri2 = "file:///tmp/save_doc2.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri1, "doc 1\n");
    let file2 = uri_to_path(uri2);
    om.open_with_provenance(uri2, &file2, 1, "doc 2\n".to_string(), true);
    om.apply_content_changes(&file2, 2, &[json!({ "text": "doc 2 dirty\n" })]);

    let (ok, _) = dispatch_did_save_with_converter(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didSave",
            "params": { "textDocument": { "uri": uri1 } }
        }),
        &mut lc,
        &mut mr,
        &mut om,
        None,
    );

    assert!(ok);
    let overlay2 = om.get(&file2).expect("overlay2 must exist");
    // doc2 was not saved, so its disk_text is still None
    assert_eq!(overlay2.disk_text, None);
    assert_eq!(overlay2.text, "doc 2 dirty\n");
}

#[test]
fn test_did_save_invalidates_position_converter_cache() {
    let temp_dir = std::env::temp_dir().join(format!("conv_save_{}", std::process::id()));
    let _ = fs::create_dir_all(&temp_dir);
    let path = temp_dir.join("conv.rs");
    let _ = fs::write(&path, "line1_initial\n");
    let uri = format!("file://{}", path.display());
    let (mut lc, mut mr, mut om) = setup_server_with_doc(&uri, "line1_initial\n");
    let file = uri_to_path(&uri);

    // Warm up converter cache
    let cached = lc.conv.get_line(&file, 1);
    assert_eq!(cached, Some("line1_initial"));

    // Write updated content to disk behind converter cache
    let _ = fs::write(&path, "line1_disk_updated\n");
    assert_eq!(lc.conv.get_line(&file, 1), Some("line1_initial"));

    let (ok, _) = dispatch_did_save_with_converter(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didSave",
            "params": { "textDocument": { "uri": uri } }
        }),
        &mut lc,
        &mut mr,
        &mut om,
        None,
    );

    assert!(ok);
    assert_eq!(lc.conv.get_line(&file, 1), Some("line1_disk_updated"));
    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_did_save_does_not_evict_overlay() {
    let uri = "file:///tmp/no_evict_save.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "stay open\n");
    let file = uri_to_path(uri);

    let (ok, _) = dispatch_did_save_with_converter(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didSave",
            "params": { "textDocument": { "uri": uri } }
        }),
        &mut lc,
        &mut mr,
        &mut om,
        None,
    );

    assert!(ok);
    assert_eq!(om.open_count(), 1);
    assert_eq!(om.get(&file).map(|d| d.text.as_str()), Some("stay open\n"));
}

#[test]
fn test_did_save_on_already_saved_document_is_idempotent() {
    let uri = "file:///tmp/idempotent_save.rs";
    let (mut lc, mut mr, mut om) = setup_server_with_doc(uri, "clean text\n");
    let file = uri_to_path(uri);

    // Save once
    let _ = dispatch_did_save_with_converter(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didSave", "params": { "textDocument": { "uri": uri } } }),
        &mut lc,
        &mut mr,
        &mut om,
        None,
    );
    let text1 = om.get(&file).unwrap().disk_text.clone();

    // Save second time
    let (ok, _) = dispatch_did_save_with_converter(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didSave", "params": { "textDocument": { "uri": uri } } }),
        &mut lc,
        &mut mr,
        &mut om,
        None,
    );

    assert!(ok);
    let text2 = om.get(&file).unwrap().disk_text.clone();
    assert_eq!(text1, text2);
}
