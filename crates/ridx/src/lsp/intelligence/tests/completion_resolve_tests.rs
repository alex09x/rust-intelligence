/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use crate::lsp::error::INVALID_PARAMS;
use crate::lsp::intelligence::handle_completion_resolve;
use serde_json::json;

#[test]
fn test_completion_resolve_missing_params() {
    let req = json!({});
    let err = handle_completion_resolve(&req).unwrap_err();
    assert_eq!(err.code, INVALID_PARAMS);
}

#[test]
fn test_completion_resolve_null_params() {
    let req = json!({
        "params": null
    });
    let err = handle_completion_resolve(&req).unwrap_err();
    assert_eq!(err.code, INVALID_PARAMS);
}

#[test]
fn test_completion_resolve_missing_label() {
    let req = json!({
        "params": {
            "kind": 3,
            "detail": "fn test()"
        }
    });
    let err = handle_completion_resolve(&req).unwrap_err();
    assert_eq!(err.code, INVALID_PARAMS);
}

#[test]
fn test_completion_resolve_basic_enrichment() {
    let req = json!({
        "params": {
            "label": "calculate",
            "kind": 3,
            "detail": "fn calculate(a: i32, b: i32) -> i32"
        }
    });
    let res = handle_completion_resolve(&req).unwrap();
    assert_eq!(res["label"], "calculate");
    assert_eq!(res["kind"], 3);
    assert_eq!(res["detail"], "fn calculate(a: i32, b: i32) -> i32");
    assert!(!res["documentation"].is_null());
    let doc = res["documentation"]["value"].as_str().unwrap();
    assert!(doc.contains("calculate"));
}

#[test]
fn test_completion_resolve_preserves_existing_documentation() {
    let req = json!({
        "params": {
            "label": "User",
            "kind": 22,
            "documentation": {
                "kind": "markdown",
                "value": "Existing documentation for User struct"
            }
        }
    });
    let res = handle_completion_resolve(&req).unwrap();
    assert_eq!(res["documentation"]["value"].as_str().unwrap(), "Existing documentation for User struct");
}

#[test]
fn test_completion_resolve_preserves_custom_data_and_insert_text() {
    let req = json!({
        "params": {
            "label": "my_function",
            "insertText": "my_function($1)",
            "insertTextFormat": 2,
            "data": {
                "crate": "my_crate",
                "custom_id": 999
            }
        }
    });
    let res = handle_completion_resolve(&req).unwrap();
    assert_eq!(res["insertText"], "my_function($1)");
    assert_eq!(res["insertTextFormat"], 2);
    assert_eq!(res["data"]["custom_id"], 999);
}

#[test]
fn test_completion_resolve_enrich_from_detail() {
    let req = json!({
        "params": {
            "label": "do_work",
            "detail": "pub fn do_work<T: Send>(val: T)"
        }
    });
    let res = handle_completion_resolve(&req).unwrap();
    let doc = res["documentation"]["value"].as_str().unwrap();
    assert!(doc.contains("pub fn do_work<T: Send>(val: T)"));
}

#[test]
fn test_completion_resolve_enrich_from_data() {
    let req = json!({
        "params": {
            "label": "helper",
            "data": "Extra info about helper function"
        }
    });
    let res = handle_completion_resolve(&req).unwrap();
    let doc = res["documentation"]["value"].as_str().unwrap();
    assert!(doc.contains("Extra info about helper function"));
}

#[test]
fn test_completion_resolve_kind_preservation() {
    for kind in [1, 2, 3, 5, 6, 7, 11, 13, 21, 22] {
        let req = json!({
            "params": {
                "label": "symbol_item",
                "kind": kind
            }
        });
        let res = handle_completion_resolve(&req).unwrap();
        assert_eq!(res["kind"], kind);
    }
}
