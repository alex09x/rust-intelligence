/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::setup_test_hierarchy;
use crate::lsp::hierarchy::call::*;
use crate::lsp::protocol::path_to_uri;
use crate::overlay::OverlayManager;
use serde_json::json;
use std::fs;

#[test]
fn test_call_hierarchy_incoming_and_outgoing() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_hierarchy();
    let src_file = temp_dir.join("src/lib.rs");
    let uri = path_to_uri(src_file.to_str().unwrap());

    let incoming_req = json!({
        "params": {
            "item": {
                "name": "add",
                "uri": uri,
                "range": {
                    "start": { "line": 0, "character": 7 },
                    "end": { "line": 0, "character": 10 }
                }
            }
        }
    });

    let incoming = handle_call_hierarchy_incoming_calls(&incoming_req, Some(&ws), &mut conv);
    let arr = incoming.as_array().unwrap();
    // Actual calls from caller and turbo_caller, skipping function value assignment and non-function context static _C
    assert_eq!(arr.len(), 2);
    let caller_names: Vec<&str> = arr
        .iter()
        .filter_map(|item| item["from"]["name"].as_str())
        .collect();
    assert!(caller_names.contains(&"caller"));
    assert!(caller_names.contains(&"turbo_caller"));

    // Test outgoing calls from caller: calls add(1, 2) and Foo(1)
    let caller_file = temp_dir.join("src/caller.rs");
    let caller_uri = path_to_uri(caller_file.to_str().unwrap());
    let outgoing_req = json!({
        "params": {
            "item": {
                "name": "caller",
                "uri": caller_uri,
                "range": {
                    "start": { "line": 0, "character": 7 },
                    "end": { "line": 4, "character": 1 }
                }
            }
        }
    });
    let outgoing = handle_call_hierarchy_outgoing_calls(&outgoing_req, Some(&ws), &mut conv);
    let out_arr = outgoing.as_array().unwrap();
    assert_eq!(out_arr.len(), 1);
    let names: Vec<&str> = out_arr
        .iter()
        .filter_map(|it| it["to"]["name"].as_str())
        .collect();
    assert!(!names.contains(&"caller"));
    assert!(names.contains(&"add"));

    // Outgoing calls: nonexistent file returns empty
    let non_exist_req = json!({
        "params": { "item": { "uri": "file:///nonexistent.rs", "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 1, "character": 0 } } } }
    });
    assert_eq!(
        handle_call_hierarchy_outgoing_calls(&non_exist_req, Some(&ws), &mut conv),
        json!([])
    );
    assert_eq!(
        handle_call_hierarchy_outgoing_calls(&outgoing_req, None, &mut conv),
        json!([])
    );

    // Incoming calls without workspace
    assert_eq!(
        handle_call_hierarchy_incoming_calls(&incoming_req, None, &mut conv),
        json!([])
    );

    // Prepare call hierarchy: workspace fallback on function
    let prep_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 8 }
        }
    });
    let prep_res = handle_prepare_call_hierarchy(&prep_req, Some(&ws), &overlay_mgr, &mut conv);
    assert!(!prep_res.as_array().unwrap().is_empty());

    // Prepare call hierarchy: workspace on struct (not a function -> returns empty)
    let prep_struct = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 12 }
        }
    });
    let prep_struct_res =
        handle_prepare_call_hierarchy(&prep_struct, Some(&ws), &overlay_mgr, &mut conv);
    assert!(prep_struct_res.as_array().unwrap().is_empty());

    // Prepare call hierarchy: workspace on whitespace (no definition -> returns empty)
    let prep_ws = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 0 }
        }
    });
    let prep_ws_res = handle_prepare_call_hierarchy(&prep_ws, Some(&ws), &overlay_mgr, &mut conv);
    assert!(prep_ws_res.as_array().unwrap().is_empty());

    // Prepare call hierarchy: with overlay on a function
    let mut ov_mgr = OverlayManager::new();
    ov_mgr.open_document(
        &uri,
        src_file.to_str().unwrap(),
        1,
        "pub fn add(x: i32) -> i32 { x }\n".to_string(),
    );
    let prep_ov_res = handle_prepare_call_hierarchy(&prep_req, Some(&ws), &ov_mgr, &mut conv);
    assert!(!prep_ov_res.as_array().unwrap().is_empty());

    // Prepare call hierarchy: with overlay on a struct (not a fn -> falls through to workspace)
    ov_mgr.open_document(
        &uri,
        src_file.to_str().unwrap(),
        2,
        "pub struct NotAFn;\n".to_string(),
    );
    let prep_ov_struct = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 12 }
        }
    });
    let prep_ov_struct_res =
        handle_prepare_call_hierarchy(&prep_ov_struct, Some(&ws), &ov_mgr, &mut conv);
    assert!(prep_ov_struct_res.as_array().unwrap().is_empty());

    // is_call_site edge cases
    assert!(!is_call_site("/nonexistent/file.rs", 1, 1));
    assert!(!is_call_site(src_file.to_str().unwrap(), 999, 1));
    let malformed_turbo = temp_dir.join("src/malformed.rs");
    fs::write(&malformed_turbo, "pub fn m() { add::<i32(1, 2); }\n").unwrap();
    assert!(!is_call_site(malformed_turbo.to_str().unwrap(), 1, 17));

    let _ = fs::remove_dir_all(&temp_dir);
}
