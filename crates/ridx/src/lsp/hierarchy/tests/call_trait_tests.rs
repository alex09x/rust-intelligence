/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use crate::lsp::hierarchy::call::handle_call_hierarchy_incoming_calls;
use crate::lsp::protocol::path_to_uri;
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter, RawDefinition, RawReference, WorkspaceIndex};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

fn setup_test_index(temp_dir: &Path) -> (WorkspaceIndex, PathBuf) {
    let _ = fs::create_dir_all(temp_dir.join("src"));

    let src_file = temp_dir.join("src/lib.rs");
    let src_code = concat!(
        "pub trait Default {\n",
        "    fn default() -> Self;\n",
        "}\n",
        "pub struct TypeA;\n",
        "impl Default for TypeA {\n",
        "    fn default() -> Self { TypeA }\n",
        "}\n",
        "pub struct TypeB;\n",
        "impl Default for TypeB {\n",
        "    fn default() -> Self { TypeB }\n",
        "}\n",
    );
    let _ = fs::write(&src_file, src_code);
    let src_str = src_file.to_str().unwrap().to_string();

    let callers_file = temp_dir.join("src/callers.rs");
    let callers_code = concat!(
        "pub fn call_a() {\n",
        "    TypeA::default();\n",
        "}\n",
        "pub fn call_b() {\n",
        "    TypeB::default();\n",
        "}\n",
        "pub fn call_trait() {\n",
        "    Default::default();\n",
        "}\n",
    );
    let _ = fs::write(&callers_file, callers_code);
    let callers_str = callers_file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("test_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Trait,
        visibility: 0,
        is_macro_expansion: false,
        file: src_str.clone(),
        name: "Default".to_string(),
        start_line: 1,
        start_col: 11,
        end_line: 1,
        end_col: 18,
        path: "test_crate::Default".to_string(),
        signature: None,
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        file: src_str.clone(),
        name: "default".to_string(),
        start_line: 2,
        start_col: 8,
        end_line: 2,
        end_col: 15,
        path: "test_crate::Default::default".to_string(),
        signature: Some("fn() -> Self".to_string()),
    });
    writer.add_definition(RawDefinition {
        def_id: 3,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        file: src_str.clone(),
        name: "TypeA".to_string(),
        start_line: 4,
        start_col: 12,
        end_line: 4,
        end_col: 17,
        path: "test_crate::TypeA".to_string(),
        signature: None,
    });
    writer.add_definition(RawDefinition {
        def_id: 4,
        kind: DefKind::AssocFn,
        visibility: 0,
        is_macro_expansion: false,
        file: src_str.clone(),
        name: "default".to_string(),
        start_line: 6,
        start_col: 8,
        end_line: 6,
        end_col: 15,
        path: "<test_crate::TypeA as test_crate::Default>::default".to_string(),
        signature: Some("fn() -> Self".to_string()),
    });
    writer.add_definition(RawDefinition {
        def_id: 5,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        file: src_str.clone(),
        name: "TypeB".to_string(),
        start_line: 8,
        start_col: 12,
        end_line: 8,
        end_col: 17,
        path: "test_crate::TypeB".to_string(),
        signature: None,
    });
    writer.add_definition(RawDefinition {
        def_id: 6,
        kind: DefKind::AssocFn,
        visibility: 0,
        is_macro_expansion: false,
        file: src_str.clone(),
        name: "default".to_string(),
        start_line: 10,
        start_col: 8,
        end_line: 10,
        end_col: 15,
        path: "<test_crate::TypeB as test_crate::Default>::default".to_string(),
        signature: Some("fn() -> Self".to_string()),
    });

    writer.add_definition(RawDefinition {
        def_id: 7,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        file: callers_str.clone(),
        name: "call_a".to_string(),
        start_line: 1,
        start_col: 8,
        end_line: 3,
        end_col: 2,
        path: "test_crate::call_a".to_string(),
        signature: Some("fn()".to_string()),
    });
    writer.add_definition(RawDefinition {
        def_id: 8,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        file: callers_str.clone(),
        name: "call_b".to_string(),
        start_line: 4,
        start_col: 8,
        end_line: 6,
        end_col: 2,
        path: "test_crate::call_b".to_string(),
        signature: Some("fn()".to_string()),
    });
    writer.add_definition(RawDefinition {
        def_id: 9,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        file: callers_str.clone(),
        name: "call_trait".to_string(),
        start_line: 7,
        start_col: 8,
        end_line: 9,
        end_col: 2,
        path: "test_crate::call_trait".to_string(),
        signature: Some("fn()".to_string()),
    });

    writer.add_reference(RawReference {
        file: callers_str.clone(),
        start_line: 2,
        start_col: 11,
        end_line: 2,
        end_col: 18,
        target_def_id: Some(4),
        target_path: "<test_crate::TypeA as test_crate::Default>::default".to_string(),
        target_unit: None,
        target_file: Some(src_str.clone()),
        target_line: 6,
        target_col: 8,
        kind: DefKind::AssocFn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.add_reference(RawReference {
        file: callers_str.clone(),
        start_line: 5,
        start_col: 11,
        end_line: 5,
        end_col: 18,
        target_def_id: Some(6),
        target_path: "<test_crate::TypeB as test_crate::Default>::default".to_string(),
        target_unit: None,
        target_file: Some(src_str.clone()),
        target_line: 10,
        target_col: 8,
        kind: DefKind::AssocFn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.add_reference(RawReference {
        file: callers_str,
        start_line: 8,
        start_col: 13,
        end_line: 8,
        end_col: 20,
        target_def_id: Some(2),
        target_path: "test_crate::Default::default".to_string(),
        target_unit: None,
        target_file: Some(src_str),
        target_line: 2,
        target_col: 8,
        kind: DefKind::AssocFn,
        is_write: false,
        is_macro_expansion: false,
    });

    let ridx_file = temp_dir.join("index.ridx");
    let mut f = fs::File::create(&ridx_file).unwrap();
    writer.write_to(&mut f).unwrap();

    let ws = WorkspaceIndex::open(&ridx_file).unwrap();
    (ws, src_file)
}

#[test]
fn test_incoming_calls_trait_impl_no_cross_pollution() {
    static TRAIT_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = TRAIT_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temp_dir =
        std::env::temp_dir().join(format!("ridx_test_call_trait_{}_{}", std::process::id(), id));

    let (ws, src_file) = setup_test_index(&temp_dir);
    let overlay_mgr = OverlayManager::new();
    let mut conv = crate::lsp::protocol::PositionConverter::new();

    let uri = path_to_uri(src_file.to_str().unwrap());
    let incoming_req = json!({
        "params": {
            "item": {
                "name": "default",
                "uri": uri,
                "range": {
                    "start": { "line": 5, "character": 7 },
                    "end": { "line": 5, "character": 14 }
                }
            }
        }
    });

    let incoming =
        handle_call_hierarchy_incoming_calls(&incoming_req, Some(&ws), &overlay_mgr, &mut conv);
    let arr = incoming.as_array().expect("incoming calls array");
    let caller_names: Vec<&str> = arr
        .iter()
        .filter_map(|item| item["from"]["name"].as_str())
        .collect();

    assert_eq!(
        caller_names,
        vec!["call_a"],
        "expected only direct call_a to <TypeA as Default>::default, got {caller_names:?}"
    );

    let _ = fs::remove_dir_all(&temp_dir);
}

/// Known limitation: calls via generic `Trait::method()` (such as `Default::default()`)
/// where type inference would be needed to determine that the call site resolves to
/// `<TypeA as Default>::default` are not returned when querying `incomingCalls` for a
/// specific trait implementation.
///
/// Because RIDX operates as a zero-memory indexing engine without full compiler type inference,
/// resolving generic trait invocations to concrete implementations would cause cross-type
/// pollution (e.g. returning all 1,300+ workspace callers of `Default::default()` for any
/// struct's `Default::default`). Therefore, only direct references to the concrete impl
/// or qualified calls (`TypeA::default()`, `<TypeA as Default>::default()`) are returned.
#[test]
fn test_incoming_calls_trait_generic_calls_not_returned() {
    static TRAIT_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = TRAIT_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temp_dir =
        std::env::temp_dir().join(format!("ridx_test_call_trait_lim_{}_{}", std::process::id(), id));

    let (ws, src_file) = setup_test_index(&temp_dir);
    let overlay_mgr = OverlayManager::new();
    let mut conv = crate::lsp::protocol::PositionConverter::new();
    let uri = path_to_uri(src_file.to_str().unwrap());

    // 1. Querying incoming calls for TypeA's impl: does NOT return call_trait or call_b
    let req_impl = json!({
        "params": {
            "item": {
                "name": "default",
                "uri": uri,
                "range": {
                    "start": { "line": 5, "character": 7 },
                    "end": { "line": 5, "character": 14 }
                }
            }
        }
    });
    let incoming_impl =
        handle_call_hierarchy_incoming_calls(&req_impl, Some(&ws), &overlay_mgr, &mut conv);
    let arr_impl = incoming_impl.as_array().expect("incoming calls array");
    let callers_impl: Vec<&str> = arr_impl
        .iter()
        .filter_map(|item| item["from"]["name"].as_str())
        .collect();

    assert_eq!(callers_impl, vec!["call_a"]);
    assert!(!callers_impl.contains(&"call_trait"));
    assert!(!callers_impl.contains(&"call_b"));

    // 2. Querying incoming calls for the trait method itself (Default::default at line 2, 0-indexed line 1):
    let req_trait = json!({
        "params": {
            "item": {
                "name": "default",
                "uri": uri,
                "range": {
                    "start": { "line": 1, "character": 7 },
                    "end": { "line": 1, "character": 14 }
                }
            }
        }
    });
    let incoming_trait =
        handle_call_hierarchy_incoming_calls(&req_trait, Some(&ws), &overlay_mgr, &mut conv);
    let arr_trait = incoming_trait.as_array().expect("incoming calls array");
    let callers_trait: Vec<&str> = arr_trait
        .iter()
        .filter_map(|item| item["from"]["name"].as_str())
        .collect();

    assert_eq!(callers_trait, vec!["call_trait"]);

    let _ = fs::remove_dir_all(&temp_dir);
}
