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
    assert!(items.len() <= 128);
}

#[test]
fn test_workspace_symbols_capping_at_128() {
    let mut h = setup_test_symbols();
    // `sym_` matches no type (the crate `test_syms` has no `_` after `sym`), so every kind is searched: the 130
    // `sym_*` functions match and rust-analyzer's default limit cuts them at 128.
    let req = json!({
        "params": {
            "query": "sym_"
        }
    });

    let res = handle_workspace_symbol(&req, std::iter::once(&h.ws), &mut h.conv);
    let items = res.as_array().expect("expected array");
    assert_eq!(items.len(), 128);
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
    // Empty query defaults to returning symbols up to 128
    assert!(!items.is_empty());
}

#[test]
fn test_workspace_symbols_generic_fn_excludes_typaram() {
    let mut h = setup_test_symbols();
    let temp_dir = std::env::temp_dir().join(format!("ridx_test_gen_syms_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    let _ = std::fs::create_dir_all(&temp_dir);
    let ridx_file = temp_dir.join("index.ridx");

    let mut writer = IndexWriter::new("test_gen_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        file: "src/lib.rs".to_string(),
        name: "deserialize".to_string(),
        start_line: 1,
        start_col: 8,
        end_line: 1,
        end_col: 19,
        path: "test_gen_crate::deserialize".to_string(),
        signature: Some("fn deserialize<X, Y>()".to_string()),
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::TyParam,
        visibility: 0,
        is_macro_expansion: false,
        file: "src/lib.rs".to_string(),
        name: "X".to_string(),
        start_line: 1,
        start_col: 23,
        end_line: 1,
        end_col: 24,
        path: "test_gen_crate::deserialize::X".to_string(),
        signature: None,
    });
    writer.add_definition(RawDefinition {
        def_id: 3,
        kind: DefKind::TyParam,
        visibility: 0,
        is_macro_expansion: false,
        file: "src/lib.rs".to_string(),
        name: "Y".to_string(),
        start_line: 1,
        start_col: 20,
        end_line: 1,
        end_col: 21,
        path: "test_gen_crate::deserialize::Y".to_string(),
        signature: None,
    });

    let mut f = std::fs::File::create(&ridx_file).unwrap();
    writer.write_to(&mut f).unwrap();
    drop(f);

    let ws = WorkspaceIndex::open(&ridx_file).unwrap();

    // Query "deserialize" should only return the function, not the type parameters X or Y
    // even though their canonical paths contain "deserialize"
    let req = json!({
        "params": {
            "query": "deserialize"
        }
    });
    let res = handle_workspace_symbol(&req, std::iter::once(&ws), &mut h.conv);
    let items = res.as_array().expect("expected array");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "deserialize");
    assert_eq!(items[0]["kind"], 12); // SymbolKind::Function = 12

    // Query "X" should return empty because X is TyParam and function name doesn't contain x
    let req_x = json!({
        "params": {
            "query": "X"
        }
    });
    let res_x = handle_workspace_symbol(&req_x, std::iter::once(&ws), &mut h.conv);
    let items_x = res_x.as_array().expect("expected array");
    assert!(items_x.is_empty(), "TyParam X must not be returned by workspace/symbol");

    let _ = std::fs::remove_dir_all(&temp_dir);
}
