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
    assert!(
        items.iter().any(|it| it["label"] == "cell"),
        "Should complete cell after core::"
    );

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
    assert!(
        items_cell.iter().any(|it| it["label"] == "Cell"),
        "Should complete Cell after core::cell::"
    );

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
        items
            .iter()
            .any(|it| it["label"] == "forward_to_deserialize_any"),
        "Should complete dep_lib items without falling into synthetic span or enclosing module"
    );

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_multi_target_use_import_and_foreign_use_guard() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_multi_target_qual_{}", std::process::id()));
    let app_dir = temp_dir.join("app_crate");
    let _ = fs::create_dir_all(app_dir.join("src"));
    let src_file = app_dir.join("src/system.rs");
    let _ = fs::write(&src_file, "use crate::schedule::Schedule;\nfn test() { Schedule::default(); }\n");
    let src_path = src_file.to_str().unwrap().to_string();

    // 1. Foreign dependency crate `async_dep` with bare `Use` at root
    let mut dep_writer = IndexWriter::new("async_dep", "1.98.1");
    dep_writer.add_definition_with_owners(
        RawDefinition {
            def_id: 1,
            kind: DefKind::Use,
            visibility: 0,
            is_macro_expansion: false,
            file: "async_dep/src/lib.rs".to_string(),
            name: "Schedule".to_string(),
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 10,
            path: "Schedule".to_string(),
            signature: Some("async_dep::runnable::Schedule".to_string()),
        },
        vec!["async_dep".to_string()],
    );
    dep_writer.add_definition_with_owners(
        RawDefinition {
            def_id: 2,
            kind: DefKind::Struct,
            visibility: 0,
            is_macro_expansion: false,
            file: "async_dep/src/runnable.rs".to_string(),
            name: "Schedule".to_string(),
            start_line: 5,
            start_col: 1,
            end_line: 5,
            end_col: 15,
            path: "async_dep::runnable::Schedule".to_string(),
            signature: None,
        },
        vec!["async_dep::runnable".to_string()],
    );
    let dep_ridx = temp_dir.join("async_dep.ridx");
    let mut f_dep = fs::File::create(&dep_ridx).unwrap();
    dep_writer.write_to(&mut f_dep).unwrap();
    drop(f_dep);

    // 2. Primary crate `app_crate` target 0 (lib unit): contains schedule definition, but no test module
    let mut lib_writer = IndexWriter::new("app_crate", "1.98.1");
    lib_writer.add_definition_with_owners(
        RawDefinition {
            def_id: 10,
            kind: DefKind::Struct,
            visibility: 0,
            is_macro_expansion: false,
            file: app_dir.join("src/schedule.rs").to_str().unwrap().to_string(),
            name: "Schedule".to_string(),
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 15,
            path: "app_crate::schedule::Schedule".to_string(),
            signature: None,
        },
        vec!["app_crate::schedule".to_string()],
    );
    lib_writer.add_definition_with_owners(
        RawDefinition {
            def_id: 11,
            kind: DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            file: src_path.clone(),
            name: "run".to_string(),
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 10,
            path: "app_crate::system::run".to_string(),
            signature: None,
        },
        vec!["app_crate::system".to_string()],
    );
    let lib_ridx = temp_dir.join("app_crate_lib.ridx");
    let mut f_lib = fs::File::create(&lib_ridx).unwrap();
    lib_writer.write_to(&mut f_lib).unwrap();
    drop(f_lib);

    // 3. Primary crate `app_crate` target 1 (test unit): contains test module with `use crate::schedule::Schedule`
    let mut test_writer = IndexWriter::new("app_crate", "1.98.1");
    test_writer.add_definition_with_owners(
        RawDefinition {
            def_id: 20,
            kind: DefKind::Use,
            visibility: 0,
            is_macro_expansion: false,
            file: src_path.clone(),
            name: "Schedule".to_string(),
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 30,
            path: "app_crate::system::tests::Schedule".to_string(),
            signature: Some("app_crate::schedule::Schedule".to_string()),
        },
        vec!["app_crate::system::tests".to_string()],
    );
    let test_ridx = temp_dir.join("app_crate_test.ridx");
    let mut f_test = fs::File::create(&test_ridx).unwrap();
    test_writer.write_to(&mut f_test).unwrap();
    drop(f_test);

    let ws = WorkspaceIndex::open(&temp_dir).unwrap();

    // Resolving `Schedule` from `system.rs` should check all target readers for `app_crate`,
    // find the `use` declaration in target 1, and resolve to `app_crate::schedule::Schedule`
    // instead of mistakenly picking `async_dep`'s foreign bare `Use`.
    let res = ws.resolve_qualifier(&src_path, "Schedule");
    assert!(res.is_some(), "Should resolve Schedule via multi-target unit scan");
    let resolved = res.unwrap();
    assert_eq!(resolved.name, "Schedule");
    assert_eq!(resolved.path, "app_crate::schedule::Schedule");
    assert_eq!(resolved.kind, DefKind::Struct);

    let _ = fs::remove_dir_all(&temp_dir);
}
