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
use ridx_format::{DefKind, IndexWriter, RawDefinition, RawReference};
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
fn test_ntm_hierarchy_and_protocol_invariants() {
    let root = std::env::temp_dir().join(format!("ridx_ntm_hier_{}", std::process::id()));
    let _ = fs::create_dir_all(root.join("src"));
    let _ = fs::create_dir_all(root.join("target/ridx"));
    let _guard = TestDir(root.clone());

    let src_path = root.join("src/lib.rs");
    let code = "pub fn caller() { callee(); }\npub fn callee() {}\n";
    fs::write(&src_path, code).unwrap();

    let ridx_path = root.join("target/ridx/hier_crate.ridx");
    let mut writer = IndexWriter::new("hier_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        file: src_path.to_str().unwrap().to_string(),
        name: "caller".to_string(),
        start_line: 1,
        start_col: 8,
        end_line: 1,
        end_col: 14,
        path: "hier_crate::caller".to_string(),
        signature: Some("pub fn caller()".to_string()),
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        file: src_path.to_str().unwrap().to_string(),
        name: "callee".to_string(),
        start_line: 2,
        start_col: 8,
        end_line: 2,
        end_col: 14,
        path: "hier_crate::callee".to_string(),
        signature: Some("pub fn callee()".to_string()),
    });
    writer.add_reference(RawReference {
        file: src_path.to_str().unwrap().to_string(),
        start_line: 1,
        start_col: 19,
        end_line: 1,
        end_col: 25,
        target_def_id: Some(2),
        target_path: "hier_crate::callee".to_string(),
        target_unit: None,
        target_file: Some(src_path.to_str().unwrap().to_string()),
        target_line: 2,
        target_col: 8,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    let mut f = File::create(&ridx_path).unwrap();
    writer.write_to(&mut f).unwrap();

    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let cancelled = Arc::new(Mutex::new(HashSet::new()));
    let index_dir = root.join("target/ridx");
    let uri = format!("file://{}", src_path.to_str().unwrap());

    // Invariant 1: request before initialize returns -32002
    let premature_req = json!({
        "jsonrpc": "2.0", "id": 1, "method": "textDocument/definition",
        "params": { "textDocument": { "uri": &uri }, "position": { "line": 0, "character": 0 } }
    });
    let prem_resp = send_lsp(premature_req, &mut lifecycle, &mut multi_root, &mut overlay_mgr, &cancelled, Some(&index_dir));
    assert_eq!(prem_resp["error"]["code"], -32002);

    // Initialize
    let _ = send_lsp(
        json!({ "jsonrpc": "2.0", "id": 2, "method": "initialize", "params": { "rootUri": format!("file://{}", root.to_str().unwrap()) } }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        Some(&index_dir),
    );
    let _ = send_lsp(json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }), &mut lifecycle, &mut multi_root, &mut overlay_mgr, &cancelled, Some(&index_dir));

    // Open document
    let _ = send_lsp(
        json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": { "textDocument": { "uri": &uri, "version": 1, "text": code } } }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        Some(&index_dir),
    );

    // PrepareCallHierarchy
    let prep_call = send_lsp(
        json!({
            "jsonrpc": "2.0", "id": 10, "method": "textDocument/prepareCallHierarchy",
            "params": { "textDocument": { "uri": &uri }, "position": { "line": 1, "character": 10 } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        Some(&index_dir),
    );
    assert_eq!(prep_call["id"], 10);
    assert!(prep_call["result"].is_array());
    let call_item = &prep_call["result"][0];

    // IncomingCalls
    let incoming = send_lsp(
        json!({
            "jsonrpc": "2.0", "id": 11, "method": "callHierarchy/incomingCalls",
            "params": { "item": call_item }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        Some(&index_dir),
    );
    assert_eq!(incoming["id"], 11);
    assert!(incoming["result"].is_array());

    // OutgoingCalls
    let outgoing = send_lsp(
        json!({
            "jsonrpc": "2.0", "id": 12, "method": "callHierarchy/outgoingCalls",
            "params": { "item": call_item }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        Some(&index_dir),
    );
    assert_eq!(outgoing["id"], 12);
    assert!(outgoing["result"].is_array());

    // SemanticTokensFull
    let sem_tokens = send_lsp(
        json!({
            "jsonrpc": "2.0", "id": 20, "method": "textDocument/semanticTokens/full",
            "params": { "textDocument": { "uri": &uri } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        Some(&index_dir),
    );
    assert_eq!(sem_tokens["id"], 20);

    // DocumentHighlight
    let highlight = send_lsp(
        json!({
            "jsonrpc": "2.0", "id": 21, "method": "textDocument/documentHighlight",
            "params": { "textDocument": { "uri": &uri }, "position": { "line": 0, "character": 10 } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        Some(&index_dir),
    );
    assert_eq!(highlight["id"], 21);

    // FoldingRange
    let folding = send_lsp(
        json!({
            "jsonrpc": "2.0", "id": 22, "method": "textDocument/foldingRange",
            "params": { "textDocument": { "uri": &uri } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        Some(&index_dir),
    );
    assert_eq!(folding["id"], 22);

    // Unknown method -> -32601
    let unknown = send_lsp(
        json!({ "jsonrpc": "2.0", "id": 90, "method": "custom/nonexistent", "params": {} }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        Some(&index_dir),
    );
    assert_eq!(unknown["error"]["code"], -32601);

    // Shutdown
    let _ = send_lsp(json!({ "jsonrpc": "2.0", "id": 99, "method": "shutdown", "params": {} }), &mut lifecycle, &mut multi_root, &mut overlay_mgr, &cancelled, Some(&index_dir));

    // Post-shutdown request -> -32600
    let post_sd = send_lsp(
        json!({
            "jsonrpc": "2.0", "id": 100, "method": "textDocument/hover",
            "params": { "textDocument": { "uri": &uri }, "position": { "line": 0, "character": 0 } }
        }),
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        Some(&index_dir),
    );
    assert_eq!(post_sd["error"]["code"], -32600);
}
