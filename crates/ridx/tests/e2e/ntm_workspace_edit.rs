/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use ridx::lsp::editing::commands::handle_execute_command;
use ridx::lsp::editing::ident::apply_workspace_text_edits_to_content;
use ridx::overlay::OverlayManager;
use serde_json::json;
use std::fs;

#[test]
fn test_same_position_inserts_preserve_declared_order() {
    let content = "x";
    let edits = vec![
        json!({
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
            "newText": "A"
        }),
        json!({
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
            "newText": "B"
        }),
    ];
    let result = apply_workspace_text_edits_to_content(content, &edits).expect("apply edits");
    assert_eq!(
        result, "ABx",
        "Array order must be preserved: A, then B gives ABx"
    );
}

#[test]
fn test_partial_workspace_edit_rejected_atomically() {
    let mut overlay_mgr = OverlayManager::new();
    let uri_a = "file:///tmp/doc_a.rs";
    let uri_b = "file:///tmp/doc_b.rs";
    let file_a = "/tmp/doc_a.rs";
    let file_b = "/tmp/doc_b.rs";

    overlay_mgr.open_document(uri_a, file_a, 1, "foo".to_string());
    overlay_mgr.open_document(uri_b, file_b, 5, "foo".to_string());

    let edit_req = json!({
        "params": {
            "command": "ridx.applyEdits",
            "arguments": [
                {
                    "documentChanges": [
                        {
                            "textDocument": { "uri": uri_a, "version": 1 },
                            "edits": [
                                {
                                    "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 3 } },
                                    "newText": "bar"
                                }
                            ]
                        },
                        {
                            "textDocument": { "uri": uri_b, "version": 1 },
                            "edits": [
                                {
                                    "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 3 } },
                                    "newText": "bar"
                                }
                            ]
                        }
                    ]
                }
            ]
        }
    });

    let res = handle_execute_command(&edit_req, &mut overlay_mgr).expect("command succeeds");
    assert_eq!(
        res["applied"], false,
        "Multi-doc edit with one stale version must fail completely"
    );
    assert_eq!(res["count"], 0);

    let overlay_a = overlay_mgr.get(file_a).unwrap();
    assert_eq!(overlay_a.text, "foo", "Document A must remain untouched");
    assert_eq!(overlay_a.version, 1);

    let overlay_b = overlay_mgr.get(file_b).unwrap();
    assert_eq!(overlay_b.text, "foo", "Document B must remain untouched");
    assert_eq!(overlay_b.version, 5);
}

#[test]
fn test_open_document_command_edits_sync_disk_and_return_payload() {
    let mut overlay_mgr = OverlayManager::new();
    let temp_file = std::env::temp_dir().join(format!("test_open_sync_{}.rs", std::process::id()));
    let _ = fs::write(&temp_file, "fn foo() {}");
    let file = temp_file.to_str().unwrap();
    let uri = format!("file://{}", temp_file.display());

    overlay_mgr.open_document(&uri, file, 1, "fn foo() {}".to_string());

    let edit_req = json!({
        "params": {
            "command": "ridx.applyEdits",
            "arguments": [
                {
                    "changes": {
                        &uri: [
                            {
                                "range": { "start": { "line": 0, "character": 3 }, "end": { "line": 0, "character": 6 } },
                                "newText": "bar"
                            }
                        ]
                    }
                }
            ]
        }
    });

    let res = handle_execute_command(&edit_req, &mut overlay_mgr).expect("command succeeds");
    assert_eq!(res["applied"], true);
    assert_eq!(res["count"], 1);
    assert!(
        res.get("edit").is_some(),
        "Response must return WorkspaceEdit payload"
    );

    let overlay = overlay_mgr.get(file).unwrap();
    assert_eq!(overlay.text, "fn bar() {}");

    let disk_text = fs::read_to_string(&temp_file).unwrap();
    assert_eq!(
        disk_text, "fn foo() {}",
        "Disk file must NOT be updated for open overlay, preserving dirty draft on disk"
    );

    let _ = fs::remove_file(temp_file);
}

#[test]
fn test_apply_workspace_edit_with_newtext() {
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/test_apply.rs";
    let file = "/tmp/test_apply.rs";
    overlay_mgr.open_document(uri, file, 1, "fn foo() {}\n".to_string());

    let edit_req = json!({
        "params": {
            "command": "ridx.applyEdits",
            "arguments": [
                {
                    "changes": {
                        uri: [
                            {
                                "range": { "start": { "line": 0, "character": 3 }, "end": { "line": 0, "character": 6 } },
                                "newText": "bar"
                            }
                        ]
                    }
                }
            ]
        }
    });

    let res = handle_execute_command(&edit_req, &mut overlay_mgr).expect("command succeeds");
    assert_eq!(res["applied"], true);
    assert_eq!(res["count"], 1);
    let overlay = overlay_mgr.get(file).unwrap();
    assert_eq!(overlay.text, "fn bar() {}\n");
    assert!(overlay.version >= 2);
}

#[test]
fn test_review_v4651_edits_use_original_coordinates() {
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/test_orig_coords.rs";
    let file = "/tmp/test_orig_coords.rs";
    overlay_mgr.open_document(uri, file, 1, "alpha beta".to_string());

    let edit_req = json!({
        "params": {
            "command": "ridx.applyEdits",
            "arguments": [
                {
                    "changes": {
                        uri: [
                            {
                                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 5 } },
                                "newText": "a"
                            },
                            {
                                "range": { "start": { "line": 0, "character": 6 }, "end": { "line": 0, "character": 10 } },
                                "newText": "b"
                            }
                        ]
                    }
                }
            ]
        }
    });

    let res = handle_execute_command(&edit_req, &mut overlay_mgr).expect("command succeeds");
    assert_eq!(res["applied"], true);
    let overlay = overlay_mgr.get(file).unwrap();
    assert_eq!(overlay.text, "a b");
}

#[test]
fn test_review_v4651_dual_edit_representation_applied_once() {
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/test_dual_rep.rs";
    let file = "/tmp/test_dual_rep.rs";
    overlay_mgr.open_document(uri, file, 1, "fn foo() {}".to_string());

    let edit_req = json!({
        "params": {
            "command": "ridx.applyEdits",
            "arguments": [
                {
                    "documentChanges": [
                        {
                            "textDocument": { "uri": uri, "version": 1 },
                            "edits": [
                                {
                                    "range": { "start": { "line": 0, "character": 3 }, "end": { "line": 0, "character": 6 } },
                                    "newText": "bar"
                                }
                            ]
                        }
                    ],
                    "changes": {
                        uri: [
                            {
                                "range": { "start": { "line": 0, "character": 3 }, "end": { "line": 0, "character": 6 } },
                                "newText": "bar"
                            }
                        ]
                    }
                }
            ]
        }
    });

    let res = handle_execute_command(&edit_req, &mut overlay_mgr).expect("command succeeds");
    assert_eq!(res["applied"], true);
    assert_eq!(
        res["count"], 1,
        "Only documentChanges should be applied, not both"
    );
    let overlay = overlay_mgr.get(file).unwrap();
    assert_eq!(overlay.text, "fn bar() {}");
}

#[test]
fn test_review_v4651_stale_version_rejected() {
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/test_stale_ver.rs";
    let file = "/tmp/test_stale_ver.rs";
    overlay_mgr.open_document(uri, file, 7, "fn foo() {}".to_string());

    let stale_req = json!({
        "params": {
            "command": "ridx.applyEdits",
            "arguments": [
                {
                    "documentChanges": [
                        {
                            "textDocument": { "uri": uri, "version": 1 },
                            "edits": [
                                {
                                    "range": { "start": { "line": 0, "character": 3 }, "end": { "line": 0, "character": 6 } },
                                    "newText": "bar"
                                }
                            ]
                        }
                    ]
                }
            ]
        }
    });

    let res = handle_execute_command(&stale_req, &mut overlay_mgr).expect("command succeeds");
    assert_eq!(res["applied"], false, "Stale version edit must be rejected");
    let overlay = overlay_mgr.get(file).unwrap();
    assert_eq!(overlay.text, "fn foo() {}");
    assert_eq!(overlay.version, 7, "Version must not rollback");
}

#[test]
fn test_review_v4651_closed_edit_remains_dirty() {
    let mut overlay_mgr = OverlayManager::new();
    let temp_file = std::env::temp_dir().join(format!("test_closed_{}.rs", std::process::id()));
    let _ = fs::write(&temp_file, "fn foo() {}");
    let uri = format!("file://{}", temp_file.display());
    let file = temp_file.to_str().unwrap();

    let edit_req = json!({
        "params": {
            "command": "ridx.applyEdits",
            "arguments": [
                {
                    "changes": {
                        &uri: [
                            {
                                "range": { "start": { "line": 0, "character": 3 }, "end": { "line": 0, "character": 6 } },
                                "newText": "bar"
                            }
                        ]
                    }
                }
            ]
        }
    });

    let res = handle_execute_command(&edit_req, &mut overlay_mgr).expect("command succeeds");
    assert_eq!(res["applied"], true);
    let overlay = overlay_mgr.get(file).unwrap();
    assert_eq!(overlay.text, "fn bar() {}");
    assert!(
        overlay.is_dirty(),
        "Closed file edited on disk must remain dirty relative to compiled index"
    );
    let _ = fs::remove_file(temp_file);
}

#[test]
fn test_multiple_arguments_preserve_all_edits() {
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/multi_arg.rs";
    let file = "/tmp/multi_arg.rs";
    overlay_mgr.open_document(uri, file, 1, "xy".to_string());

    let edit_req = json!({
        "params": {
            "command": "ridx.applyEdits",
            "arguments": [
                { "changes": { uri: [{ "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 1 } }, "newText": "A" }] } },
                { "changes": { uri: [{ "range": { "start": { "line": 0, "character": 1 }, "end": { "line": 0, "character": 2 } }, "newText": "B" }] } }
            ]
        }
    });

    let res = handle_execute_command(&edit_req, &mut overlay_mgr).expect("command succeeds");
    assert_eq!(res["applied"], true);
    assert_eq!(res["count"], 2);

    let overlay = overlay_mgr.get(file).unwrap();
    assert_eq!(
        overlay.text, "AB",
        "Multiple arguments must sequentially transform the document to AB"
    );
}

#[test]
fn test_disk_failure_rollback_and_reported_failure() {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    let mut overlay_mgr = OverlayManager::new();
    let temp_file = std::env::temp_dir().join(format!("test_ro_{}.rs", std::process::id()));
    let _ = fs::write(&temp_file, "orig");
    #[cfg(unix)]
    let _ = fs::set_permissions(&temp_file, fs::Permissions::from_mode(0o444));
    let uri = format!("file://{}", temp_file.display());
    let _file = temp_file.to_str().unwrap();

    let edit_req = json!({
        "params": {
            "command": "ridx.applyEdits",
            "arguments": [{ "changes": { &uri: [{ "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 4 } }, "newText": "modified" }] } }]
        }
    });

    let res = handle_execute_command(&edit_req, &mut overlay_mgr).expect("command succeeds");
    #[cfg(unix)]
    {
        assert_eq!(
            res["applied"], false,
            "Disk write failure must report applied: false"
        );
        let _ = fs::set_permissions(&temp_file, std::fs::Permissions::from_mode(0o644));
    }
    let _ = fs::remove_file(temp_file);
}
