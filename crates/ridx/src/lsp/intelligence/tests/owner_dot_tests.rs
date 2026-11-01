/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Unit tests for dot-completion (`receiver.`) via owner-index and autoderef.

use crate::lsp::intelligence::handle_completion;
use crate::lsp::protocol::PositionConverter;
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter, RawDefinition, RawType, WorkspaceIndex};
use serde_json::json;
use std::fs;

#[test]
fn test_completion_owner_method_isolation() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_owner_iso_{}", std::process::id()));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    let code = "fn main() {\n    let a = ServiceA;\n    a.ex\n}\n";
    let _ = fs::write(&src_file, code);

    let src_path = src_file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("test_iso", "0.1.0");

    // Add ServiceA::execute(&self) with owner "test_iso::ServiceA"
    writer.add_definition_with_owners(
        RawDefinition {
            def_id: 1,
            kind: DefKind::AssocFn,
            visibility: 0,
            is_macro_expansion: false,
            file: src_path.clone(),
            name: "execute".to_string(),
            start_line: 10,
            start_col: 5,
            end_line: 10,
            end_col: 12,
            path: "test_iso::ServiceA::execute".to_string(),
            signature: Some("fn execute(&self)".to_string()),
        },
        vec!["test_iso::ServiceA".to_string()],
    );

    // Add ServiceB::execute(&self) with owner "test_iso::ServiceB"
    writer.add_definition_with_owners(
        RawDefinition {
            def_id: 2,
            kind: DefKind::AssocFn,
            visibility: 0,
            is_macro_expansion: false,
            file: src_path.clone(),
            name: "execute".to_string(),
            start_line: 20,
            start_col: 5,
            end_line: 20,
            end_col: 12,
            path: "test_iso::ServiceB::execute".to_string(),
            signature: Some("fn execute(&self)".to_string()),
        },
        vec!["test_iso::ServiceB".to_string()],
    );

    // Expression `a` at line 3, col 5 has owner "test_iso::ServiceA"
    writer.add_type_with_owner(
        RawType { file: src_path.clone(), start_line: 3, start_col: 5, end_line: 3, end_col: 6, type_name: "ServiceA".to_string() },
        Some("test_iso::ServiceA".to_string()),
    );

    let ridx_file = temp_dir.join("index.ridx");
    let mut f = fs::File::create(&ridx_file).unwrap();
    writer.write_to(&mut f).unwrap();
    drop(f);

    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // Query completion at `a.ex` (line 3, col 8, 0-based: line 2, character 8)
    let uri = format!("file://{}", src_file.to_str().unwrap());
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 2, "character": 8 }
        }
    });

    let res = handle_completion(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    let items = res["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "Must isolate and return exactly 1 item");
    assert_eq!(items[0]["label"], "execute");
    assert!(items[0]["detail"].as_str().unwrap().contains("ServiceA::execute"), "Detail must refer to ServiceA, not ServiceB");

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_completion_autoderef_chain() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_deref_chain_{}", std::process::id()));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    let code = "fn main() {\n    let ptr = SmartPtr;\n    ptr.in\n}\n";
    let _ = fs::write(&src_file, code);

    let src_path = src_file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("test_deref", "0.1.0");

    // Add SmartPtr -> InnerTarget deref mapping
    writer.add_deref("test_deref::SmartPtr", "test_deref::InnerTarget");

    // Add InnerTarget::inner_action(&self) with owner "test_deref::InnerTarget"
    writer.add_definition_with_owners(
        RawDefinition {
            def_id: 1,
            kind: DefKind::AssocFn,
            visibility: 0,
            is_macro_expansion: false,
            file: src_path.clone(),
            name: "inner_action".to_string(),
            start_line: 10,
            start_col: 5,
            end_line: 10,
            end_col: 17,
            path: "test_deref::InnerTarget::inner_action".to_string(),
            signature: Some("fn inner_action(&self)".to_string()),
        },
        vec!["test_deref::InnerTarget".to_string()],
    );

    // Expression `ptr` at line 3, col 5 has owner "test_deref::SmartPtr"
    writer.add_type_with_owner(
        RawType { file: src_path.clone(), start_line: 3, start_col: 5, end_line: 3, end_col: 8, type_name: "SmartPtr".to_string() },
        Some("test_deref::SmartPtr".to_string()),
    );

    let ridx_file = temp_dir.join("index.ridx");
    let mut f = fs::File::create(&ridx_file).unwrap();
    writer.write_to(&mut f).unwrap();
    drop(f);

    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // Query completion at `ptr.in` (line 3, col 10, 0-based: line 2, character 10)
    let uri = format!("file://{}", src_file.to_str().unwrap());
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 2, "character": 10 }
        }
    });

    let res = handle_completion(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    let items = res["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "Autoderef must find inner_action from InnerTarget");
    assert_eq!(items[0]["label"], "inner_action");

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_dot_fallback_completion_empty_prefix_finds_methods() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_dot_fallback_{}", std::process::id()));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    let code = "struct Wrapper(fixedbitset::FixedBitSet);\nimpl Wrapper {\n    fn check(&self) {\n        self.0.\n    }\n}\n";
    let _ = fs::write(&src_file, code);

    let src_path = src_file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("fixedbitset", "0.4.0");

    // Add methods on FixedBitSet without defs_by_owner so owner_completion falls through to fallback_completion
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::AssocFn,
        visibility: 1,
        is_macro_expansion: false,
        file: src_path.clone(),
        name: "is_disjoint".to_string(),
        start_line: 10,
        start_col: 5,
        end_line: 10,
        end_col: 20,
        path: "fixedbitset::FixedBitSet::is_disjoint".to_string(),
        signature: Some("fn is_disjoint(&self, other: &Self) -> bool".to_string()),
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::AssocFn,
        visibility: 1,
        is_macro_expansion: false,
        file: src_path.clone(),
        name: "insert".to_string(),
        start_line: 20,
        start_col: 5,
        end_line: 20,
        end_col: 20,
        path: "fixedbitset::FixedBitSet::insert".to_string(),
        signature: Some("fn insert(&mut self, bit: usize)".to_string()),
    });

    // Add type for `self.0` at line 4, col 9-15
    writer.add_type_with_owner(
        RawType { file: src_path.clone(), start_line: 4, start_col: 9, end_line: 4, end_col: 15, type_name: "fixedbitset::FixedBitSet".to_string() },
        Some("fixedbitset::FixedBitSet".to_string()),
    );

    let ridx_file = temp_dir.join("index.ridx");
    let mut f = fs::File::create(&ridx_file).unwrap();
    writer.write_to(&mut f).unwrap();
    drop(f);

    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // Query completion at `self.0.` (line 4 col 16, 0-based: line 3, character 15)
    let uri = format!("file://{}", src_file.to_str().unwrap());
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 3, "character": 15 }
        }
    });

    let res = handle_completion(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    let items = res["items"].as_array().unwrap();
    let labels: Vec<&str> = items.iter().filter_map(|it| it["label"].as_str()).collect();
    assert!(labels.contains(&"is_disjoint"), "Fallback dot completion must find is_disjoint, got: {:?}", labels);
    assert!(labels.contains(&"insert"), "Fallback dot completion must find insert, got: {:?}", labels);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_completion_multiline_chained_dot() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_multiline_dot_{}", std::process::id()));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    let code = "fn main() {\n    builder\n        .step()\n        .b\n}\n";
    let _ = fs::write(&src_file, code);

    let src_path = src_file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("test_chain", "0.1.0");
    writer.add_definition_with_owners(
        RawDefinition {
            def_id: 1,
            kind: DefKind::AssocFn,
            visibility: 0,
            is_macro_expansion: false,
            file: src_path.clone(),
            name: "build".to_string(),
            start_line: 10,
            start_col: 5,
            end_line: 10,
            end_col: 10,
            path: "test_chain::Builder::build".to_string(),
            signature: Some("fn build(&mut self)".to_string()),
        },
        vec!["Builder".to_string()],
    );
    writer.add_type_with_owner(
        RawType { file: src_path.clone(), start_line: 2, start_col: 5, end_line: 3, end_col: 15, type_name: "&mut Builder".to_string() },
        Some("test_chain::Builder".to_string()),
    );

    let ridx_file = temp_dir.join("index.ridx");
    let mut f = fs::File::create(&ridx_file).unwrap();
    writer.write_to(&mut f).unwrap();
    drop(f);

    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let uri = format!("file://{}", src_file.to_str().unwrap());
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 3, "character": 10 }
        }
    });

    let res = handle_completion(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    let items = res["items"].as_array().unwrap();
    let labels: Vec<&str> = items.iter().filter_map(|it| it["label"].as_str()).collect();
    assert!(labels.contains(&"build"), "Multiline chained dot completion must resolve receiver on previous line, got: {:?}", labels);

    let _ = fs::remove_dir_all(&temp_dir);
}
