/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Compiler diagnostics are placed in the file as saved for Zed, in the edited buffer for other clients.

use super::compiler_merge::set_client_moves_rustc;
use super::diagnostics::handle_diagnostic_with_flycheck;
use super::flycheck::FlycheckState;
use super::workspace_diag::handle_workspace_diagnostic_with_flycheck;
use crate::lsp::protocol::path_to_uri;
use crate::lsp::workspace::MultiRootWorkspace;
use crate::overlay::OverlayManager;
use serde_json::{json, Value};

const CHECKED: &str = "fn total() -> u64 {\n    let messy = 1 + 2;\n    let other = 5;\n    7\n}\n";
const SAVED: &str = "fn total() -> u64 {\n    7\n}\n";
const LIVE: &str = "fn total() -> u64 {\n    let messy = 1 + 2;\n    7\n}\n";

fn warning(line: u32, name: &str) -> Value {
    json!({ "range": { "start": { "line": line, "character": 8 }, "end": { "line": line, "character": 8 + name.len() } },
            "severity": 2, "source": "rustc", "code": "unused_variables", "message": format!("unused variable: `{name}`") })
}

fn lines(items: &Value) -> Vec<(u64, String)> {
    items
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["source"] == "rustc")
        .map(|d| (d["range"]["start"]["line"].as_u64().unwrap(), d["message"].as_str().unwrap().to_string()))
        .collect()
}

/// In Zed: save with two unused variables (the check reports both), revert the file (`git checkout`), retype the
/// first one without saving. Zed moves `rustc` diagnostics through the unsaved edits itself, so they must come in
/// the coordinates of the file on disk: the two checked lines are gone from it, so nothing is reported, where
/// placing them in the buffer put `messy` a line below its name once Zed moved it again.
#[test]
fn test_compiler_diagnostics_use_saved_coordinates() {
    set_client_moves_rustc(true);
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("lib.rs");
    let (path, uri) = (file.to_str().unwrap().to_string(), path_to_uri(file.to_str().unwrap()));
    std::fs::write(&file, SAVED).unwrap();
    let mut overlay_mgr = OverlayManager::new();
    overlay_mgr.open_document(&uri, &path, 3, LIVE.to_string());
    let fc = FlycheckState::new();
    fc.set_latest_diagnostics_with_baseline(&uri, vec![warning(1, "messy"), warning(2, "other")], Some(CHECKED.to_string()));

    let doc = handle_diagnostic_with_flycheck(&json!({ "params": { "textDocument": { "uri": uri } } }), &overlay_mgr, Some(&fc));
    assert!(lines(&doc["items"]).is_empty(), "{doc}");
    let ws = handle_workspace_diagnostic_with_flycheck(&json!({ "params": {} }), &overlay_mgr, &MultiRootWorkspace::new(), Some(&fc));
    let report = ws["items"].as_array().unwrap().iter().find(|r| r["uri"] == uri.as_str()).unwrap().clone();
    assert!(lines(&report["items"]).is_empty(), "{report}");

    // Saved with a line added above the checked ones: they stay where the file on disk has them, line 2 and 3,
    // and the editor moves them through its unsaved edits.
    let saved_above = "// note\nfn total() -> u64 {\n    let messy = 1 + 2;\n    let other = 5;\n    7\n}\n";
    std::fs::write(&file, saved_above).unwrap();
    overlay_mgr.open_document(&uri, &path, 4, format!("// one\n{saved_above}"));
    let doc = handle_diagnostic_with_flycheck(&json!({ "params": { "textDocument": { "uri": uri } } }), &overlay_mgr, Some(&fc));
    assert_eq!(lines(&doc["items"]), vec![(2, "unused variable: `messy`".to_string()), (3, "unused variable: `other`".to_string())], "{doc}");

    // Another client takes them in the buffer: one line further down, under the line typed above.
    set_client_moves_rustc(false);
    let doc = handle_diagnostic_with_flycheck(&json!({ "params": { "textDocument": { "uri": uri } } }), &overlay_mgr, Some(&fc));
    assert_eq!(lines(&doc["items"]), vec![(3, "unused variable: `messy`".to_string()), (4, "unused variable: `other`".to_string())], "{doc}");
}

/// The editor's own mismatch check and the compiler's report of it meet in the buffer, where both sit: saved with
/// the error, a line typed above, Zed got the compiler's (saved line 1, moved by Zed to 2) and ridx's own (line 2)
/// both, as the two were compared across coordinates.
#[test]
fn test_editor_mismatch_shadowed_by_the_compiler_after_an_edit_above() {
    set_client_moves_rustc(true);
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("lib.rs");
    let (path, uri) = (file.to_str().unwrap().to_string(), path_to_uri(file.to_str().unwrap()));
    let saved = "fn f() {\n    let x: u32 = \"abc\";\n}\n";
    std::fs::write(&file, saved).unwrap();
    let mut overlay_mgr = OverlayManager::new();
    overlay_mgr.open_document(&uri, &path, 2, format!("// typed\n{saved}"));
    let fc = FlycheckState::new();
    let e0308 = json!({ "range": { "start": { "line": 1, "character": 4 }, "end": { "line": 1, "character": 24 } }, "severity": 1, "source": "rustc", "code": "E0308", "message": "mismatched types" });
    fc.set_latest_diagnostics_with_baseline(&uri, vec![e0308], Some(saved.to_string()));

    let own = handle_diagnostic_with_flycheck(&json!({ "params": { "textDocument": { "uri": uri } } }), &overlay_mgr, None);
    assert!(own["items"].as_array().unwrap().iter().any(|d| d["code"] == "E0308" && d["source"] == "ridx" && d["range"]["start"]["line"] == 2), "{own}");
    let doc = handle_diagnostic_with_flycheck(&json!({ "params": { "textDocument": { "uri": uri } } }), &overlay_mgr, Some(&fc));
    let e0308: Vec<&Value> = doc["items"].as_array().unwrap().iter().filter(|d| d["code"] == "E0308").collect();
    assert_eq!(e0308.len(), 1, "{doc}");
    assert_eq!((e0308[0]["source"].as_str(), e0308[0]["range"]["start"]["line"].as_u64()), (Some("rustc"), Some(1)), "{doc}");
}
