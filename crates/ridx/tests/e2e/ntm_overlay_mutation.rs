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
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter, RawDefinition};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

struct TestDir(PathBuf);
impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn send_lsp(
    msg: Value,
    lifecycle: &mut LifecycleManager,
    multi_root: &mut MultiRootWorkspace,
    overlay_mgr: &mut OverlayManager,
    cancelled: &Arc<Mutex<HashSet<Value>>>,
    index_path: Option<&PathBuf>,
) -> Value {
    let mut out = Cursor::new(Vec::new());
    let _ = dispatch_lsp_message(msg, &mut out, lifecycle, multi_root, overlay_mgr, cancelled, index_path);
    let bytes = out.into_inner();
    if bytes.is_empty() {
        return Value::Null;
    }
    let text = String::from_utf8_lossy(&bytes);
    if let Some(pos) = text.find("\r\n\r\n") {
        serde_json::from_str(&text[pos + 4..]).unwrap_or(Value::Null)
    } else {
        Value::Null
    }
}

#[test]
fn test_ntm_overlay_mutations_and_editing_pipeline() {
    let root = std::env::temp_dir().join(format!("ridx_ntm_mut_{}", std::process::id()));
    let _ = fs::create_dir_all(root.join("src"));
    let _ = fs::create_dir_all(root.join("target/ridx"));
    let _guard = TestDir(root.clone());

    let src_path = root.join("src/lib.rs");
    let initial_code = "pub fn worker(job_id: u32) -> bool {\n    true\n}\n";
    fs::write(&src_path, initial_code).unwrap();

    let ridx_path = root.join("target/ridx/my_crate.ridx");
    let mut writer = IndexWriter::new("my_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        file: src_path.to_str().unwrap().to_string(),
        name: "worker".to_string(),
        start_line: 1,
        start_col: 8,
        end_line: 1,
        end_col: 14,
        path: "my_crate::worker".to_string(),
        signature: Some("pub fn worker(job_id: u32) -> bool".to_string()),
    });
    let mut f = File::create(&ridx_path).unwrap();
    writer.write_to(&mut f).unwrap();

    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let cancelled = Arc::new(Mutex::new(HashSet::new()));
    let index_dir = root.join("target/ridx");
    let uri = format!("file://{}", src_path.to_str().unwrap());

    // 1. Initialize & Initialized
    let _ = send_lsp(
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "rootUri": format!("file://{}", root.to_str().unwrap()) } }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        Some(&index_dir),
    );
    let _ = send_lsp(json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }), &mut lifecycle, &mut multi_root, &mut overlay_mgr, &cancelled, Some(&index_dir));

    // 2. didOpen
    let _ = send_lsp(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": &uri, "version": 1, "text": initial_code } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        Some(&index_dir),
    );

    // 3. Inlay hints on initial code
    let hints_resp = send_lsp(
        json!({
            "jsonrpc": "2.0", "id": 10, "method": "textDocument/inlayHint",
            "params": { "textDocument": { "uri": &uri }, "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 2, "character": 0 } } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        Some(&index_dir),
    );
    assert_eq!(hints_resp["id"], 10);
    assert!(hints_resp["result"].is_array());

    // 4. didChange: insert comment line at start, shifting worker from line 1 to line 2
    let modified_code = format!("// Added header comment\n{}", initial_code);
    let _ = send_lsp(
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": &uri, "version": 2 },
                "contentChanges": [{ "text": modified_code }]
            }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        Some(&index_dir),
    );

    // 5. Hover on shifted position (line 1, col 10 in 0-indexed coords)
    let hover_shifted = send_lsp(
        json!({
            "jsonrpc": "2.0", "id": 20, "method": "textDocument/hover",
            "params": { "textDocument": { "uri": &uri }, "position": { "line": 1, "character": 10 } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        Some(&index_dir),
    );
    assert_eq!(hover_shifted["id"], 20);
    assert!(!hover_shifted["result"].is_null());

    // 6. Formatting
    let fmt_resp = send_lsp(
        json!({
            "jsonrpc": "2.0", "id": 30, "method": "textDocument/formatting",
            "params": { "textDocument": { "uri": &uri }, "options": { "tabSize": 4, "insertSpaces": true } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        Some(&index_dir),
    );
    assert_eq!(fmt_resp["id"], 30);

    // 7. PrepareRename & Rename
    let prep_rename = send_lsp(
        json!({
            "jsonrpc": "2.0", "id": 40, "method": "textDocument/prepareRename",
            "params": { "textDocument": { "uri": &uri }, "position": { "line": 1, "character": 10 } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        Some(&index_dir),
    );
    assert_eq!(prep_rename["id"], 40);

    let rename_resp = send_lsp(
        json!({
            "jsonrpc": "2.0", "id": 41, "method": "textDocument/rename",
            "params": { "textDocument": { "uri": &uri }, "position": { "line": 1, "character": 10 }, "newName": "new_worker" }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        Some(&index_dir),
    );
    assert_eq!(rename_resp["id"], 41);

    // 8. CodeAction
    let ca_resp = send_lsp(
        json!({
            "jsonrpc": "2.0", "id": 50, "method": "textDocument/codeAction",
            "params": { "textDocument": { "uri": &uri }, "range": { "start": { "line": 1, "character": 0 }, "end": { "line": 1, "character": 10 } }, "context": { "diagnostics": [] } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        Some(&index_dir),
    );
    assert_eq!(ca_resp["id"], 50);

    // 9. didSave & didClose
    let _ = send_lsp(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didSave", "params": { "textDocument": { "uri": &uri } } }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        Some(&index_dir),
    );
    let _ = send_lsp(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didClose", "params": { "textDocument": { "uri": &uri } } }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        Some(&index_dir),
    );

    // 10. Shutdown
    let sd = send_lsp(json!({ "jsonrpc": "2.0", "id": 99, "method": "shutdown", "params": {} }), &mut lifecycle, &mut multi_root, &mut overlay_mgr, &cancelled, Some(&index_dir));
    assert_eq!(sd["id"], 99);
}
