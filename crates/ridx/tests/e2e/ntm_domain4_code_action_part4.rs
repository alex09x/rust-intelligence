/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! End-to-end tests for `textDocument/codeAction` (Part 4: auto-import, symbol resolution, protocol conformance).

use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::dispatch::dispatch_lsp_message;
use ridx::lsp::protocol::LspMessage;
use ridx::overlay::OverlayManager;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx_format::DefKind;
use ridx_test_support::FixtureIndexBuilder;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

fn extract_response(out: &[u8]) -> Value {
    let s = String::from_utf8_lossy(out);
    for part in s.split("Content-Length: ") {
        if let Some(idx) = part.find("\r\n\r\n") {
            let body = &part[idx + 4..];
            if let Ok(v) = serde_json::from_str::<Value>(body.trim()) {
                return v;
            }
        }
    }
    Value::Null
}

fn setup_initialized_server() -> (
    MultiRootWorkspace,
    OverlayManager,
    LifecycleManager,
    Arc<Mutex<HashSet<Value>>>,
) {
    let mut mr = MultiRootWorkspace::new();
    let mut ov = OverlayManager::new();
    let mut lc = LifecycleManager::new();
    let can = Arc::new(Mutex::new(HashSet::new()));
    let mut out = Vec::new();

    let init = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "capabilities": {} }
    });
    dispatch_lsp_message(LspMessage::Client(init), &mut out, &mut lc, &mut mr, &mut ov, &can, None).unwrap();
    (mr, ov, lc, can)
}

fn send_code_action(uri: &str, range: Value, ctx: Value, mr: &mut MultiRootWorkspace, ov: &mut OverlayManager, lc: &mut LifecycleManager, can: &Arc<Mutex<HashSet<Value>>>) -> Value {
    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0", "id": 100, "method": "textDocument/codeAction",
        "params": { "textDocument": { "uri": uri }, "range": range, "context": ctx }
    });
    dispatch_lsp_message(LspMessage::Client(req), &mut out, lc, mr, ov, can, None).unwrap();
    extract_response(&out)
}

#[test]
fn test_code_action_auto_import_e0425_from_workspace_symbol() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let temp = tempfile::tempdir().expect("tempdir");
    let file_path = temp.path().join("main.rs");
    std::fs::write(&file_path, "fn main() { let _ = Widget::new(); }\n").expect("write file");

    let mut builder = FixtureIndexBuilder::new("app", "0.1.0");
    let (code, _) = builder.add_defs_with_ids("pub struct «Widget»;\n", file_path.to_str().unwrap(), "app", DefKind::Struct);
    std::fs::write(&file_path, code).expect("write code");
    let ridx_file = temp.path().join("app.ridx");
    builder.write_to_file(&ridx_file);
    mr.add_root(ridx_file);

    let uri = format!("file://{}", file_path.display());
    ov.open_with_provenance(&uri, file_path.to_str().unwrap(), 1, "fn main() { let _ = Widget::new(); }\n".to_string(), true);

    let range = json!({ "start": { "line": 0, "character": 20 }, "end": { "line": 0, "character": 26 } });
    let diag = json!({ "range": range, "message": "cannot find value `Widget` in this scope", "code": "E0425" });
    let res = send_code_action(&uri, range, json!({ "only": ["quickfix"], "diagnostics": [diag] }), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["id"], 100);
    let actions = res["result"].as_array().expect("actions array");
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0]["title"], "Import `app::Widget`");
    assert_eq!(actions[0]["kind"], "quickfix");
}

#[test]
fn test_code_action_auto_import_e0412_from_workspace_symbol() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let temp = tempfile::tempdir().expect("tempdir");
    let file_path = temp.path().join("types.rs");
    std::fs::write(&file_path, "fn test<T: TraitItem>() {}\n").expect("write file");

    let mut builder = FixtureIndexBuilder::new("app", "0.1.0");
    let (code, _) = builder.add_defs_with_ids("pub trait «TraitItem» {}\n", file_path.to_str().unwrap(), "app", DefKind::Trait);
    std::fs::write(&file_path, code).expect("write code");
    let ridx_file = temp.path().join("app.ridx");
    builder.write_to_file(&ridx_file);
    mr.add_root(ridx_file);

    let uri = format!("file://{}", file_path.display());
    ov.open_with_provenance(&uri, file_path.to_str().unwrap(), 1, "fn test<T: TraitItem>() {}\n".to_string(), true);

    let range = json!({ "start": { "line": 0, "character": 11 }, "end": { "line": 0, "character": 20 } });
    let diag = json!({ "range": range, "message": "cannot find type `TraitItem` in this scope", "code": "E0412" });
    let res = send_code_action(&uri, range, json!({ "only": ["quickfix"], "diagnostics": [diag] }), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["id"], 100);
    let actions = res["result"].as_array().expect("actions array");
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0]["title"], "Import `app::TraitItem`");
    assert_eq!(actions[0]["kind"], "quickfix");
}

#[test]
fn test_code_action_auto_import_cannot_find_message_fallback() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let temp = tempfile::tempdir().expect("tempdir");
    let file_path = temp.path().join("helper.rs");
    std::fs::write(&file_path, "fn run() { calc(); }\n").expect("write file");

    let mut builder = FixtureIndexBuilder::new("app", "0.1.0");
    let (code, _) = builder.add_defs_with_ids("pub fn «calc»() {}\n", file_path.to_str().unwrap(), "app", DefKind::Fn);
    std::fs::write(&file_path, code).expect("write code");
    let ridx_file = temp.path().join("app.ridx");
    builder.write_to_file(&ridx_file);
    mr.add_root(ridx_file);

    let uri = format!("file://{}", file_path.display());
    ov.open_with_provenance(&uri, file_path.to_str().unwrap(), 1, "fn run() { calc(); }\n".to_string(), true);

    let range = json!({ "start": { "line": 0, "character": 11 }, "end": { "line": 0, "character": 15 } });
    let diag = json!({ "range": range, "message": "cannot find function `calc` in this scope", "code": "" });
    let res = send_code_action(&uri, range, json!({ "only": ["quickfix"], "diagnostics": [diag] }), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["id"], 100);
    let actions = res["result"].as_array().expect("actions array");
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0]["title"], "Import `app::calc`");
}

#[test]
fn test_code_action_auto_import_deduplicates_identical_paths() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let temp = tempfile::tempdir().expect("tempdir");
    let file_path = temp.path().join("dedup.rs");
    std::fs::write(&file_path, "fn check() { item(); }\n").expect("write file");

    let mut builder = FixtureIndexBuilder::new("app", "0.1.0");
    let (code, _) = builder.add_defs_with_ids("pub fn «item»() {}\n", file_path.to_str().unwrap(), "app", DefKind::Fn);
    std::fs::write(&file_path, code).expect("write code");
    let ridx_file = temp.path().join("app.ridx");
    builder.write_to_file(&ridx_file);
    mr.add_root(ridx_file);

    let uri = format!("file://{}", file_path.display());
    ov.open_with_provenance(&uri, file_path.to_str().unwrap(), 1, "fn check() { item(); }\n".to_string(), true);

    let range = json!({ "start": { "line": 0, "character": 13 }, "end": { "line": 0, "character": 17 } });
    let diag = json!({ "range": range, "message": "cannot find value `item` in this scope", "code": "E0425" });
    let res = send_code_action(&uri, range, json!({ "only": ["quickfix"], "diagnostics": [diag] }), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["id"], 100);
    let actions = res["result"].as_array().expect("actions array");
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0]["title"], "Import `app::item`");
}

#[test]
fn test_code_action_auto_import_filtered_out_by_only_refactor() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/auto_imp_filter.rs";
    let file = "/tmp/auto_imp_filter.rs";
    ov.open_with_provenance(uri, file, 1, "fn run() { missing(); }\n".to_string(), true);

    let range = json!({ "start": { "line": 0, "character": 11 }, "end": { "line": 0, "character": 18 } });
    let diag = json!({ "range": range, "message": "cannot find value `missing` in this scope", "code": "E0425" });
    let res = send_code_action(uri, range, json!({ "only": ["refactor"], "diagnostics": [diag] }), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["id"], 100);
    let actions = res["result"].as_array().expect("actions array");
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0]["kind"], "refactor.extract");
    assert_eq!(actions[0]["title"], "Extract variable");
}

#[test]
fn test_code_action_auto_import_no_matching_symbol_yields_no_action() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/no_match.rs";
    let file = "/tmp/no_match.rs";
    ov.open_with_provenance(uri, file, 1, "fn run() { unknown(); }\n".to_string(), true);

    let range = json!({ "start": { "line": 0, "character": 11 }, "end": { "line": 0, "character": 18 } });
    let diag = json!({ "range": range, "message": "cannot find value `TotallyUnknownItem` in this scope", "code": "E0425" });
    let res = send_code_action(uri, range, json!({ "only": ["quickfix"], "diagnostics": [diag] }), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["id"], 100);
    assert_eq!(res["result"], json!([]));
}

#[test]
fn test_code_action_combined_organize_and_quickfix() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/comb_both.rs";
    let file = "/tmp/comb_both.rs";
    ov.open_with_provenance(uri, file, 1, "use b::B;\nuse a::A;\nfn main() { let x = 1; }\n".to_string(), true);

    let range = json!({ "start": { "line": 2, "character": 16 }, "end": { "line": 2, "character": 17 } });
    let diag = json!({ "range": range, "message": "unused variable: `x`", "code": "unused_variable" });
    let res = send_code_action(uri, range, json!({ "diagnostics": [diag] }), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["id"], 100);
    let actions = res["result"].as_array().expect("actions array");
    assert_eq!(actions.len(), 3);
    assert_eq!(actions[0]["title"], "Organize Imports");
    assert_eq!(actions[1]["title"], "Extract variable");
    assert_eq!(actions[2]["title"], "Prefix with underscore: `_x`");
}

#[test]
fn test_code_action_combined_filtered_by_only_quickfix() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/comb_qf.rs";
    let file = "/tmp/comb_qf.rs";
    ov.open_with_provenance(uri, file, 1, "use b::B;\nuse a::A;\nfn main() { let x = 1; }\n".to_string(), true);

    let range = json!({ "start": { "line": 2, "character": 16 }, "end": { "line": 2, "character": 17 } });
    let diag = json!({ "range": range, "message": "unused variable: `x`", "code": "unused_variable" });
    let res = send_code_action(uri, range, json!({ "only": ["quickfix"], "diagnostics": [diag] }), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["id"], 100);
    let actions = res["result"].as_array().expect("actions array");
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0]["title"], "Prefix with underscore: `_x`");
}

#[test]
fn test_code_action_combined_filtered_by_only_source() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/comb_src.rs";
    let file = "/tmp/comb_src.rs";
    ov.open_with_provenance(uri, file, 1, "use b::B;\nuse a::A;\nfn main() { let x = 1; }\n".to_string(), true);

    let range = json!({ "start": { "line": 2, "character": 16 }, "end": { "line": 2, "character": 17 } });
    let diag = json!({ "range": range, "message": "unused variable: `x`", "code": "unused_variable" });
    let res = send_code_action(uri, range, json!({ "only": ["source"], "diagnostics": [diag] }), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["id"], 100);
    let actions = res["result"].as_array().expect("actions array");
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0]["title"], "Organize Imports");
}

#[test]
fn test_code_action_zero_width_range_at_eof() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/eof_action.rs";
    let file = "/tmp/eof_action.rs";
    ov.open_with_provenance(uri, file, 1, "fn main() {}\n".to_string(), true);

    let range = json!({ "start": { "line": 1, "character": 0 }, "end": { "line": 1, "character": 0 } });
    let res = send_code_action(uri, range, json!({ "diagnostics": [] }), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["id"], 100);
    assert_eq!(res["result"], json!([]));
}

#[test]
fn test_code_action_response_id_echoed_correctly() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/echo_id.rs";
    let file = "/tmp/echo_id.rs";
    ov.open_with_provenance(uri, file, 1, "fn f() {}\n".to_string(), true);

    let mut out = Vec::new();
    let req = json!({
        "jsonrpc": "2.0",
        "id": 4242,
        "method": "textDocument/codeAction",
        "params": {
            "textDocument": { "uri": uri },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
            "context": { "diagnostics": [] }
        }
    });
    dispatch_lsp_message(LspMessage::Client(req), &mut out, &mut lc, &mut mr, &mut ov, &can, None).unwrap();
    let res = extract_response(&out);
    assert_eq!(res["id"], 4242);
}

#[test]
fn test_code_action_jsonrpc_version_2_0_echoed() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/echo_jsonrpc.rs";
    let file = "/tmp/echo_jsonrpc.rs";
    ov.open_with_provenance(uri, file, 1, "fn g() {}\n".to_string(), true);

    let range = json!({ "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } });
    let res = send_code_action(uri, range, json!({ "diagnostics": [] }), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["jsonrpc"], "2.0");
}

#[test]
fn test_code_action_null_context_only_matches_all_kinds() {
    let (mut mr, mut ov, mut lc, can) = setup_initialized_server();
    let uri = "file:///tmp/null_only.rs";
    let file = "/tmp/null_only.rs";
    ov.open_with_provenance(uri, file, 1, "use b::B;\nuse a::A;\n".to_string(), true);

    let range = json!({ "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } });
    let res = send_code_action(uri, range, json!({ "only": null, "diagnostics": [] }), &mut mr, &mut ov, &mut lc, &can);
    assert_eq!(res["id"], 100);
    let actions = res["result"].as_array().expect("actions array");
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0]["title"], "Organize Imports");
}
