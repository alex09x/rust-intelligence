/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use ridx::lsp::editing::format::{handle_formatting, handle_range_formatting};
use ridx::overlay::OverlayManager;
use ridx_test_support::apply_text_edits;
use serde_json::json;

#[test]
fn test_refactor_formatting_and_range_formatting_execution() {
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///workspace/src/messy.rs";
    let file_path = "/workspace/src/messy.rs";

    let unformatted = concat!("fn first( x: i32 ) -> i32 { x+1 }\n", "\n", "fn second(y:i32)->i32{\n", "let z=y*2;\n", "z\n", "}\n",);

    overlay_mgr.open_document(uri, file_path, 1, unformatted.to_string());

    let rustfmt_available = std::process::Command::new("rustfmt").arg("--version").output().map(|o| o.status.success()).unwrap_or(false);
    if !rustfmt_available {
        eprintln!("Skipping formatting execution test: rustfmt binary is not available in PATH");
        return;
    }

    let fmt_req = json!({
        "params": {
            "textDocument": { "uri": uri },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    let fmt_edits = handle_formatting(&fmt_req, &overlay_mgr);
    let edits_arr = fmt_edits.as_array().expect("formatting edits array");
    assert!(!edits_arr.is_empty(), "Full formatting MUST return edits for unformatted code when rustfmt is available");
    let formatted = apply_text_edits(unformatted, edits_arr);
    assert_ne!(formatted, unformatted);
    assert!(formatted.contains("fn first(x: i32) -> i32 {"));
    assert!(formatted.contains("fn second(y: i32) -> i32 {"));

    let range_req = json!({
        "params": {
            "textDocument": { "uri": uri },
            "range": {
                "start": { "line": 2, "character": 0 },
                "end": { "line": 5, "character": 1 }
            },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    let range_edits = handle_range_formatting(&range_req, &overlay_mgr);
    let r_arr = range_edits.as_array().expect("range edits array");
    assert!(!r_arr.is_empty(), "Range formatting MUST return edits for unformatted second function when rustfmt is available");
    let partially_formatted = apply_text_edits(unformatted, r_arr);
    assert_ne!(partially_formatted, unformatted);
    assert!(partially_formatted.starts_with("fn first( x: i32 ) -> i32 {"));
    assert!(partially_formatted.contains("fn second(y: i32) -> i32 {"));
}
