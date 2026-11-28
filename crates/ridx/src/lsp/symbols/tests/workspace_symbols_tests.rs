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

/// prod-code, m=20226: `Config` of crate `smoke` had no container; rust-analyzer gives the crate, and prod-code prints
/// `smoke::Config` from it.
#[test]
fn test_a_crate_root_item_is_held_by_its_crate() {
    use crate::lsp::symbols::container_name;
    assert_eq!(container_name("Config", "smoke", false), "smoke");
    assert_eq!(container_name("Config::new", "smoke", false), "Config");
    assert_eq!(container_name("a::b::Item", "smoke", false), "b");
    assert_eq!(container_name("smoke", "smoke", true), "", "a crate root has none");
}

/// Review of 465e02ad: resolve took a top-level module stored by its bare path for a crate root, with no container.
#[test]
fn test_resolve_gives_a_top_level_module_its_crate() {
    let dir = tempfile::tempdir().unwrap();
    let mut w = ridx_format::IndexWriter::new("app", "0.1.0");
    for (id, name, kind) in [(1, "app", ridx_format::DefKind::Mod), (2, "net", ridx_format::DefKind::Mod)] {
        w.add_definition(ridx_format::RawDefinition {
            def_id: id,
            kind,
            visibility: 0,
            is_macro_expansion: false,
            file: "/w/src/lib.rs".to_string(),
            name: name.to_string(),
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 4,
            path: name.to_string(),
            signature: None,
        });
    }
    let shard = dir.path().join("app.ridx");
    w.write_to(&mut std::fs::File::create(&shard).unwrap()).unwrap();
    let index = ridx_format::WorkspaceIndex::open(&shard).unwrap();
    let mut conv = PositionConverter::new();
    let net = crate::lsp::symbols::handle_workspace_symbol_resolve(&json!({ "params": { "name": "net" } }), std::iter::once(&index), &mut conv);
    assert_eq!(net["containerName"], "app");
    let root = crate::lsp::symbols::handle_workspace_symbol_resolve(&json!({ "params": { "name": "app" } }), std::iter::once(&index), &mut conv);
    assert_eq!(root["containerName"], "", "the crate root itself has none");
}

/// prod-code, m=23269: a method of a generic impl is recorded as `mpsc::Receiver::<T>::recv_many`, and its container
/// came back as `<T>`; it is the type, as rust-analyzer gives it, and for a trait impl the implementing type.
#[test]
fn test_the_container_of_a_method_in_a_generic_impl_is_its_type() {
    use crate::lsp::symbols::container_name;
    assert_eq!(container_name("mpsc::Receiver::<T>::recv_many", "rapidfire", false), "Receiver");
    assert_eq!(container_name("queue::Queue::<T, N>::claim_bounded", "rapidfire", false), "Queue");
    assert_eq!(container_name("a::Map<K, V>::get", "app", false), "Map");
    assert_eq!(container_name("<a::Foo<T> as core::fmt::Debug>::fmt", "app", false), "Foo");
    assert_eq!(container_name("m::Handler::<fn() -> a::B>::call", "app", false), "Handler", "the `>` of `->` closes nothing");
    assert_eq!(container_name("batch::sym_10", "app", false), "batch");
    assert_eq!(container_name("Config", "smoke", false), "smoke", "an item of a crate root: its crate");
    assert_eq!(container_name("smoke", "smoke", true), "", "the crate root itself");
}
