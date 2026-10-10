/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Unit tests for colon-completion (`path::`) via owner-index.

use crate::lsp::intelligence::handle_completion;
use crate::lsp::protocol::PositionConverter;
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter, RawDefinition, WorkspaceIndex};
use serde_json::json;
use std::fs;

#[test]
fn test_completion_enum_variant_colon() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_enum_colon_{}", std::process::id()));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    let code = "fn main() {\n    Status::\n}\n";
    let _ = fs::write(&src_file, code);

    let src_path = src_file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("test_enum", "0.1.0");

    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Enum,
        visibility: 0,
        is_macro_expansion: false,
        file: src_path.clone(),
        name: "Status".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 7,
        path: "test_enum::Status".to_string(),
        signature: None,
    });

    // Variants with owner "test_enum::Status"
    writer.add_definition_with_owners(
        RawDefinition {
            def_id: 2,
            kind: DefKind::Variant,
            visibility: 0,
            is_macro_expansion: false,
            file: src_path.clone(),
            name: "Active".to_string(),
            start_line: 2,
            start_col: 5,
            end_line: 2,
            end_col: 11,
            path: "test_enum::Status::Active".to_string(),
            signature: None,
        },
        vec!["test_enum::Status".to_string()],
    );

    writer.add_definition_with_owners(
        RawDefinition {
            def_id: 3,
            kind: DefKind::Variant,
            visibility: 0,
            is_macro_expansion: false,
            file: src_path.clone(),
            name: "Inactive".to_string(),
            start_line: 3,
            start_col: 5,
            end_line: 3,
            end_col: 13,
            path: "test_enum::Status::Inactive".to_string(),
            signature: None,
        },
        vec!["test_enum::Status".to_string()],
    );

    let ridx_file = temp_dir.join("index.ridx");
    let mut f = fs::File::create(&ridx_file).unwrap();
    writer.write_to(&mut f).unwrap();
    drop(f);

    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // Query completion at `Status::` (line 2, character 12)
    let uri = format!("file://{}", src_file.to_str().unwrap());
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 12 }
        }
    });

    let res = handle_completion(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    let items = res["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    let names: Vec<&str> = items.iter().map(|i| i["label"].as_str().unwrap()).collect();
    assert!(names.contains(&"Active"));
    assert!(names.contains(&"Inactive"));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_completion_module_colon() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_mod_colon_{}", std::process::id()));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    let code = "fn main() {\n    tools::\n}\n";
    let _ = fs::write(&src_file, code);

    let src_path = src_file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("test_mod", "0.1.0");

    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Mod,
        visibility: 0,
        is_macro_expansion: false,
        file: src_path.clone(),
        name: "tools".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 6,
        path: "test_mod::tools".to_string(),
        signature: None,
    });

    // Module items with owner "test_mod::tools"
    writer.add_definition_with_owners(
        RawDefinition {
            def_id: 2,
            kind: DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            file: src_path.clone(),
            name: "build".to_string(),
            start_line: 2,
            start_col: 5,
            end_line: 2,
            end_col: 10,
            path: "test_mod::tools::build".to_string(),
            signature: Some("fn build()".to_string()),
        },
        vec!["test_mod::tools".to_string()],
    );

    writer.add_definition_with_owners(
        RawDefinition {
            def_id: 3,
            kind: DefKind::Struct,
            visibility: 0,
            is_macro_expansion: false,
            file: src_path.clone(),
            name: "Config".to_string(),
            start_line: 3,
            start_col: 5,
            end_line: 3,
            end_col: 11,
            path: "test_mod::tools::Config".to_string(),
            signature: None,
        },
        vec!["test_mod::tools".to_string()],
    );

    let ridx_file = temp_dir.join("index.ridx");
    let mut f = fs::File::create(&ridx_file).unwrap();
    writer.write_to(&mut f).unwrap();
    drop(f);

    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // Query completion at `tools::` (line 2, character 11)
    let uri = format!("file://{}", src_file.to_str().unwrap());
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 11 }
        }
    });

    let res = handle_completion(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    let items = res["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    let names: Vec<&str> = items.iter().map(|i| i["label"].as_str().unwrap()).collect();
    assert!(names.contains(&"build"));
    assert!(names.contains(&"Config"));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_completion_trait_method_dedup() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_trait_dedup_{}", std::process::id()));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    let code = "fn main() {\n    Worker::\n}\n";
    let _ = fs::write(&src_file, code);

    let src_path = src_file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("test_trait", "0.1.0");

    // Trait definition: Worker
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Trait,
        visibility: 0,
        is_macro_expansion: false,
        file: src_path.clone(),
        name: "Worker".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 7,
        path: "test_trait::Worker".to_string(),
        signature: None,
    });

    // Trait declaration: Worker::work(&self) owned by test_trait::Worker
    writer.add_definition_with_owners(
        RawDefinition {
            def_id: 2,
            kind: DefKind::AssocFn,
            visibility: 0,
            is_macro_expansion: false,
            file: src_path.clone(),
            name: "work".to_string(),
            start_line: 2,
            start_col: 5,
            end_line: 2,
            end_col: 9,
            path: "test_trait::Worker::work".to_string(),
            signature: Some("fn work(&self)".to_string()),
        },
        vec!["test_trait::Worker".to_string()],
    );

    // Struct TaskA implements Worker: work(&self) owned ONLY by test_trait::TaskA (P3.2)
    writer.add_definition_with_owners(
        RawDefinition {
            def_id: 3,
            kind: DefKind::AssocFn,
            visibility: 0,
            is_macro_expansion: false,
            file: src_path.clone(),
            name: "work".to_string(),
            start_line: 10,
            start_col: 5,
            end_line: 10,
            end_col: 9,
            path: "<test_trait::TaskA as test_trait::Worker>::work".to_string(),
            signature: Some("fn work(&self)".to_string()),
        },
        vec!["test_trait::TaskA".to_string()],
    );

    // Struct TaskB implements Worker: work(&self) owned ONLY by test_trait::TaskB (P3.2)
    writer.add_definition_with_owners(
        RawDefinition {
            def_id: 4,
            kind: DefKind::AssocFn,
            visibility: 0,
            is_macro_expansion: false,
            file: src_path.clone(),
            name: "work".to_string(),
            start_line: 20,
            start_col: 5,
            end_line: 20,
            end_col: 9,
            path: "<test_trait::TaskB as test_trait::Worker>::work".to_string(),
            signature: Some("fn work(&self)".to_string()),
        },
        vec!["test_trait::TaskB".to_string()],
    );

    let ridx_file = temp_dir.join("index.ridx");
    let mut f = fs::File::create(&ridx_file).unwrap();
    writer.write_to(&mut f).unwrap();
    drop(f);

    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // Query completion at `Worker::` (line 2, character 12)
    let uri = format!("file://{}", src_file.to_str().unwrap());
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 12 }
        }
    });

    let res = handle_completion(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    let items = res["items"].as_array().unwrap();
    // Must return exactly 1 item (the trait declaration), NOT 3 duplicates from TaskA and TaskB!
    assert_eq!(items.len(), 1, "Trait completion must not include duplicate impl methods from every implementor");
    assert_eq!(items[0]["label"], "work");
    assert_eq!(items[0]["detail"], "fn work(&self) (in test_trait::Worker::work)");

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_completion_bevy_comp_colon_1_cell() {
    if !std::path::Path::new("/tmp/mitts-bench/bevy").is_dir() {
        return;
    }
    let ws = WorkspaceIndex::open("/tmp/mitts-bench/bevy").unwrap();
    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let file = "/tmp/mitts-bench/bevy/crates/bevy_ecs/src/system/system_registry.rs";
    let text = std::fs::read_to_string(file).unwrap();
    let uri = format!("file://{file}");
    overlay_mgr.open_document(&uri, file, 1, text);

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 909, "character": 14 }
        }
    });

    let res = handle_completion(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    let items = res["items"].as_array().unwrap();
    assert!(!items.is_empty(), "Should complete items after core:: in system_registry.rs");

    let req_c = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 909, "character": 15 }
        }
    });
    let res_c = handle_completion(&req_c, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    let items_c = res_c["items"].as_array().unwrap();
    assert!(items_c.iter().any(|it| it["label"] == "cell"), "Should include cell");
}
