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
use crate::lsp::symbols::handle_workspace_symbol;
use serde_json::json;

#[test]
fn test_workspace_symbols_exact_query() {
    let mut h = setup_test_symbols();
    let req = json!({
        "params": {
            "query": "my_symbol"
        }
    });

    let res = handle_workspace_symbol(&req, std::iter::once(&h.ws), &mut h.conv);
    let items = res.as_array().expect("expected array");
    assert!(!items.is_empty());
    assert_eq!(items[0]["name"], "my_symbol");
    assert_eq!(items[0]["kind"], 12); // SymbolKind::Function = 12
}

#[test]
fn test_workspace_symbols_prefix_query() {
    let mut h = setup_test_symbols();
    let req = json!({
        "params": {
            "query": "My"
        }
    });

    let res = handle_workspace_symbol(&req, std::iter::once(&h.ws), &mut h.conv);
    let items = res.as_array().expect("expected array");
    let names: Vec<&str> = items.iter().filter_map(|s| s["name"].as_str()).collect();
    assert!(names.contains(&"MyStruct"));
    assert!(names.contains(&"MyEnum"));
    assert!(names.contains(&"MyTrait"));
}

#[test]
fn test_workspace_symbols_empty_query() {
    let mut h = setup_test_symbols();
    let req = json!({
        "params": {
            "query": ""
        }
    });

    let res = handle_workspace_symbol(&req, std::iter::once(&h.ws), &mut h.conv);
    let items = res.as_array().expect("expected array");
    assert!(!items.is_empty());
    assert!(items.len() <= 100);
}

#[test]
fn test_workspace_symbols_capping_at_100() {
    let mut h = setup_test_symbols();
    let req = json!({
        "params": {
            "query": "sym"
        }
    });

    let res = handle_workspace_symbol(&req, std::iter::once(&h.ws), &mut h.conv);
    let items = res.as_array().expect("expected array");
    assert_eq!(items.len(), 100);
}

#[test]
fn test_workspace_symbols_container_name() {
    let mut h = setup_test_symbols();
    let req = json!({
        "params": {
            "query": "sym_10"
        }
    });

    let res = handle_workspace_symbol(&req, std::iter::once(&h.ws), &mut h.conv);
    let items = res.as_array().expect("expected array");
    assert!(!items.is_empty());
    let item = &items[0];
    assert_eq!(item["name"], "sym_10");
    assert_eq!(item["containerName"], "batch");
}

#[test]
fn test_workspace_symbols_symbol_kind_mapping() {
    let mut h = setup_test_symbols();
    let req = json!({
        "params": {
            "query": "MY_CONST"
        }
    });

    let res = handle_workspace_symbol(&req, std::iter::once(&h.ws), &mut h.conv);
    let items = res.as_array().expect("expected array");
    assert!(!items.is_empty());
    assert_eq!(items[0]["name"], "MY_CONST");
    assert_eq!(items[0]["kind"], 14); // SymbolKind::Constant = 14
}

#[test]
fn test_workspace_symbols_location_uri_format() {
    let mut h = setup_test_symbols();
    let req = json!({
        "params": {
            "query": "MyStruct"
        }
    });

    let res = handle_workspace_symbol(&req, std::iter::once(&h.ws), &mut h.conv);
    let items = res.as_array().expect("expected array");
    assert!(!items.is_empty());
    let uri = items[0]["location"]["uri"].as_str().unwrap();
    assert!(uri.starts_with("file://"));
    assert!(uri.ends_with("src/lib.rs"));
}

#[test]
fn test_workspace_symbols_multiple_workspaces() {
    let mut h1 = setup_test_symbols();
    let h2 = setup_test_symbols();

    let req = json!({
        "params": {
            "query": "my_symbol"
        }
    });

    let workspaces = vec![&h1.ws, &h2.ws];
    let res = handle_workspace_symbol(&req, workspaces.into_iter(), &mut h1.conv);
    let items = res.as_array().expect("expected array");
    // Both workspaces have my_symbol; since path/file/start_line match, deduplication works or keeps entries
    assert!(!items.is_empty());
}

#[test]
fn test_workspace_symbols_no_matches() {
    let mut h = setup_test_symbols();
    let req = json!({
        "params": {
            "query": "NonExistentSymbolNameZzz123"
        }
    });

    let res = handle_workspace_symbol(&req, std::iter::once(&h.ws), &mut h.conv);
    let items = res.as_array().expect("expected array");
    assert!(items.is_empty());
}

#[test]
fn test_workspace_symbols_missing_params() {
    let mut h = setup_test_symbols();
    let empty_req = json!({});
    let res = handle_workspace_symbol(&empty_req, std::iter::once(&h.ws), &mut h.conv);
    let items = res.as_array().expect("expected array");
    // Empty query defaults to returning symbols up to 100
    assert!(!items.is_empty());
}
