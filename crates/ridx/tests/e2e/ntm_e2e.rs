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
use ridx_format::{DefKind, IndexWriter, RawDefinition, RawReference, RawType};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

struct TestFixture {
    root_dir: PathBuf,
    #[allow(dead_code)]
    lib_src: PathBuf,
    app_src: PathBuf,
}

impl Drop for TestFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root_dir);
    }
}

fn setup_multi_crate_fixture() -> TestFixture {
    let root = std::env::temp_dir().join(format!("ridx_ntm_{}", std::process::id()));
    let _ = fs::create_dir_all(root.join("core_lib/src"));
    let _ = fs::create_dir_all(root.join("app_bin/src"));
    let _ = fs::create_dir_all(root.join("target/ridx"));

    let lib_src = root.join("core_lib/src/lib.rs");
    let lib_code = r#"pub struct Engine { pub id: u64 }
impl Engine {
    pub fn new(id: u64) -> Self { Self { id } }
    pub fn run(&self) -> bool { true }
}
pub fn compute_hash(data: &[u8]) -> u64 { 42 }
"#;
    fs::write(&lib_src, lib_code).unwrap();

    let app_src = root.join("app_bin/src/main.rs");
    let app_code = r#"use core_lib::{Engine, compute_hash};
fn main() {
    let engine = Engine::new(100);
    let ok = engine.run();
    let hash = compute_hash(b"hello");
}
"#;
    fs::write(&app_src, app_code).unwrap();

    // Write core_lib.ridx
    let lib_ridx = root.join("target/ridx/core_lib.ridx");
    let mut lib_writer = IndexWriter::new("core_lib", "0.1.0");
    lib_writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        file: lib_src.to_str().unwrap().to_string(),
        name: "Engine".to_string(),
        start_line: 1,
        start_col: 12,
        end_line: 1,
        end_col: 18,
        path: "core_lib::Engine".to_string(),
        signature: Some("pub struct Engine".to_string()),
    });
    lib_writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        file: lib_src.to_str().unwrap().to_string(),
        name: "compute_hash".to_string(),
        start_line: 6,
        start_col: 8,
        end_line: 6,
        end_col: 20,
        path: "core_lib::compute_hash".to_string(),
        signature: Some("pub fn compute_hash(data: &[u8]) -> u64".to_string()),
    });
    lib_writer.add_file_hash(lib_src.to_str().unwrap(), ridx_format::normalized_source_hash(lib_code.as_bytes()));
    let mut f_lib = File::create(&lib_ridx).unwrap();
    lib_writer.write_to(&mut f_lib).unwrap();

    // Write app_bin.ridx
    let app_ridx = root.join("target/ridx/app_bin.ridx");
    let mut app_writer = IndexWriter::new("app_bin", "0.1.0");
    app_writer.add_file_hash(app_src.to_str().unwrap(), ridx_format::normalized_source_hash(app_code.as_bytes()));
    app_writer.add_definition(RawDefinition {
        def_id: 10,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        file: app_src.to_str().unwrap().to_string(),
        name: "main".to_string(),
        start_line: 2,
        start_col: 4,
        end_line: 2,
        end_col: 8,
        path: "app_bin::main".to_string(),
        signature: Some("fn main()".to_string()),
    });
    app_writer.add_reference(RawReference {
        file: app_src.to_str().unwrap().to_string(),
        start_line: 3,
        start_col: 18,
        end_line: 3,
        end_col: 24,
        target_def_id: Some(1),
        target_path: "core_lib::Engine".to_string(),
        target_unit: None,
        target_file: Some(lib_src.to_str().unwrap().to_string()),
        target_line: 1,
        target_col: 12,
        kind: DefKind::Struct,
        is_write: false,
        is_macro_expansion: false,
    });
    app_writer.add_reference(RawReference {
        file: app_src.to_str().unwrap().to_string(),
        start_line: 5,
        start_col: 16,
        end_line: 5,
        end_col: 28,
        target_def_id: Some(2),
        target_path: "core_lib::compute_hash".to_string(),
        target_unit: None,
        target_file: Some(lib_src.to_str().unwrap().to_string()),
        target_line: 6,
        target_col: 8,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    app_writer.add_type(RawType { file: app_src.to_str().unwrap().to_string(), start_line: 5, start_col: 9, end_line: 5, end_col: 13, type_name: "u64".to_string() });
    let mut f_app = File::create(&app_ridx).unwrap();
    app_writer.write_to(&mut f_app).unwrap();

    TestFixture { root_dir: root, lib_src, app_src }
}

fn send_req(
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
fn test_ntm_end_to_end_lsp_pipeline() {
    let fixture = setup_multi_crate_fixture();
    let mut lifecycle = LifecycleManager::new();
    let mut multi_root = MultiRootWorkspace::new();
    let mut overlay_mgr = OverlayManager::new();
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    let index_dir = fixture.root_dir.join("target/ridx");
    let app_uri = format!("file://{}", fixture.app_src.to_str().unwrap());

    // 1. initialize
    let init_req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "rootUri": format!("file://{}", fixture.root_dir.to_str().unwrap()),
            "workspaceFolders": [{
                "uri": format!("file://{}", fixture.root_dir.to_str().unwrap()),
                "name": "test_workspace"
            }]
        }
    });
    let init_resp = send_req(init_req, &mut lifecycle, &mut multi_root, &mut overlay_mgr, &cancelled, Some(&index_dir));
    assert_eq!(init_resp["id"], 1);
    assert!(init_resp["result"]["capabilities"]["definitionProvider"].as_bool().unwrap());
    assert!(init_resp["result"]["capabilities"]["referencesProvider"].as_bool().unwrap());

    // 2. initialized notification
    let initialized = json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} });
    let _ = send_req(initialized, &mut lifecycle, &mut multi_root, &mut overlay_mgr, &cancelled, Some(&index_dir));

    // 3. didOpen app_bin/src/main.rs
    let app_text = fs::read_to_string(&fixture.app_src).unwrap();
    let did_open = json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": { "uri": &app_uri, "version": 1, "text": &app_text }
        }
    });
    let _ = send_req(did_open, &mut lifecycle, &mut multi_root, &mut overlay_mgr, &cancelled, Some(&index_dir));

    // 4. textDocument/definition on compute_hash at line 5 (0-indexed line 4, col 18)
    let def_req = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "textDocument/definition",
        "params": {
            "textDocument": { "uri": &app_uri },
            "position": { "line": 4, "character": 18 }
        }
    });
    let def_resp = send_req(def_req, &mut lifecycle, &mut multi_root, &mut overlay_mgr, &cancelled, Some(&index_dir));
    assert_eq!(def_resp["id"], 2);
    let def_uri = def_resp["result"]["uri"].as_str().unwrap_or("");
    assert!(def_uri.contains("core_lib"));

    // 5. textDocument/references on compute_hash
    let refs_req = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "textDocument/references",
        "params": {
            "textDocument": { "uri": &app_uri },
            "position": { "line": 4, "character": 18 },
            "context": { "includeDeclaration": true }
        }
    });
    let refs_resp = send_req(refs_req, &mut lifecycle, &mut multi_root, &mut overlay_mgr, &cancelled, Some(&index_dir));
    assert_eq!(refs_resp["id"], 3);
    assert!(!refs_resp["result"].as_array().unwrap().is_empty());

    // 6. textDocument/hover on compute_hash
    let hover_req = json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": &app_uri },
            "position": { "line": 4, "character": 18 }
        }
    });
    let hover_resp = send_req(hover_req, &mut lifecycle, &mut multi_root, &mut overlay_mgr, &cancelled, Some(&index_dir));
    assert_eq!(hover_resp["id"], 4);
    assert!(hover_resp["result"]["contents"]["value"].as_str().unwrap().contains("compute_hash"));

    // 7. workspace/symbol query
    let sym_req = json!({
        "jsonrpc": "2.0",
        "id": 5,
        "method": "workspace/symbol",
        "params": { "query": "Engine" }
    });
    let sym_resp = send_req(sym_req, &mut lifecycle, &mut multi_root, &mut overlay_mgr, &cancelled, Some(&index_dir));
    assert_eq!(sym_resp["id"], 5);
    assert!(sym_resp["result"].as_array().unwrap().iter().any(|s| s["name"] == "Engine"));

    // 8. shutdown and exit
    let shutdown_req = json!({ "jsonrpc": "2.0", "id": 6, "method": "shutdown", "params": {} });
    let shutdown_resp = send_req(shutdown_req, &mut lifecycle, &mut multi_root, &mut overlay_mgr, &cancelled, Some(&index_dir));
    assert_eq!(shutdown_resp["id"], 6);

    let exit_req = json!({ "jsonrpc": "2.0", "method": "exit", "params": {} });
    let mut exit_buf = Vec::new();
    let _ = dispatch_lsp_message(exit_req, &mut exit_buf, &mut lifecycle, &mut multi_root, &mut overlay_mgr, &cancelled, Some(&index_dir));
}
