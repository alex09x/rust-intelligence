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
use ridx::lsp::editing::{handle_code_action, handle_execute_command};
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use ridx_format::{IndexWriter, WorkspaceIndex};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs;
use std::io::Cursor;
use std::sync::{Arc, Mutex};

fn parse_framed_json(bytes: &[u8]) -> Vec<Value> {
    let mut out = Vec::new();
    let text = String::from_utf8_lossy(bytes);
    let mut rest = &text[..];
    while let Some(pos) = rest.find("\r\n\r\n") {
        let body = &rest[pos + 4..];
        let len_header = &rest[..pos];
        let len: usize = len_header
            .lines()
            .find_map(|l| l.strip_prefix("Content-Length: "))
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);
        if body.len() >= len && len > 0 {
            if let Ok(val) = serde_json::from_str::<Value>(&body[..len]) {
                out.push(val);
            }
            rest = &body[len..];
        } else {
            break;
        }
    }
    out
}

fn make_apply_cmd(id: i64, uri: &str, new_text: &str) -> Value {
    json!({
        "jsonrpc": "2.0", "id": id, "method": "workspace/executeCommand",
        "params": {
            "command": "ridx.applyEdits",
            "arguments": [{
                "changes": { uri: [{ "range": { "start": { "line": 0, "character": 3 }, "end": { "line": 0, "character": 6 } }, "newText": new_text }] }
            }]
        }
    })
}

#[test]
fn test_extract_variable_inside_closure_rejected() {
    let mut overlay_mgr = OverlayManager::new();
    let file = "/tmp/test_closure_scope.rs";
    let uri = format!("file://{file}");
    let code = "fn run() {\n    let c = |x: i32| x + 1;\n}\n";
    overlay_mgr.open_with_provenance(&uri, file, 1, code.to_string(), true);

    let req = json!({
        "params": {
            "textDocument": { "uri": uri },
            "range": {
                "start": { "line": 1, "character": 21 },
                "end": { "line": 1, "character": 26 }
            },
            "context": { "diagnostics": [] }
        }
    });

    let actions = handle_code_action(&req, None, &overlay_mgr).expect("code action succeeds");
    let arr = actions.as_array().expect("code action array");
    for action in arr {
        let kind = action.get("kind").and_then(|k| k.as_str()).unwrap_or("");
        assert_ne!(
            kind, "refactor.extract.variable",
            "Variable extraction must not escape closure binding scope"
        );
    }
}

#[test]
fn test_container_matching_exact_components() {
    fn matches_container(container: &str, tc: &str) -> bool {
        container == tc || container.ends_with(&format!("::{tc}"))
    }

    assert!(!matches_container("notmod", "mod"));
    assert!(matches_container("mod", "mod"));
    assert!(matches_container("foo::mod", "mod"));
    assert!(matches_container("crate::sub::mod", "mod"));
    assert!(!matches_container("sub_model", "mod"));
}

#[test]
fn test_sequential_workspace_edit_client_snapshot_consistency() {
    let mut overlay_mgr = OverlayManager::new();
    let file = "/tmp/test_sequential_snapshot.rs";
    let uri = format!("file://{file}");
    let initial_text = "abcdef\n";
    overlay_mgr.open_with_provenance(&uri, file, 1, initial_text.to_string(), true);

    let req = json!({
        "params": {
            "command": "ridx.applyEdits",
            "arguments": [
                {
                    "changes": {
                        uri.clone(): [
                            {
                                "range": { "start": { "line": 0, "character": 1 }, "end": { "line": 0, "character": 2 } },
                                "newText": "B"
                            }
                        ]
                    }
                },
                {
                    "changes": {
                        uri.clone(): [
                            {
                                "range": { "start": { "line": 0, "character": 3 }, "end": { "line": 0, "character": 4 } },
                                "newText": "D"
                            }
                        ]
                    }
                }
            ]
        }
    });

    let res = handle_execute_command(&req, &mut overlay_mgr).expect("command succeeds");
    assert_eq!(res["applied"], true);
    assert_eq!(res["count"], 2);

    let server_doc = overlay_mgr.get(file).unwrap();
    assert_eq!(server_doc.text, "aBcDef\n");

    let changes = res["edit"]["changes"][&uri].as_array().unwrap();
    assert_eq!(
        changes.len(),
        1,
        "Multi-arg edit must produce single full-doc replacement"
    );
    assert_eq!(changes[0]["newText"], "aBcDef\n");
}

#[test]
fn test_apply_edit_two_way_handshake_reject_and_accept() {
    let mut overlay_mgr = OverlayManager::new();
    let file = "/tmp/test_handshake.rs";
    let uri = format!("file://{file}");
    let initial_text = "fn main() {}\n";
    overlay_mgr.open_with_provenance(&uri, file, 1, initial_text.to_string(), true);

    let mut lifecycle = LifecycleManager::new();
    let init_req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "capabilities": {} }
    });
    let mut out = Cursor::new(Vec::new());
    let mut multi_root = MultiRootWorkspace::new();
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    let _ = dispatch_lsp_message(
        init_req,
        &mut out,
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        None,
    );

    let edit_cmd = make_apply_cmd(42, &uri, "renamed");

    let mut cmd_out = Cursor::new(Vec::new());
    let _ = dispatch_lsp_message(
        edit_cmd,
        &mut cmd_out,
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        None,
    );

    let msgs = parse_framed_json(&cmd_out.into_inner());
    assert_eq!(
        msgs.len(),
        1,
        "Must emit workspace/applyEdit without command response yet"
    );
    assert_eq!(msgs[0]["method"], "workspace/applyEdit");
    let apply_id = msgs[0]["id"].clone();
    assert!(apply_id.as_u64().is_some());

    let reject_resp = json!({
        "jsonrpc": "2.0",
        "id": apply_id,
        "result": { "applied": false }
    });

    let mut resp_out = Cursor::new(Vec::new());
    let _ = dispatch_lsp_message(
        reject_resp,
        &mut resp_out,
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        None,
    );

    let final_msgs = parse_framed_json(&resp_out.into_inner());
    assert_eq!(
        final_msgs.len(),
        1,
        "Must complete command with client rejection"
    );
    assert_eq!(final_msgs[0]["id"], 42);
    assert_eq!(final_msgs[0]["result"]["applied"], false);
    assert_eq!(final_msgs[0]["result"]["failure"], "client_rejected");

    let overlay = overlay_mgr.get(file).unwrap();
    assert_eq!(
        overlay.text, initial_text,
        "Overlay must remain untouched on client rejection"
    );
}

#[test]
fn test_two_concurrent_pending_edits_distinct_ids() {
    let mut overlay_mgr = OverlayManager::new();
    let file = "/tmp/test_concurrent.rs";
    let uri = format!("file://{file}");
    overlay_mgr.open_with_provenance(&uri, file, 1, "fn foo() {}\n".to_string(), true);

    let mut lifecycle = LifecycleManager::new();
    let mut out = Cursor::new(Vec::new());
    let mut multi_root = MultiRootWorkspace::new();
    let cancelled = Arc::new(Mutex::new(HashSet::new()));

    let _ = dispatch_lsp_message(
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } }),
        &mut out,
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        None,
    );

    let cmd1 = make_apply_cmd(101, &uri, "bar");
    let mut out1 = Cursor::new(Vec::new());
    let _ = dispatch_lsp_message(
        cmd1,
        &mut out1,
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        None,
    );
    let msgs1 = parse_framed_json(&out1.into_inner());
    assert_eq!(msgs1.len(), 1);
    let id1 = msgs1[0]["id"].clone();

    let cmd2 = make_apply_cmd(102, &uri, "baz");
    let mut out2 = Cursor::new(Vec::new());
    let _ = dispatch_lsp_message(
        cmd2,
        &mut out2,
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        None,
    );
    let msgs2 = parse_framed_json(&out2.into_inner());
    assert_eq!(msgs2.len(), 1);
    let id2 = msgs2[0]["id"].clone();

    assert_ne!(
        id1, id2,
        "Concurrent commands must have distinct applyEdit request IDs"
    );

    // Reply to id1:
    let mut resp_out1 = Cursor::new(Vec::new());
    let _ = dispatch_lsp_message(
        json!({ "jsonrpc": "2.0", "id": id1, "result": { "applied": true } }),
        &mut resp_out1,
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        None,
    );
    let final1 = parse_framed_json(&resp_out1.into_inner());
    assert_eq!(final1.len(), 1);
    assert_eq!(final1[0]["id"], 101);
    assert_eq!(final1[0]["result"]["applied"], true);

    // Reply to id2:
    let mut resp_out2 = Cursor::new(Vec::new());
    let _ = dispatch_lsp_message(
        json!({ "jsonrpc": "2.0", "id": id2, "result": { "applied": false } }),
        &mut resp_out2,
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        None,
    );
    let final2 = parse_framed_json(&resp_out2.into_inner());
    assert_eq!(final2.len(), 1);
    assert_eq!(final2[0]["id"], 102);
    assert_eq!(final2[0]["result"]["applied"], false);
}

#[test]
fn test_atomic_closed_write_preserves_large_file_on_error() {
    let tmp = std::env::temp_dir().join(format!("ridx_atomic_{}", std::process::id()));
    let _ = fs::create_dir_all(&tmp);
    let target = tmp.join("original.rs");
    let original_content = "X".repeat(256);
    fs::write(&target, &original_content).unwrap();

    let invalid_target = "/invalid_nonexistent_dir_cannot_write/should_fail.rs".to_string();
    let writes = vec![
        (
            target.to_str().unwrap().to_string(),
            "new content".to_string(),
        ),
        (invalid_target, "fail".to_string()),
    ];

    let ok = ridx::lsp::editing::commit_closed_writes_atomic(&writes);
    assert!(!ok, "Batch write with failing path must return false");

    let restored = fs::read_to_string(&target).unwrap();
    assert_eq!(
        restored, original_content,
        "Original file must be 100% preserved without truncation"
    );
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_workspace_index_source_root_at() {
    let tmp = std::env::temp_dir().join(format!("ridx_src_root_{}", std::process::id()));
    let _ = fs::create_dir_all(&tmp);
    let ridx_file = tmp.join("test.ridx");
    let mut writer = IndexWriter::new("test_crate", "0.1.0");
    let mut f = fs::File::create(&ridx_file).unwrap();
    writer.write_to(&mut f).unwrap();

    let ws = WorkspaceIndex::open(&tmp);
    assert!(ws.is_ok());
    let ws = ws.unwrap();
    assert_eq!(ws.source_root_at(0), None);
    assert_eq!(ws.source_root_at(999), None);
    let _ = fs::remove_dir_all(&tmp);
}
