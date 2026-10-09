/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::*;
use crate::lsp::symbols::handle_workspace_symbol_resolve;
use serde_json::{json, Value};

#[test]
fn test_workspace_symbol_resolve_item_without_location() {
    let mut h = setup_test_symbols();
    let req = json!({
        "params": {
            "name": "my_symbol",
            "kind": 12
        }
    });

    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&h.ws), &mut h.conv);
    assert_eq!(res["name"], "my_symbol");
    assert!(res.get("location").is_some());
    let loc = &res["location"];
    assert!(loc["uri"].as_str().unwrap().starts_with("file://"));
    assert_eq!(loc["range"]["start"]["line"], 0); // 1-based line 1 -> 0-based 0
}

#[test]
fn test_workspace_symbol_resolve_item_with_existing_location() {
    let mut h = setup_test_symbols();
    let existing_loc = json!({
        "uri": "file:///custom/path.rs",
        "range": {
            "start": { "line": 42, "character": 5 },
            "end": { "line": 42, "character": 15 }
        }
    });
    let req = json!({
        "params": {
            "name": "my_symbol",
            "kind": 12,
            "location": existing_loc.clone()
        }
    });

    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&h.ws), &mut h.conv);
    assert_eq!(res["location"], existing_loc);
}

#[test]
fn test_workspace_symbol_resolve_nonexistent_symbol() {
    let mut h = setup_test_symbols();
    let req = json!({
        "params": {
            "name": "UnknownSymbolNotInIndex",
            "kind": 12
        }
    });

    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&h.ws), &mut h.conv);
    assert_eq!(res["name"], "UnknownSymbolNotInIndex");
    // Since it's not found in index, location remains unset
    assert!(res.get("location").is_none());
}

#[test]
fn test_workspace_symbol_resolve_null_params() {
    let mut h = setup_test_symbols();
    let req = json!({});
    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&h.ws), &mut h.conv);
    assert_eq!(res, Value::Null);

    let null_req = json!({ "params": null });
    let res2 = handle_workspace_symbol_resolve(&null_req, std::iter::once(&h.ws), &mut h.conv);
    assert_eq!(res2, Value::Null);
}

#[test]
fn test_workspace_symbol_resolve_struct_symbol() {
    let mut h = setup_test_symbols();
    let req = json!({
        "params": {
            "name": "MyStruct",
            "kind": 23
        }
    });

    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&h.ws), &mut h.conv);
    assert!(res.get("location").is_some());
    assert_eq!(res["location"]["range"]["start"]["line"], 1); // line 2 in source -> line 1
}

#[test]
fn test_workspace_symbol_resolve_nested_item_container() {
    let mut h = setup_test_symbols();
    let req = json!({
        "params": {
            "name": "sym_15",
            "kind": 12
        }
    });

    let res = handle_workspace_symbol_resolve(&req, std::iter::once(&h.ws), &mut h.conv);
    assert!(res.get("location").is_some());
    assert_eq!(res["containerName"], "batch");
}

#[test]
fn test_workspace_symbol_resolve_multiple_workspaces() {
    let mut h1 = setup_test_symbols();
    let h2 = setup_test_symbols();

    let req = json!({
        "params": {
            "name": "MY_CONST",
            "kind": 14
        }
    });

    let workspaces = vec![&h1.ws, &h2.ws];
    let res = handle_workspace_symbol_resolve(&req, workspaces.into_iter(), &mut h1.conv);
    assert!(res.get("location").is_some());
    assert_eq!(res["location"]["range"]["start"]["line"], 4); // line 5 in source -> line 4
}
