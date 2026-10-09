/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Domain 3: Document content modification tests (`didChange` full and incremental edits,
//! batch ordering, version sequencing, and implicit open).

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::path_to_uri;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::{DocumentOverlay, OverlayManager};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

struct CleanupDir(PathBuf);
impl Drop for CleanupDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

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
fn test_did_change_full_content_sync() {
    let mut overlay =
        DocumentOverlay::new("file:///test.rs", "/test.rs", 1, "initial text".to_string());
    assert_eq!(overlay.version, 1);
    assert_eq!(overlay.text, "initial text");

    let changes = vec![json!({ "text": "fully replaced text" })];
    overlay.apply_content_changes(2, &changes);

    assert_eq!(overlay.version, 2);
    assert_eq!(overlay.text, "fully replaced text");
}

#[test]
fn test_did_change_incremental_single_line_insert() {
    let initial = "fn main() {\n    let a = 1;\n}\n";
    let mut overlay = DocumentOverlay::new("file:///test.rs", "/test.rs", 1, initial.to_string());

    // Insert `let b = 2;\n    ` before `let a` on line 1, char 4
    let changes = vec![json!({
        "range": {
            "start": { "line": 1, "character": 4 },
            "end": { "line": 1, "character": 4 }
        },
        "text": "let b = 2;\n    "
    })];
    overlay.apply_content_changes(2, &changes);

    let expected = "fn main() {\n    let b = 2;\n    let a = 1;\n}\n";
    assert_eq!(overlay.text, expected);
    assert_eq!(overlay.line_starts.len(), 5);
}

#[test]
fn test_did_change_incremental_single_line_delete() {
    let initial = "fn main() {\n    let a = 1;\n    let b = 2;\n}\n";
    let mut overlay = DocumentOverlay::new("file:///test.rs", "/test.rs", 1, initial.to_string());

    // Delete `let a = 1;\n    ` on line 1, char 4 to line 2, char 4
    let changes = vec![json!({
        "range": {
            "start": { "line": 1, "character": 4 },
            "end": { "line": 2, "character": 4 }
        },
        "text": ""
    })];
    overlay.apply_content_changes(2, &changes);

    let expected = "fn main() {\n    let b = 2;\n}\n";
    assert_eq!(overlay.text, expected);
}

#[test]
fn test_did_change_incremental_multiline_insert() {
    let initial = "fn run() {\n}\n";
    let mut overlay = DocumentOverlay::new("file:///test.rs", "/test.rs", 1, initial.to_string());

    let inserted = "    let x = 1;\n    let y = 2;\n    let z = 3;\n";
    let changes = vec![json!({
        "range": {
            "start": { "line": 1, "character": 0 },
            "end": { "line": 1, "character": 0 }
        },
        "text": inserted
    })];
    overlay.apply_content_changes(2, &changes);

    let expected = format!("fn run() {{\n{}}}\n", inserted);
    assert_eq!(overlay.text, expected);
    assert_eq!(overlay.line_starts.len(), 6);
}

#[test]
fn test_did_change_incremental_multiline_delete() {
    let initial = "line 0\nline 1\nline 2\nline 3\nline 4\n";
    let mut overlay = DocumentOverlay::new("file:///test.rs", "/test.rs", 1, initial.to_string());

    // Delete line 1 through line 3 (from start of line 1 to start of line 4)
    let changes = vec![json!({
        "range": {
            "start": { "line": 1, "character": 0 },
            "end": { "line": 4, "character": 0 }
        },
        "text": ""
    })];
    overlay.apply_content_changes(2, &changes);

    let expected = "line 0\nline 4\n";
    assert_eq!(overlay.text, expected);
    assert_eq!(overlay.line_starts.len(), 3);
}

#[test]
fn test_did_change_multi_change_batch_in_order() {
    let initial = "alpha beta gamma\n";
    let mut overlay = DocumentOverlay::new("file:///test.rs", "/test.rs", 1, initial.to_string());

    // In a single didChange message, apply 3 incremental edits sequentially:
    // 1. Replace "alpha" with "first" at (0, 0)-(0, 5) -> "first beta gamma\n"
    // 2. Replace "beta" with "second" at (0, 6)-(0, 10) -> "first second gamma\n"
    // 3. Append " delta" at end (0, 18)-(0, 18) -> "first second gamma delta\n"
    let changes = vec![
        json!({
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 5 } },
            "text": "first"
        }),
        json!({
            "range": { "start": { "line": 0, "character": 6 }, "end": { "line": 0, "character": 10 } },
            "text": "second"
        }),
        json!({
            "range": { "start": { "line": 0, "character": 18 }, "end": { "line": 0, "character": 18 } },
            "text": " delta"
        }),
    ];
    overlay.apply_content_changes(2, &changes);

    assert_eq!(overlay.text, "first second gamma delta\n");
}

#[test]
fn test_did_change_version_monotonic_increments() {
    let mut overlay = DocumentOverlay::new("file:///test.rs", "/test.rs", 1, "v1\n".to_string());
    assert_eq!(overlay.version, 1);

    overlay.apply_content_changes(2, &[json!({ "text": "v2\n" })]);
    assert_eq!(overlay.version, 2);

    overlay.apply_content_changes(3, &[json!({ "text": "v3\n" })]);
    assert_eq!(overlay.version, 3);
}

#[test]
fn test_did_change_version_skip_tolerance() {
    let mut overlay = DocumentOverlay::new("file:///test.rs", "/test.rs", 1, "v1\n".to_string());

    // Simulating dropped frames/notifications: jumps directly from v1 to v10
    overlay.apply_content_changes(10, &[json!({ "text": "v10\n" })]);
    assert_eq!(overlay.version, 10);
    assert_eq!(overlay.text, "v10\n");
}

#[test]
fn test_did_change_version_duplicate_or_zero() {
    let mut overlay = DocumentOverlay::new("file:///test.rs", "/test.rs", 5, "v5\n".to_string());

    // Version 0 or non-positive triggers fallback increment
    overlay.apply_content_changes(0, &[json!({ "text": "v6\n" })]);
    assert_eq!(overlay.version, 6);

    // Repeated version 6
    overlay.apply_content_changes(6, &[json!({ "text": "v6_updated\n" })]);
    assert_eq!(overlay.version, 6);
    assert_eq!(overlay.text, "v6_updated\n");
}

#[test]
fn test_did_change_unopened_document_implicit_open() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_implicit_{}", std::process::id()));
    let _ = fs::create_dir_all(&temp_dir);
    let _guard = CleanupDir(temp_dir.clone());

    let file_path = temp_dir.join("implicit.rs");
    fs::write(&file_path, "fn from_disk() {}\n").unwrap();
    let uri = path_to_uri(file_path.to_str().unwrap());

    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();

    // Client sends didChange without prior didOpen
    dispatch_msg(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": &uri, "version": 1 },
                "contentChanges": [{ "text": "fn from_implicit_open() {}\n" }]
            }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
    );

    let file_str = file_path.to_str().unwrap();
    let ov = overlay_mgr
        .get(file_str)
        .expect("overlay must be opened implicitly");
    assert_eq!(ov.text, "fn from_implicit_open() {}\n");
    assert_eq!(ov.version, 1);
}

#[test]
fn test_did_change_reparses_symbols_and_scopes() {
    let mut overlay = DocumentOverlay::new(
        "file:///test.rs",
        "/test.rs",
        1,
        "fn first() {}\n".to_string(),
    );
    assert_eq!(overlay.symbols.len(), 1);
    assert_eq!(overlay.symbols[0].name, "first");

    let updated = "fn first() {}\npub fn second() -> u32 { 42 }\n";
    overlay.apply_content_changes(2, &[json!({ "text": updated })]);

    assert_eq!(overlay.symbols.len(), 2);
    let names: Vec<&str> = overlay.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"first"));
    assert!(names.contains(&"second"));
}
