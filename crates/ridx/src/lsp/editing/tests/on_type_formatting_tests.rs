/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Unit tests for textDocument/onTypeFormatting and range formatting handlers.

use crate::lsp::editing::format::{handle_formatting, handle_on_type_formatting, handle_range_formatting};
use crate::overlay::OverlayManager;
use serde_json::json;

#[test]
fn test_on_type_formatting_semicolon_trigger() {
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/on_type_semi.rs";
    let code = "fn main() {let x=1;}\n";
    overlay_mgr.open_document(uri, "/tmp/on_type_semi.rs", 1, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": 0, "character": 19 },
            "ch": ";",
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });

    let res = handle_on_type_formatting(&req, &overlay_mgr);
    assert!(res.is_array());
}

#[test]
fn test_on_type_formatting_closing_brace_trigger() {
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/on_type_brace.rs";
    let code = "fn main() {\nlet y=2;\n}\n";
    overlay_mgr.open_document(uri, "/tmp/on_type_brace.rs", 1, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": 2, "character": 1 },
            "ch": "}",
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });

    let res = handle_on_type_formatting(&req, &overlay_mgr);
    assert!(res.is_array());
}

#[test]
fn test_on_type_formatting_syntax_error_returns_empty_cleanly() {
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/syntax_err.rs";
    let code = "fn main( {\n"; // Incomplete syntax while typing
    overlay_mgr.open_document(uri, "/tmp/syntax_err.rs", 1, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": 0, "character": 10 },
            "ch": "(",
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });

    let res = handle_on_type_formatting(&req, &overlay_mgr);
    assert_eq!(res, json!([]), "Syntax errors during typing must return empty edits without panic");
}

#[test]
fn test_on_type_formatting_missing_file_returns_empty() {
    let overlay_mgr = OverlayManager::new();
    let req = json!({
        "params": {
            "textDocument": { "uri": "file:///tmp/non_existent_file_xyz.rs" },
            "position": { "line": 0, "character": 0 },
            "ch": ";",
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });

    let res = handle_on_type_formatting(&req, &overlay_mgr);
    assert_eq!(res, json!([]));
}

#[test]
fn test_on_type_formatting_already_formatted_returns_empty() {
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/already_formatted.rs";
    let code = "fn main() {\n    let x = 1;\n}\n";
    overlay_mgr.open_document(uri, "/tmp/already_formatted.rs", 1, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": 1, "character": 14 },
            "ch": ";",
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });

    let res = handle_on_type_formatting(&req, &overlay_mgr);
    assert_eq!(res, json!([]), "Already formatted code must return empty edits");
}

#[test]
fn test_range_formatting_basic() {
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/range_format.rs";
    let code = "fn foo() {\nlet a = 1+2;\nlet b = 3;\n}\n";
    overlay_mgr.open_document(uri, "/tmp/range_format.rs", 1, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": uri },
            "range": {
                "start": { "line": 1, "character": 0 },
                "end": { "line": 1, "character": 12 }
            },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });

    let res = handle_range_formatting(&req, &overlay_mgr);
    assert!(res.is_array());
}

#[test]
fn test_range_formatting_invalid_bounds() {
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/range_invalid.rs";
    let code = "fn main() {}\n";
    overlay_mgr.open_document(uri, "/tmp/range_invalid.rs", 1, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": uri },
            "range": {
                "start": { "line": 999, "character": 0 },
                "end": { "line": 1000, "character": 0 }
            },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });

    let res = handle_range_formatting(&req, &overlay_mgr);
    assert_eq!(res, json!([]));
}

#[test]
fn test_range_formatting_empty_doc() {
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/range_empty.rs";
    overlay_mgr.open_document(uri, "/tmp/range_empty.rs", 1, String::new());

    let req = json!({
        "params": {
            "textDocument": { "uri": uri },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 0, "character": 0 }
            },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });

    let res = handle_range_formatting(&req, &overlay_mgr);
    assert_eq!(res, json!([]));
}

#[test]
fn test_handle_formatting_with_dirty_overlay() {
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/format_overlay.rs";
    let code = "fn test() {let z=99;}\n";
    overlay_mgr.open_document(uri, "/tmp/format_overlay.rs", 1, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": uri },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });

    let res = handle_formatting(&req, &overlay_mgr);
    assert!(res.is_array());
}
