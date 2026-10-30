/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Compiler-grounded L2 integration tests for qualifier shadowing priority
//! (local `mod` over sysroot, and prelude types without `use`).

use super::l2_harness::{fixture_dir, get_or_index_l2_fixture, pos_in_file};
use ridx::lsp::intelligence::completion::handle_completion;
use ridx::lsp::protocol::{path_to_uri, PositionConverter};
use ridx::overlay::OverlayManager;
use serde_json::json;

#[test]
fn test_l2_completion_local_mod_shadows_sysroot() {
    let ws = get_or_index_l2_fixture("shorthand_and_reexports");
    let lib_file = fixture_dir("shorthand_and_reexports").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // Query completion after `range::` inside `invoke_local_range`
    let (line, col) = pos_in_file(&lib_file, "let _ = range::RangeVisitor;", "range::");
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": line, "character": col + 7 } // right after `range::`
        }
    });

    let res = handle_completion(&req, Some(ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!res.is_null(), "Completion on local range:: must return items");
    let items = res["items"].as_array().expect("items must be array");

    let labels: Vec<&str> = items.iter().filter_map(|it| it["label"].as_str()).collect();

    assert!(labels.contains(&"RangeVisitor"), "Must contain local RangeVisitor from mod range, got: {labels:?}");
    assert!(labels.contains(&"FIELDS"), "Must contain local FIELDS from mod range, got: {labels:?}");
    assert!(!labels.contains(&"RangeInclusive"), "Must NOT contain sysroot RangeInclusive (local mod range must shadow sysroot), got: {labels:?}");
    assert!(!labels.contains(&"legacy"), "Must NOT contain sysroot legacy, got: {labels:?}");
}

#[test]
fn test_l2_completion_sysroot_unshadowed_when_no_local_mod() {
    let ws = get_or_index_l2_fixture("shorthand_and_reexports");
    let lib_file = fixture_dir("shorthand_and_reexports").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // Query completion after `ops::` inside `remote_ops` (where `use core::ops;` is present)
    let (line, col) = pos_in_file(&lib_file, "let _ = ops::Range { start: 0, end: 1 };", "ops::");
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": line, "character": col + 5 }
        }
    });

    let res = handle_completion(&req, Some(ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!res.is_null());
    let items = res["items"].as_array().expect("items must be array");
    let labels: Vec<&str> = items.iter().filter_map(|it| it["label"].as_str()).collect();

    assert!(!labels.contains(&"RangeVisitor"), "Must NOT contain RangeVisitor from unrelated local module, got: {labels:?}");
    assert!(labels.contains(&"Range"), "Must contain Range from sysroot core::ops, got: {labels:?}");
}

#[test]
fn test_l2_completion_prelude_types_without_use() {
    let ws = get_or_index_l2_fixture("shorthand_and_reexports");
    let lib_file = fixture_dir("shorthand_and_reexports").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // 1. Query completion after `Vec::` in `invoke_prelude_types` (no `use` declaration in file)
    let (line, col) = pos_in_file(&lib_file, "let _: Vec<i32> = Vec::new();", "Vec::");
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": line, "character": col + 5 }
        }
    });

    let res = handle_completion(&req, Some(ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!res.is_null(), "Completion on Vec:: without use must return items");
    let items = res["items"].as_array().expect("items must be array");
    let labels: Vec<&str> = items.iter().filter_map(|it| it["label"].as_str()).collect();

    assert!(labels.contains(&"new"), "Vec:: completion must contain `new`, got: {labels:?}");
    assert!(labels.contains(&"with_capacity"), "Vec:: completion must contain `with_capacity`, got: {labels:?}");

    // 2. Query completion after `String::` in `invoke_prelude_types`
    let (line2, col2) = pos_in_file(&lib_file, "let _ = String::from(\"hello\");", "String::");
    let req2 = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": line2, "character": col2 + 8 }
        }
    });

    let res2 = handle_completion(&req2, Some(ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!res2.is_null(), "Completion on String:: without use must return items");
    let items2 = res2["items"].as_array().expect("items must be array");
    let labels2: Vec<&str> = items2.iter().filter_map(|it| it["label"].as_str()).collect();

    assert!(labels2.contains(&"new"), "String:: completion must contain `new`, got: {labels2:?}");
    assert!(labels2.contains(&"from_utf8"), "String:: completion must contain `from_utf8`, got: {labels2:?}");
}
