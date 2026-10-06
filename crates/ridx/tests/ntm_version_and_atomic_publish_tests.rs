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
use ridx::lsp::editing::commit_closed_writes_atomic;
use ridx::lsp::lifecycle::LifecycleManager;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
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
fn test_outbound_apply_edit_preserves_document_changes_version() {
    let mut overlay_mgr = OverlayManager::new();
    let file = "/tmp/test_ver.rs";
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

    let cmd = json!({
        "jsonrpc": "2.0",
        "id": 100,
        "method": "workspace/executeCommand",
        "params": {
            "command": "ridx.applyEdits",
            "arguments": [{
                "documentChanges": [{
                    "textDocument": { "uri": uri.clone(), "version": 1 },
                    "edits": [{
                        "range": { "start": { "line": 0, "character": 3 }, "end": { "line": 0, "character": 6 } },
                        "newText": "bar"
                    }]
                }]
            }]
        }
    });

    let mut cmd_out = Cursor::new(Vec::new());
    let _ = dispatch_lsp_message(
        cmd,
        &mut cmd_out,
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        None,
    );

    let msgs = parse_framed_json(&cmd_out.into_inner());
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["method"], "workspace/applyEdit");

    let edit = &msgs[0]["params"]["edit"];
    let doc_changes = edit["documentChanges"]
        .as_array()
        .expect("Must preserve documentChanges structure");
    assert_eq!(doc_changes.len(), 1);
    assert_eq!(doc_changes[0]["textDocument"]["uri"], uri);
    assert_eq!(
        doc_changes[0]["textDocument"]["version"], 1,
        "Outbound applyEdit must preserve client document version"
    );
}

#[test]
fn test_client_ack_does_not_rewrite_closed_file_on_disk() {
    let pid = std::process::id();
    let temp_file = std::env::temp_dir().join(format!("test_ack_rewrite_{pid}.rs"));
    let _ = fs::write(&temp_file, "fn foo() {}\n");
    let uri = format!("file://{}", temp_file.display());

    let mut overlay_mgr = OverlayManager::new();
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

    let cmd = json!({
        "jsonrpc": "2.0",
        "id": 200,
        "method": "workspace/executeCommand",
        "params": {
            "command": "ridx.applyEdits",
            "arguments": [{
                "changes": {
                    uri.clone(): [{
                        "range": { "start": { "line": 0, "character": 3 }, "end": { "line": 0, "character": 6 } },
                        "newText": "bar"
                    }]
                }
            }]
        }
    });

    let mut cmd_out = Cursor::new(Vec::new());
    let _ = dispatch_lsp_message(
        cmd,
        &mut cmd_out,
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        None,
    );

    let msgs = parse_framed_json(&cmd_out.into_inner());
    let apply_id = msgs[0]["id"].clone();

    // Client applies the change, writes extra comment, and saves to disk
    let client_text = "fn bar() {} // updated by client\n";
    fs::write(&temp_file, client_text).unwrap();

    // Client responds applied: true
    let ack_msg = json!({
        "jsonrpc": "2.0",
        "id": apply_id,
        "result": { "applied": true }
    });

    let mut ack_out = Cursor::new(Vec::new());
    let _ = dispatch_lsp_message(
        ack_msg,
        &mut ack_out,
        &mut lifecycle,
        &mut multi_root,
        &mut overlay_mgr,
        &cancelled,
        None,
    );

    // Verify disk content was NOT overwritten with stale server snapshot
    let current_disk = fs::read_to_string(&temp_file).unwrap();
    assert_eq!(
        current_disk, client_text,
        "Server must not clobber client's file on ack"
    );

    let _ = fs::remove_file(temp_file);
}

#[test]
fn test_atomic_write_preserves_file_permissions() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let pid = std::process::id();
        let temp_file = std::env::temp_dir().join(format!("test_perm_{pid}.rs"));
        let _ = fs::write(&temp_file, "script content\n");
        let orig_mode = 0o755;
        fs::set_permissions(&temp_file, fs::Permissions::from_mode(orig_mode)).unwrap();

        let ok = commit_closed_writes_atomic(&[(
            temp_file.to_str().unwrap().to_string(),
            "updated script\n".to_string(),
        )]);
        assert!(ok, "Atomic commit must succeed");

        let meta = fs::metadata(&temp_file).unwrap();
        let current_mode = meta.permissions().mode() & 0o777;
        assert_eq!(
            current_mode, orig_mode,
            "Atomic publish must preserve original file mode"
        );

        let _ = fs::remove_file(temp_file);
    }
}

#[test]
fn test_atomic_write_preserves_symlink_referent() {
    #[cfg(unix)]
    {
        let pid = std::process::id();
        let target_file = std::env::temp_dir().join(format!("test_sym_target_{pid}.rs"));
        let link_file = std::env::temp_dir().join(format!("test_sym_link_{pid}.rs"));

        let _ = fs::write(&target_file, "original target\n");
        let _ = fs::remove_file(&link_file);
        std::os::unix::fs::symlink(&target_file, &link_file).unwrap();

        let ok = commit_closed_writes_atomic(&[(
            link_file.to_str().unwrap().to_string(),
            "modified target\n".to_string(),
        )]);
        assert!(ok, "Atomic commit via symlink must succeed");

        let link_meta = fs::symlink_metadata(&link_file).unwrap();
        assert!(
            link_meta.file_type().is_symlink(),
            "Link file must remain a symlink"
        );

        let target_content = fs::read_to_string(&target_file).unwrap();
        assert_eq!(
            target_content, "modified target\n",
            "Referent file must receive the updated content"
        );

        let _ = fs::remove_file(link_file);
        let _ = fs::remove_file(target_file);
    }
}
