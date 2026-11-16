/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::protocol::LspMessage;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

fn extract_published_diagnostics(output: &[u8]) -> Vec<Value> {
    let mut diags = Vec::new();
    let text = String::from_utf8_lossy(output);
    for part in text.split("Content-Length: ") {
        if let Some(idx) = part.find("\r\n\r\n") {
            let json_str = &part[idx + 4..];
            if let Ok(val) = serde_json::from_str::<Value>(json_str.trim()) {
                if val["method"] == "textDocument/publishDiagnostics" {
                    diags.push(val);
                }
            }
        }
    }
    diags
}

#[test]
fn test_e2e_flycheck_e0425_line_shifted_after_lines_inserted_above() {
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let mut lifecycle = LifecycleManager::new();
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    // 1. Initialize as push client (default)
    let init_req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "rootUri": "file:///tmp/dummy_workspace",
            "capabilities": {}
        }
    });
    let mut out = Vec::new();
    dispatch_lsp_message(LspMessage::Client(init_req), &mut out, &mut lifecycle, &mut multi_root, &mut overlay_mgr, &cancelled, None).unwrap();

    let uri = "file:///tmp/dummy_workspace/src/lib.rs";
    let initial_text = "fn foo() {\n    let a = 1;\n    let b = 2;\n    let c = unresolved_var;\n}\n";

    // 2. didOpen
    out.clear();
    let open_notif = json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": uri,
                "languageId": "rust",
                "version": 1,
                "text": initial_text
            }
        }
    });
    dispatch_lsp_message(LspMessage::Client(open_notif), &mut out, &mut lifecycle, &mut multi_root, &mut overlay_mgr, &cancelled, None).unwrap();

    // 3. didSave -> saves document to disk (flycheck baseline established, compiled_text untouched)
    out.clear();
    let save_notif = json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didSave",
        "params": {
            "textDocument": { "uri": uri }
        }
    });
    dispatch_lsp_message(LspMessage::Client(save_notif), &mut out, &mut lifecycle, &mut multi_root, &mut overlay_mgr, &cancelled, None).unwrap();

    // 4. Flycheck delivers E0425 on line 3 (0-based)
    out.clear();
    let gen = lifecycle.flycheck.current_generation();
    let flycheck_diag = json!({
        "jsonrpc": "2.0",
        "method": "textDocument/publishDiagnostics",
        "params": {
            "uri": uri,
            "diagnostics": [
                {
                    "range": {
                        "start": { "line": 3, "character": 12 },
                        "end": { "line": 3, "character": 26 }
                    },
                    "severity": 1,
                    "code": "E0425",
                    "source": "rustc",
                    "message": "cannot find value `unresolved_var` in this scope"
                }
            ]
        }
    });
    dispatch_lsp_message(LspMessage::InternalDiagnostic(gen, flycheck_diag), &mut out, &mut lifecycle, &mut multi_root, &mut overlay_mgr, &cancelled, None).unwrap();

    let published1 = extract_published_diagnostics(&out);
    assert_eq!(published1.len(), 1);
    assert_eq!(published1[0]["params"]["diagnostics"][0]["code"], "E0425");
    assert_eq!(published1[0]["params"]["diagnostics"][0]["range"]["start"]["line"], 3);

    // 5. User inserts 2 comment lines at the very beginning of the document
    out.clear();
    let shifted_text = "// header 1\n// header 2\nfn foo() {\n    let a = 1;\n    let b = 2;\n    let c = unresolved_var;\n}\n";
    let change_notif = json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didChange",
        "params": {
            "textDocument": {
                "uri": uri,
                "version": 2
            },
            "contentChanges": [
                { "text": shifted_text }
            ]
        }
    });
    dispatch_lsp_message(LspMessage::Client(change_notif), &mut out, &mut lifecycle, &mut multi_root, &mut overlay_mgr, &cancelled, None).unwrap();

    // Verify: E0425 stays where the saved file has it. `rustc` diagnostics are disk-based for the client (Zed under
    // the rust-analyzer adapter moves them through unsaved edits itself); moved here too, Zed showed them twice as
    // far down.
    let published2 = extract_published_diagnostics(&out);
    assert_eq!(published2.len(), 1, "Should publish reconciled diagnostics on didChange");
    let diags = &published2[0]["params"]["diagnostics"];
    let e0425 = diags.as_array().unwrap().iter().find(|d| d["code"] == "E0425");
    assert!(e0425.is_some(), "E0425 must be preserved");
    assert_eq!(e0425.unwrap()["range"]["start"]["line"], 3, "E0425 is on line 3 of the saved file");
    assert_eq!(e0425.unwrap()["range"]["end"]["line"], 3);

    // 6. User edits the error line directly (fixes the variable)
    out.clear();
    let fixed_text = "// header 1\n// header 2\nfn foo() {\n    let a = 1;\n    let b = 2;\n    let c = 42;\n}\n";
    let change_notif_fixed = json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didChange",
        "params": {
            "textDocument": {
                "uri": uri,
                "version": 3
            },
            "contentChanges": [
                { "text": fixed_text }
            ]
        }
    });
    dispatch_lsp_message(LspMessage::Client(change_notif_fixed), &mut out, &mut lifecycle, &mut multi_root, &mut overlay_mgr, &cancelled, None).unwrap();

    // Verify: the saved file still has the error, so it stays until the fix is saved and checked.
    let published3 = extract_published_diagnostics(&out);
    assert_eq!(published3.len(), 1);
    let diags3 = published3[0]["params"]["diagnostics"].as_array().unwrap();
    assert!(diags3.iter().any(|d| d["code"] == "E0425" && d["range"]["start"]["line"] == 3), "{diags3:?}");
}
