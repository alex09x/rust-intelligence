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

    let actions = handle_code_action(&req, &overlay_mgr);
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

    let res = handle_execute_command(&req, &mut overlay_mgr);
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

    let edit_cmd = json!({
        "jsonrpc": "2.0",
        "id": 42,
        "method": "workspace/executeCommand",
        "params": {
            "command": "ridx.applyEdits",
            "arguments": [{
                "changes": {
                    uri.clone(): [{
                        "range": { "start": { "line": 0, "character": 3 }, "end": { "line": 0, "character": 7 } },
                        "newText": "renamed"
                    }]
                }
            }]
        }
    });

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
    assert_eq!(msgs[0]["id"], 9999);

    let reject_resp = json!({
        "jsonrpc": "2.0",
        "id": 9999,
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
        "Overlay must be rolled back on client rejection"
    );
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
