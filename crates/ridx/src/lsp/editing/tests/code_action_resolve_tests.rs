/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Unit tests for codeAction/resolve protocol handler and version conflict safety.

use super::setup_test_editing;
use crate::lsp::editing::actions::handle_code_action_resolve;
use crate::lsp::error::{CONTENT_MODIFIED, INVALID_PARAMS, REQUEST_FAILED};
use crate::overlay::OverlayManager;
use ridx_test_support::apply_workspace_edit_with_versions;
use serde_json::json;
use std::collections::HashMap;

#[test]
fn test_code_action_resolve_missing_params_rejected() {
    let overlay_mgr = OverlayManager::new();
    let req = json!({ "jsonrpc": "2.0", "id": 1, "method": "codeAction/resolve" });
    let err = handle_code_action_resolve(&req, None, &overlay_mgr).unwrap_err();
    assert_eq!(err.code, INVALID_PARAMS);
}

#[test]
fn test_code_action_resolve_missing_title_rejected() {
    let overlay_mgr = OverlayManager::new();
    let req = json!({
        "params": {
            "kind": "source.organizeImports",
            "data": { "uri": "file:///tmp/t.rs", "kind": "source.organizeImports" }
        }
    });
    let err = handle_code_action_resolve(&req, None, &overlay_mgr).unwrap_err();
    assert_eq!(err.code, INVALID_PARAMS);
}

#[test]
fn test_code_action_resolve_passthrough_without_data() {
    let overlay_mgr = OverlayManager::new();
    let req = json!({
        "params": {
            "title": "Do something",
            "kind": "quickfix"
        }
    });
    let res = handle_code_action_resolve(&req, None, &overlay_mgr).unwrap();
    assert_eq!(res["title"], "Do something");
}

#[test]
fn test_code_action_resolve_data_missing_uri_rejected() {
    let overlay_mgr = OverlayManager::new();
    let req = json!({
        "params": {
            "title": "Do something",
            "data": { "kind": "quickfix" }
        }
    });
    let err = handle_code_action_resolve(&req, None, &overlay_mgr).unwrap_err();
    assert_eq!(err.code, INVALID_PARAMS);
}

#[test]
fn test_code_action_resolve_unknown_kind_rejected() {
    let overlay_mgr = OverlayManager::new();
    let req = json!({
        "params": {
            "title": "Unknown Action",
            "data": {
                "uri": "file:///tmp/t.rs",
                "kind": "unknown.special.action"
            }
        }
    });
    let err = handle_code_action_resolve(&req, None, &overlay_mgr).unwrap_err();
    assert_eq!(err.code, INVALID_PARAMS);
}

#[test]
fn test_code_action_resolve_already_resolved_action_returned() {
    let overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/already_resolved.rs";
    let req = json!({
        "params": {
            "title": "Clean Imports",
            "kind": "source.organizeImports",
            "edit": {
                "changes": {
                    uri: [{
                        "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
                        "newText": "// comment\n"
                    }]
                }
            }
        }
    });
    let res = handle_code_action_resolve(&req, None, &overlay_mgr).unwrap();
    assert_eq!(res["title"], "Clean Imports");
    assert!(res["edit"]["changes"][uri].is_array());
}

#[test]
fn test_code_action_resolve_external_path_rejected() {
    let (_temp, ws, overlay_mgr, _conv) = setup_test_editing();
    let ext_uri = "file:///.cargo/registry/src/ext_service-0.5.0/src/lib.rs";
    let req = json!({
        "params": {
            "title": "Organize External",
            "kind": "source.organizeImports",
            "data": {
                "uri": ext_uri,
                "version": 1,
                "kind": "source.organizeImports"
            }
        }
    });
    let err = handle_code_action_resolve(&req, Some(&ws), &overlay_mgr).unwrap_err();
    assert_eq!(err.code, REQUEST_FAILED);
    assert!(err.message.contains("external dependency"));
}

#[test]
fn test_code_action_resolve_version_conflict_modified_returns_content_modified() {
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/conflict.rs";
    let code = "use b::B;\nuse a::A;\nfn main() {}\n";
    // Document is opened at version 1, but later updated to version 2
    overlay_mgr.open_document(uri, "/tmp/conflict.rs", 2, code.to_string());

    // Client requests resolve using stale action computed at version 1
    let req = json!({
        "params": {
            "title": "Organize Imports",
            "kind": "source.organizeImports",
            "data": {
                "uri": uri,
                "version": 1,
                "kind": "source.organizeImports"
            }
        }
    });

    let err = handle_code_action_resolve(&req, None, &overlay_mgr).unwrap_err();
    assert_eq!(
        err.code, CONTENT_MODIFIED,
        "Stale action resolution after document change must return ContentModified (-32801)"
    );
}

#[test]
fn test_code_action_resolve_deferred_organize_imports() {
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/resolve_org.rs";
    let code = "use z::Z;\nuse a::A;\nfn main() {}\n";
    overlay_mgr.open_document(uri, "/tmp/resolve_org.rs", 1, code.to_string());

    let req = json!({
        "params": {
            "title": "Organize Imports",
            "kind": "source.organizeImports",
            "data": {
                "uri": uri,
                "version": 1,
                "kind": "source.organizeImports"
            }
        }
    });

    let res = handle_code_action_resolve(&req, None, &overlay_mgr).unwrap();
    let changes = &res["edit"]["changes"][uri];
    assert!(changes.is_array());
    assert_eq!(changes[0]["newText"], "use a::A;\nuse z::Z;");
}

#[test]
fn test_code_action_resolve_deferred_extract_variable() {
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/resolve_extract.rs";
    let code = "fn main() {\n    let val = 10 + 20;\n}\n";
    overlay_mgr.open_document(uri, "/tmp/resolve_extract.rs", 1, code.to_string());

    let req = json!({
        "params": {
            "title": "Extract variable",
            "kind": "refactor.extract",
            "data": {
                "uri": uri,
                "version": 1,
                "kind": "refactor.extract",
                "range": {
                    "start": { "line": 1, "character": 14 },
                    "end": { "line": 1, "character": 21 }
                }
            }
        }
    });

    let res = handle_code_action_resolve(&req, None, &overlay_mgr).unwrap();
    let changes = &res["edit"]["changes"][uri];
    assert_eq!(changes.as_array().unwrap().len(), 2);
}

#[test]
fn test_code_action_resolve_deferred_auto_import() {
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/resolve_auto.rs";
    let code = "fn main() {}\n";
    overlay_mgr.open_document(uri, "/tmp/resolve_auto.rs", 1, code.to_string());

    let req = json!({
        "params": {
            "title": "Import `my_crate::Foo`",
            "kind": "quickfix",
            "data": {
                "uri": uri,
                "version": 1,
                "kind": "quickfix.auto_import",
                "import_path": "my_crate::Foo"
            }
        }
    });

    let res = handle_code_action_resolve(&req, None, &overlay_mgr).unwrap();
    let changes = &res["edit"]["changes"][uri];
    assert_eq!(changes[0]["newText"], "use my_crate::Foo;\n");
}

#[test]
fn test_code_action_resolve_result_with_apply_workspace_edit_with_versions() {
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/apply_ver.rs";
    let code = "use z::Z;\nuse a::A;\nfn main() {}\n";
    overlay_mgr.open_document(uri, "/tmp/apply_ver.rs", 3, code.to_string());

    let req = json!({
        "params": {
            "title": "Organize Imports",
            "kind": "source.organizeImports",
            "data": {
                "uri": uri,
                "version": 3,
                "kind": "source.organizeImports"
            }
        }
    });

    let res = handle_code_action_resolve(&req, None, &overlay_mgr).unwrap();
    let edit = &res["edit"];

    let mut files = HashMap::new();
    files.insert(uri.to_string(), code.to_string());

    let mut expected_versions = HashMap::new();
    expected_versions.insert(uri.to_string(), 3i64);

    let updated = apply_workspace_edit_with_versions(&files, edit, Some(&expected_versions));
    let expected_code = "use a::A;\nuse z::Z;\nfn main() {}\n";
    assert_eq!(updated.get(uri).unwrap(), expected_code);
}

#[test]
fn test_code_action_resolve_closed_file_disk_modified_returns_content_modified() {
    let temp = tempfile::tempdir().unwrap();
    let file_path = temp.path().join("closed_mod.rs");
    let initial_code = "use b::B;\nuse a::A;\nfn main() {}\n";
    std::fs::write(&file_path, initial_code).unwrap();
    let uri = format!("file://{}", file_path.display());
    let initial_hash = ridx_format::normalized_source_hash(initial_code.as_bytes());

    let overlay_mgr = OverlayManager::new();

    let req = json!({
        "params": {
            "title": "Organize Imports",
            "kind": "source.organizeImports",
            "data": {
                "uri": &uri,
                "kind": "source.organizeImports",
                "content_hash": initial_hash
            }
        }
    });

    // 1. Resolve succeeds while disk content hash matches
    let res = handle_code_action_resolve(&req, None, &overlay_mgr);
    assert!(
        res.is_ok(),
        "Resolve should succeed when disk content matches hash"
    );

    // 2. Modify disk content
    std::fs::write(
        &file_path,
        "use b::B;\nuse a::A;\n// modified\nfn main() {}\n",
    )
    .unwrap();

    // 3. Resolve must fail with ContentModified (-32801)
    let err = handle_code_action_resolve(&req, None, &overlay_mgr).unwrap_err();
    assert_eq!(
        err.code, CONTENT_MODIFIED,
        "Resolving action for modified closed file must return ContentModified error"
    );
}

#[test]
fn test_code_action_resolve_organize_imports_preserves_leading_comments() {
    let mut overlay_mgr = OverlayManager::new();
    let uri = "file:///tmp/leading_comments.rs";
    let code = "// Important comment\nuse b::B;\nuse a::A;\nfn main() {}\n";
    overlay_mgr.open_document(uri, "/tmp/leading_comments.rs", 1, code.to_string());

    let req = json!({
        "params": {
            "title": "Organize Imports",
            "kind": "source.organizeImports",
            "data": {
                "uri": uri,
                "version": 1,
                "kind": "source.organizeImports"
            }
        }
    });

    let res = handle_code_action_resolve(&req, None, &overlay_mgr).unwrap();
    let edit = &res["edit"];

    let mut files = HashMap::new();
    files.insert(uri.to_string(), code.to_string());

    let updated = apply_workspace_edit_with_versions(&files, edit, None);
    let expected_code = "// Important comment\nuse a::A;\nuse b::B;\nfn main() {}\n";
    assert_eq!(updated.get(uri).unwrap(), expected_code);
}
