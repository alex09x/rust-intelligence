/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Unit tests for root crate and dependency qualifier resolution.

use crate::lsp::intelligence::handle_completion;
use crate::lsp::protocol::PositionConverter;
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter, RawDefinition, WorkspaceIndex};
use serde_json::json;
use std::fs;

#[test]
fn test_completion_root_crate_qualifier_core_cell() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_core_colon_{}", std::process::id()));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    let code = "fn main() {\n    core::c\n}\n";
    let _ = fs::write(&src_file, code);

    let src_path = src_file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("core", "1.98.1");

    writer.add_definition_with_owners(
        RawDefinition {
            def_id: 1,
            kind: DefKind::Mod,
            visibility: 0,
            is_macro_expansion: false,
            file: src_path.clone(),
            name: "cell".to_string(),
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 5,
            path: "core::cell".to_string(),
            signature: None,
        },
        vec!["core".to_string()],
    );

    writer.add_definition_with_owners(
        RawDefinition {
            def_id: 2,
            kind: DefKind::Struct,
            visibility: 0,
            is_macro_expansion: false,
            file: src_path.clone(),
            name: "Cell".to_string(),
            start_line: 2,
            start_col: 1,
            end_line: 2,
            end_col: 5,
            path: "core::cell::Cell".to_string(),
            signature: None,
        },
        vec!["core::cell".to_string()],
    );

    let ridx_file = temp_dir.join("core.ridx");
    let mut f = fs::File::create(&ridx_file).unwrap();
    writer.write_to(&mut f).unwrap();
    drop(f);

    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // 1. Completion after core:: returns cell
    let uri = format!("file://{}", src_file.to_str().unwrap());
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 10 }
        }
    });
    let res = handle_completion(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    let items = res["items"].as_array().unwrap();
    assert!(items.iter().any(|it| it["label"] == "cell"), "Should complete cell after core::");

    // 2. Completion after core::cell:: returns Cell
    let code_cell = "fn main() {\n    core::cell::\n}\n";
    let _ = fs::write(&src_file, code_cell);
    let req_cell = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 16 }
        }
    });
    let res_cell = handle_completion(&req_cell, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    let items_cell = res_cell["items"].as_array().unwrap();
    assert!(items_cell.iter().any(|it| it["label"] == "Cell"), "Should complete Cell after core::cell::");

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_completion_dep_crate_qualifier_with_synthetic_span() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_dep_qual_{}", std::process::id()));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let app_src = temp_dir.join("src/lib.rs");
    let code = "mod private {\n    fn test() {\n        dep_lib::\n    }\n}\n";
    let _ = fs::write(&app_src, code);

    // 1. Dependency shard
    let mut dep_writer = IndexWriter::new("dep_lib", "0.1.0");
    dep_writer.add_definition_with_owners(
        RawDefinition {
            def_id: 1,
            kind: DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            file: "dep/src/lib.rs".to_string(),
            name: "forward_to_deserialize_any".to_string(),
            start_line: 10,
            start_col: 1,
            end_line: 10,
            end_col: 30,
            path: "dep_lib::forward_to_deserialize_any".to_string(),
            signature: Some("fn()".to_string()),
        },
        vec!["dep_lib".to_string()],
    );
    let dep_ridx = temp_dir.join("dep_lib.ridx");
    let mut f_dep = fs::File::create(&dep_ridx).unwrap();
    dep_writer.write_to(&mut f_dep).unwrap();
    drop(f_dep);

    // 2. App shard with synthetic span at (1, 1) representing `extern crate dep_lib;`
    let mut app_writer = IndexWriter::new("app", "0.1.0");
    app_writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Use,
        visibility: 0,
        is_macro_expansion: false,
        file: app_src.to_str().unwrap().to_string(),
        name: "dep_lib".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 1,
        path: "app::dep_lib".to_string(),
        signature: None,
    });
    app_writer.add_definition(RawDefinition {
        def_id: 3,
        kind: DefKind::Mod,
        visibility: 0,
        is_macro_expansion: false,
        file: app_src.to_str().unwrap().to_string(),
        name: "private".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 5,
        end_col: 2,
        path: "app::private".to_string(),
        signature: None,
    });
    let app_ridx = temp_dir.join("app.ridx");
    let mut f_app = fs::File::create(&app_ridx).unwrap();
    app_writer.write_to(&mut f_app).unwrap();
    drop(f_app);

    let mut ws = WorkspaceIndex::open(&temp_dir).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));

    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let uri = format!("file://{}", app_src.to_str().unwrap());
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 2, "character": 17 }
        }
    });

    let res = handle_completion(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    let items = res["items"].as_array().unwrap();
    assert!(
        items.iter().any(|it| it["label"] == "forward_to_deserialize_any"),
        "Should complete dep_lib items without falling into synthetic span or enclosing module"
    );

    let _ = fs::remove_dir_all(&temp_dir);
}
