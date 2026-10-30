/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Compiler-grounded L2 integration tests for Domain 10: Multi-Root & Dynamic Workspaces.
//! Tests dynamic folder addition/removal, cross-crate navigation, longest-prefix matching,
//! and multi-root workspace symbol discovery on real compiler fixtures.

use super::l2_harness::{fixture_dir, get_or_index_l2_fixture, pos_in_file};
use ridx::lsp::navigation::handle_definition;
use ridx::lsp::protocol::{path_to_uri, PositionConverter};
use ridx::lsp::symbols::handle_workspace_symbol;
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use serde_json::json;

#[test]
fn test_l2_multiroot_path_routing_longest_prefix() {
    let mut mr = MultiRootWorkspace::new();

    // Add roots
    let temp_a = std::env::temp_dir().join(format!("ridx_mr_a_{}", std::process::id()));
    let temp_b = temp_a.join("nested/crate");
    let _ = std::fs::create_dir_all(&temp_b);

    let ridx_a = temp_a.join("index.ridx");
    let ridx_b = temp_b.join("index.ridx");

    let mut w_a = ridx_format::IndexWriter::new("repo_crate", "0.1.0");
    let mut f_a = std::fs::File::create(&ridx_a).unwrap();
    w_a.write_to(&mut f_a).unwrap();

    let mut w_b = ridx_format::IndexWriter::new("nested_crate", "0.1.0");
    let mut f_b = std::fs::File::create(&ridx_b).unwrap();
    w_b.write_to(&mut f_b).unwrap();

    assert!(mr.add_root(ridx_a.clone()));
    assert!(mr.add_root(ridx_b.clone()));
    assert_eq!(mr.len(), 2);

    // 1. Path under nested crate matches root_b (longest prefix)
    let nested_file = temp_b.join("src/lib.rs");
    let matched_nested = mr.find_workspace_for_path(&nested_file).unwrap();
    assert_eq!(matched_nested.readers().first().unwrap().crate_name(), "nested_crate");

    // 2. Path under top-level matches root_a
    let top_file = temp_a.join("src/lib.rs");
    let matched_top = mr.find_workspace_for_path(&top_file).unwrap();
    assert_eq!(matched_top.readers().first().unwrap().crate_name(), "repo_crate");

    let _ = std::fs::remove_dir_all(&temp_a);
}

#[test]
fn test_l2_multiroot_cross_crate_navigation() {
    let ws = get_or_index_l2_fixture("multi_crate");
    let app_lib = fixture_dir("multi_crate").join("crates/app_crate/src/lib.rs");
    let app_uri = path_to_uri(app_lib.to_str().unwrap());
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // 1. Go to definition on `ExternalCoreService` in `let ext_svc = ExternalCoreService::new(42);`
    let (ext_line, ext_col) = pos_in_file(&app_lib, "let ext_svc = ExternalCoreService::new(42);", "ExternalCoreService");

    let ext_req = json!({
        "params": {
            "textDocument": { "uri": &app_uri },
            "position": { "line": ext_line, "character": ext_col }
        }
    });

    let ext_res = handle_definition(&ext_req, Some(ws), &overlay_mgr, &mut conv);
    assert!(!ext_res.is_null(), "Definition for ExternalCoreService must resolve");

    // 2. Go to definition on local `CoreService` in `let local_svc = CoreService { local_val: 10 };`
    let (local_line, local_col) = pos_in_file(&app_lib, "let local_svc = CoreService { local_val: 10 };", "CoreService");

    let local_req = json!({
        "params": {
            "textDocument": { "uri": &app_uri },
            "position": { "line": local_line, "character": local_col }
        }
    });

    let local_res = handle_definition(&local_req, Some(ws), &overlay_mgr, &mut conv);
    assert!(!local_res.is_null(), "Definition for local CoreService must resolve");
}

#[test]
fn test_l2_multiroot_dynamic_workspace_folders() {
    let mut mr = MultiRootWorkspace::new();
    let temp_dir = std::env::temp_dir().join(format!("ridx_dyn_mr_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);

    let ridx_1 = temp_dir.join("one.ridx");
    let mut w1 = ridx_format::IndexWriter::new("crate_one", "0.1.0");
    let mut f1 = std::fs::File::create(&ridx_1).unwrap();
    w1.write_to(&mut f1).unwrap();

    let ridx_2 = temp_dir.join("two.ridx");
    let mut w2 = ridx_format::IndexWriter::new("crate_two", "0.1.0");
    let mut f2 = std::fs::File::create(&ridx_2).unwrap();
    w2.write_to(&mut f2).unwrap();

    // 1. Dynamically add folder
    let add_event = json!({
        "params": {
            "event": {
                "added": [{ "uri": path_to_uri(ridx_1.to_str().unwrap()) }],
                "removed": []
            }
        }
    });
    mr.handle_did_change_workspace_folders(&add_event);
    assert_eq!(mr.len(), 1);
    assert_eq!(mr.first_workspace().unwrap().readers().first().unwrap().crate_name(), "crate_one");

    // 2. Add second folder and remove first
    let swap_event = json!({
        "params": {
            "event": {
                "added": [{ "uri": path_to_uri(ridx_2.to_str().unwrap()) }],
                "removed": [{ "uri": path_to_uri(ridx_1.to_str().unwrap()) }]
            }
        }
    });
    mr.handle_did_change_workspace_folders(&swap_event);
    assert_eq!(mr.len(), 1);
    assert_eq!(mr.first_workspace().unwrap().readers().first().unwrap().crate_name(), "crate_two");

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_l2_multiroot_symbols_search_across_crates() {
    let ws = get_or_index_l2_fixture("multi_crate");
    let mut conv = PositionConverter::new();

    // Search for `CoreService` which exists in both core_lib and app_crate
    let req = json!({
        "params": {
            "query": "CoreService"
        }
    });

    let res = handle_workspace_symbol(&req, std::iter::once(ws), &mut conv);
    let items = res.as_array().expect("workspace symbols array");
    assert!(!items.is_empty(), "Must find CoreService definitions across multi-crate workspace");

    let names: Vec<&str> = items.iter().filter_map(|it| it["name"].as_str()).collect();
    assert!(names.contains(&"CoreService"), "Must contain CoreService in search results");
}

#[test]
fn test_l2_multiroot_provenance_and_source_hash() {
    let _ws = get_or_index_l2_fixture("multi_crate");
    let core_lib = fixture_dir("multi_crate").join("crates/core_lib/src/lib.rs");
    let content = std::fs::read_to_string(&core_lib).unwrap();

    let mut mr = MultiRootWorkspace::new();
    let ridx_file = fixture_dir("multi_crate").join(".ridx/index.ridx");
    if ridx_file.exists() {
        mr.add_root(ridx_file);
        let valid = mr.check_file_provenance(core_lib.to_str().unwrap(), &content);
        assert!(valid, "Unmodified file must pass provenance check");
    }

    assert!(!mr.reload_all_if_modified(), "Unmodified index needs no reload");
}
