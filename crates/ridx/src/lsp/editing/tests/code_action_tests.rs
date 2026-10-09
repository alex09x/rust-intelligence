/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Unit tests for textDocument/codeAction protocol handler.

use super::setup_test_editing;
use crate::lsp::editing::actions::handle_code_action;
use crate::lsp::error::{INVALID_PARAMS, REQUEST_FAILED};
use crate::overlay::OverlayManager;
use ridx_test_support::apply_text_edits;
use serde_json::json;

#[test]
fn test_code_action_missing_params_rejected() {
    let overlay_mgr = OverlayManager::new();
    let req = json!({ "jsonrpc": "2.0", "id": 1, "method": "textDocument/codeAction" });
    let err = handle_code_action(&req, None, &overlay_mgr).unwrap_err();
    assert_eq!(err.code, INVALID_PARAMS);
}

#[test]
fn test_code_action_missing_uri_rejected() {
    let overlay_mgr = OverlayManager::new();
    let req = json!({
        "params": {
            "textDocument": {},
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } }
        }
    });
    let err = handle_code_action(&req, None, &overlay_mgr).unwrap_err();
    assert_eq!(err.code, INVALID_PARAMS);
}

#[test]
fn test_code_action_empty_uri_rejected() {
    let overlay_mgr = OverlayManager::new();
    let req = json!({
        "params": {
            "textDocument": { "uri": "" },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } }
        }
    });
    let err = handle_code_action(&req, None, &overlay_mgr).unwrap_err();
    assert_eq!(err.code, INVALID_PARAMS);
}

#[test]
fn test_code_action_missing_range_rejected() {
    let overlay_mgr = OverlayManager::new();
    let req = json!({
        "params": {
            "textDocument": { "uri": "file:///tmp/test.rs" }
        }
    });
    let err = handle_code_action(&req, None, &overlay_mgr).unwrap_err();
    assert_eq!(err.code, INVALID_PARAMS);
}

#[test]
fn test_code_action_external_dependency_rejected() {
    let (_temp, ws, overlay_mgr, _conv) = setup_test_editing();
    let ext_uri = "file:///.cargo/registry/src/ext_service-0.5.0/src/lib.rs";
    let req = json!({
        "params": {
            "textDocument": { "uri": ext_uri },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
            "context": { "diagnostics": [] }
        }
    });
    let err = handle_code_action(&req, Some(&ws), &overlay_mgr).unwrap_err();
    assert_eq!(err.code, REQUEST_FAILED);
    assert!(err.message.contains("external dependency"));
}

#[test]
fn test_code_action_source_organize_imports_filtering() {
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/organize_filter.rs";
    let code = "use z::Z;\nuse a::A;\nfn main() {}\n";
    overlay_mgr.open_document(uri, "/tmp/organize_filter.rs", 1, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": uri },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
            "context": {
                "only": ["source.organizeImports"],
                "diagnostics": []
            }
        }
    });
    let res = handle_code_action(&req, None, &overlay_mgr).unwrap();
    let actions = res.as_array().unwrap();
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0]["kind"], "source.organizeImports");
    assert_eq!(actions[0]["title"], "Organize Imports");
}

#[test]
fn test_code_action_refactor_extract_empty_range_ignored() {
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/extract_empty.rs";
    let code = "fn main() { let x = 1 + 2; }\n";
    overlay_mgr.open_document(uri, "/tmp/extract_empty.rs", 1, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": uri },
            "range": { "start": { "line": 0, "character": 20 }, "end": { "line": 0, "character": 20 } },
            "context": {
                "only": ["refactor.extract"],
                "diagnostics": []
            }
        }
    });
    let res = handle_code_action(&req, None, &overlay_mgr).unwrap();
    let actions = res.as_array().unwrap();
    assert!(actions.is_empty(), "Empty range must not propose extract variable");
}

#[test]
fn test_code_action_quickfix_unused_variable() {
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/quickfix_unused.rs";
    let code = "fn main() { let foo = 42; }\n";
    overlay_mgr.open_document(uri, "/tmp/quickfix_unused.rs", 1, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": uri },
            "range": { "start": { "line": 0, "character": 16 }, "end": { "line": 0, "character": 19 } },
            "context": {
                "only": ["quickfix"],
                "diagnostics": [{
                    "code": "unused_variable",
                    "message": "unused variable: `foo`",
                    "range": { "start": { "line": 0, "character": 16 }, "end": { "line": 0, "character": 19 } }
                }]
            }
        }
    });
    let res = handle_code_action(&req, None, &overlay_mgr).unwrap();
    let actions = res.as_array().unwrap();
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0]["kind"], "quickfix");
    assert_eq!(actions[0]["title"], "Prefix with underscore: `_foo`");
}

#[test]
fn test_code_action_quickfix_unused_mut() {
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/quickfix_mut.rs";
    let code = "fn main() { let mut x = 42; }\n";
    overlay_mgr.open_document(uri, "/tmp/quickfix_mut.rs", 1, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": uri },
            "range": { "start": { "line": 0, "character": 16 }, "end": { "line": 0, "character": 20 } },
            "context": {
                "only": ["quickfix"],
                "diagnostics": [{
                    "code": "unused_mut",
                    "message": "variable does not need to be mutable",
                    "range": { "start": { "line": 0, "character": 16 }, "end": { "line": 0, "character": 20 } }
                }]
            }
        }
    });
    let res = handle_code_action(&req, None, &overlay_mgr).unwrap();
    let actions = res.as_array().unwrap();
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0]["kind"], "quickfix");
    assert_eq!(actions[0]["title"], "Remove unnecessary `mut`");
}

#[test]
fn test_code_action_auto_import_from_workspace_symbol() {
    let (_temp, ws, overlay_mgr, _conv) = setup_test_editing();
    let uri = "file:///tmp/auto_import.rs";

    let req = json!({
        "params": {
            "textDocument": { "uri": uri },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 10 } },
            "context": {
                "only": ["quickfix"],
                "diagnostics": [{
                    "code": "E0425",
                    "message": "cannot find value `add_values` in this scope",
                    "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 10 } }
                }]
            }
        }
    });
    let res = handle_code_action(&req, Some(&ws), &overlay_mgr).unwrap();
    let actions = res.as_array().unwrap();
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0]["kind"], "quickfix");
    assert_eq!(actions[0]["title"], "Import `test_edit::add_values`");
}

#[test]
fn test_code_action_auto_import_multiple_insertions_order() {
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/order_test.rs";
    let code = "fn main() {}\n";
    overlay_mgr.open_document(uri, "/tmp/order_test.rs", 1, code.to_string());

    let edits = vec![
        json!({
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
            "newText": "use alpha::A;\n"
        }),
        json!({
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
            "newText": "use beta::B;\n"
        }),
    ];

    let applied = apply_text_edits(code, &edits);
    let expected = "use alpha::A;\nuse beta::B;\nfn main() {}\n";
    assert_eq!(applied, expected, "Multiple insertions at (0, 0) must preserve sequence order");
}

#[test]
fn test_code_action_includes_data_payload_for_deferred_resolve() {
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/data_payload.rs";
    let code = "use b::B;\nuse a::A;\nfn main() {}\n";
    overlay_mgr.open_document(uri, "/tmp/data_payload.rs", 2, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": uri },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
            "context": { "only": ["source.organizeImports"], "diagnostics": [] }
        }
    });
    let res = handle_code_action(&req, None, &overlay_mgr).unwrap();
    let actions = res.as_array().unwrap();
    let data = &actions[0]["data"];
    assert_eq!(data["uri"], uri);
    assert_eq!(data["version"], 2);
    assert_eq!(data["kind"], "source.organizeImports");
}
