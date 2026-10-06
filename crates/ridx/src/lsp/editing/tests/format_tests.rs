/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::setup_test_editing;
use crate::lsp::editing::actions::*;
use crate::lsp::editing::format::*;
use crate::lsp::protocol::path_to_uri;
use serde_json::json;
use std::fs;

#[test]
fn test_format_and_actions() {
    let (temp_dir, _ws, mut overlay_mgr, _conv) = setup_test_editing();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    // 1. Formatting
    let fmt_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    });
    let fmt_res = handle_formatting(&fmt_req, &overlay_mgr);
    assert!(fmt_res.as_array().is_some());
    let range_fmt = handle_range_formatting(&fmt_req, &overlay_mgr);
    assert!(range_fmt.as_array().is_some());
    let on_type_fmt = handle_on_type_formatting(&fmt_req, &overlay_mgr);
    assert!(on_type_fmt.as_array().is_some());

    let missing_fmt_req =
        json!({ "params": { "textDocument": { "uri": "file:///nonexistent/file.rs" } } });
    assert_eq!(handle_formatting(&missing_fmt_req, &overlay_mgr), json!([]));

    let unformatted_file = temp_dir.join("unformatted.rs");
    let unformatted_uri = path_to_uri(unformatted_file.to_str().unwrap());
    overlay_mgr.open_with_provenance(
        &unformatted_uri,
        unformatted_file.to_str().unwrap(),
        1,
        "fn  unformatted  (  x : i32 ) -> i32 {   x   }\n".to_string(),
        true,
    );
    let unformatted_req = json!({
        "params": {
            "textDocument": { "uri": &unformatted_uri }
        }
    });
    let unformatted_res = handle_formatting(&unformatted_req, &overlay_mgr);
    assert!(!unformatted_res.as_array().unwrap().is_empty());

    // 2. Code action
    let action_req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 1, "character": 0 }
            },
            "context": { "diagnostics": [] }
        }
    });
    let action_res = handle_code_action(&action_req, &overlay_mgr);
    assert!(action_res.is_array());
    let resolve_res = handle_code_action_resolve(&action_req);
    assert!(!resolve_res.is_null());

    let action_diag = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 1 } },
            "context": {
                "only": ["quickfix"],
                "diagnostics": [{
                    "code": "unused_variable",
                    "message": "unused variable: `foo`",
                    "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 1 } }
                }]
            }
        }
    });
    let act_res = handle_code_action(&action_diag, &overlay_mgr);
    assert_eq!(act_res.as_array().unwrap().len(), 1);

    // 3. Execute command
    assert!(handle_execute_command(&action_req, &mut overlay_mgr).is_null());
    let cmd_resolve = json!({ "params": { "command": "ridx.resolveWorkspace" } });
    assert_eq!(
        handle_execute_command(&cmd_resolve, &mut overlay_mgr)["resolved"],
        true
    );

    // 4. Edition helper
    let cargo_toml = temp_dir.join("Cargo.toml");
    fs::write(&cargo_toml, "[package]\nedition = \"2024\"\n").unwrap();
    assert_eq!(find_crate_edition(file_path.to_str().unwrap()), "2024");

    // Workspace inherited edition
    let sub_dir = temp_dir.join("sub_crate");
    fs::create_dir_all(&sub_dir).unwrap();
    let sub_file = sub_dir.join("src/lib.rs");
    fs::create_dir_all(sub_file.parent().unwrap()).unwrap();
    fs::write(&sub_file, "fn test() {}").unwrap();
    let sub_cargo = sub_dir.join("Cargo.toml");
    fs::write(
        &sub_cargo,
        "[package]\nname = \"sub\"\nedition.workspace = true\n",
    )
    .unwrap();
    let root_cargo = temp_dir.join("Cargo.toml");
    fs::write(&root_cargo, "[workspace.package]\nedition = \"2021\"\n").unwrap();
    assert_eq!(find_crate_edition(sub_file.to_str().unwrap()), "2021");

    // Already formatted file returns empty edits
    let already_fmt_file = temp_dir.join("already_formatted.rs");
    let already_fmt_uri = path_to_uri(already_fmt_file.to_str().unwrap());
    overlay_mgr.open_with_provenance(
        &already_fmt_uri,
        already_fmt_file.to_str().unwrap(),
        1,
        "fn already_formatted(x: i32) -> i32 {\n    x\n}\n".to_string(),
        true,
    );
    let already_fmt_req = json!({
        "params": {
            "textDocument": { "uri": &already_fmt_uri }
        }
    });
    assert_eq!(handle_formatting(&already_fmt_req, &overlay_mgr), json!([]));

    // Invalid syntax formatting returns empty edits
    let broken_fmt_file = temp_dir.join("broken.rs");
    let broken_fmt_uri = path_to_uri(broken_fmt_file.to_str().unwrap());
    overlay_mgr.open_with_provenance(
        &broken_fmt_uri,
        broken_fmt_file.to_str().unwrap(),
        1,
        "fn broken( {{{ \n".to_string(),
        true,
    );
    let broken_fmt_req = json!({
        "params": {
            "textDocument": { "uri": &broken_fmt_uri }
        }
    });
    assert_eq!(handle_formatting(&broken_fmt_req, &overlay_mgr), json!([]));

    // Edition 2018
    fs::write(&cargo_toml, "[package]\nedition = \"2018\"\n").unwrap();
    assert_eq!(find_crate_edition(file_path.to_str().unwrap()), "2018");

    // Edition 2015
    fs::write(&cargo_toml, "[package]\nedition = \"2015\"\n").unwrap();
    assert_eq!(find_crate_edition(file_path.to_str().unwrap()), "2015");

    // Edition with workspace.package 2024
    fs::write(
        &cargo_toml,
        "[package]\nname = \"root\"\nedition = true\n\n[workspace.package]\nedition = \"2024\"\n",
    )
    .unwrap();
    assert_eq!(find_crate_edition(file_path.to_str().unwrap()), "2024");

    let _ = fs::remove_dir_all(&temp_dir);
}
