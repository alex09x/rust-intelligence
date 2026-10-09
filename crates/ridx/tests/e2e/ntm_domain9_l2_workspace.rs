/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Compiler-grounded L2 integration tests for Domain 9: Workspace & File Operations.
//! Tests workspace/symbol, workspaceSymbol/resolve, workspace/diagnostic,
//! workspace/executeCommand, and workspace file event notifications.

use super::l2_harness::{fixture_dir, get_or_index_l2_fixture};
use ridx::lsp::editing::commands::handle_execute_command;
use ridx::lsp::presentation::workspace_diag::handle_workspace_diagnostic;
use ridx::lsp::protocol::{path_to_uri, PositionConverter};
use ridx::lsp::symbols::{handle_workspace_symbol, handle_workspace_symbol_resolve};
use ridx::lsp::workspace::MultiRootWorkspace;
use ridx::overlay::OverlayManager;
use serde_json::json;

#[test]
fn test_l2_workspace_symbol_search_and_deduplication() {
    let ws = get_or_index_l2_fixture("traits_and_generics");
    let mut conv = PositionConverter::new();

    // 1. Search for trait `Greeter`
    let req = json!({
        "params": {
            "query": "Greeter"
        }
    });

    let res = handle_workspace_symbol(&req, std::iter::once(ws), &mut conv);
    let items = res.as_array().expect("workspace symbols array");
    assert!(
        !items.is_empty(),
        "Workspace symbol query for 'Greeter' must find definition"
    );

    let greeter = items
        .iter()
        .find(|item| item["name"].as_str() == Some("Greeter"))
        .expect("Greeter symbol");
    assert_eq!(greeter["name"], "Greeter");
    assert!(greeter["location"]["uri"].as_str().is_some());
    assert!(greeter["location"]["range"]["start"]["line"].as_u64().is_some());

    // 2. Multi-root deduplication: pass the same workspace twice
    let double_res = handle_workspace_symbol(&req, [ws, ws].into_iter(), &mut conv);
    let double_items = double_res.as_array().unwrap();
    assert_eq!(
        items.len(),
        double_items.len(),
        "Workspace symbol search must deduplicate across roots"
    );
}

#[test]
fn test_l2_workspace_symbol_resolve() {
    let ws = get_or_index_l2_fixture("traits_and_generics");
    let mut conv = PositionConverter::new();

    // Incomplete symbol item without location
    let unresolved_req = json!({
        "params": {
            "name": "Greeter",
            "kind": 11
        }
    });

    let resolved =
        handle_workspace_symbol_resolve(&unresolved_req, std::iter::once(ws), &mut conv);
    assert!(!resolved.is_null(), "Resolved symbol must not be null");
    assert!(
        resolved["location"].is_object(),
        "Resolve must populate location for Greeter"
    );
    assert!(
        resolved["location"]["uri"].as_str().unwrap().contains("lib.rs"),
        "Resolved location URI must point to lib.rs"
    );
}

#[test]
fn test_l2_workspace_diagnostic_report() {
    let lib_file = fixture_dir("traits_and_generics").join("src/lib.rs");
    let uri = path_to_uri(lib_file.to_str().unwrap());
    let mut overlay_mgr = OverlayManager::new();
    let mut multi_root = MultiRootWorkspace::new();

    let ridx_file = fixture_dir("traits_and_generics").join(".ridx/index.ridx");
    if ridx_file.exists() {
        multi_root.add_root(ridx_file);
    }

    // Open overlay with clean code
    let clean_code = "pub fn valid_func() -> i32 { 42 }\n";
    overlay_mgr.open_document(&uri, lib_file.to_str().unwrap(), 1, clean_code.to_string());

    let req = json!({ "params": {} });
    let report = handle_workspace_diagnostic(&req, &overlay_mgr, &multi_root);
    let items = report["items"].as_array().expect("workspace diagnostic items");
    assert!(items.is_empty() || items[0]["items"].as_array().unwrap().is_empty());

    // Open overlay with broken syntax
    let broken_code = "pub fn broken( { \n";
    overlay_mgr.open_document(&uri, lib_file.to_str().unwrap(), 2, broken_code.to_string());

    let report_broken = handle_workspace_diagnostic(&req, &overlay_mgr, &multi_root);
    let broken_items = report_broken["items"].as_array().unwrap();
    let file_report = broken_items.iter().find(|it| it["uri"].as_str() == Some(&uri));
    assert!(file_report.is_some(), "Must report diagnostics for broken overlay");
    assert!(
        !file_report.unwrap()["items"].as_array().unwrap().is_empty(),
        "Must contain syntax error items"
    );
}

#[test]
fn test_l2_workspace_execute_commands() {
    let mut overlay_mgr = OverlayManager::new();

    // 1. ridx.resolveWorkspace
    let resolve_req = json!({
        "params": {
            "command": "ridx.resolveWorkspace",
            "rootUri": "file:///workspace/root"
        }
    });
    let resolve_res = handle_execute_command(&resolve_req, &mut overlay_mgr).unwrap();
    assert_eq!(resolve_res["resolved"], true);
    assert_eq!(resolve_res["status"], "ready");

    // 2. ridx.showReferences
    let show_req = json!({
        "params": {
            "command": "ridx.showReferences",
            "uri": "file:///workspace/src/lib.rs",
            "position": { "line": 5, "character": 4 },
            "locations": [
                {
                    "uri": "file:///workspace/src/lib.rs",
                    "range": {
                        "start": { "line": 10, "character": 4 },
                        "end": { "line": 10, "character": 9 }
                    }
                }
            ]
        }
    });
    let show_res = handle_execute_command(&show_req, &mut overlay_mgr).unwrap();
    assert_eq!(show_res["success"], true);
    assert_eq!(show_res["count"], 1);

    // 3. Unknown command returns error
    let unknown_req = json!({
        "params": {
            "command": "invalid.command.name"
        }
    });
    let err_res = handle_execute_command(&unknown_req, &mut overlay_mgr);
    assert!(err_res.is_err(), "Unknown command must return Err");
}

#[test]
fn test_l2_workspace_file_operations_lifecycle() {
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let mut multi_root = MultiRootWorkspace::new();

    let old_uri = "file:///tmp/workspace_test/old.rs";
    let old_path = "/tmp/workspace_test/old.rs";
    let new_uri = "file:///tmp/workspace_test/new.rs";
    let new_path = "/tmp/workspace_test/new.rs";

    // 1. Open document in overlay
    overlay_mgr.open_document(old_uri, old_path, 1, "fn rename_me() {}\n".to_string());
    assert!(overlay_mgr.get(old_path).is_some());

    // 2. Dispatch workspace/didRenameFiles
    let rename_msg = json!({
        "params": {
            "files": [
                { "oldUri": old_uri, "newUri": new_uri }
            ]
        }
    });
    multi_root.handle_file_event(
        "workspace/didRenameFiles",
        &rename_msg,
        &mut overlay_mgr,
        &mut conv,
    );

    assert!(
        overlay_mgr.get(old_path).is_none(),
        "Old file path must be closed in overlay"
    );
    assert!(
        overlay_mgr.get(new_path).is_some(),
        "New file path must be active in overlay"
    );

    // 3. Dispatch workspace/didDeleteFiles
    let delete_msg = json!({
        "params": {
            "files": [
                { "uri": new_uri }
            ]
        }
    });
    multi_root.handle_file_event(
        "workspace/didDeleteFiles",
        &delete_msg,
        &mut overlay_mgr,
        &mut conv,
    );

    assert!(
        overlay_mgr.get(new_path).is_none(),
        "Deleted file must be closed in overlay"
    );
}
