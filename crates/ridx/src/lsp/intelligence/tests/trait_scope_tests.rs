/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use crate::lsp::intelligence::handle_completion;
use crate::lsp::protocol::PositionConverter;
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter, RawDefinition, RawReference, RawType, WorkspaceIndex};
use serde_json::json;
use std::collections::HashSet;
use std::fs;

#[test]
fn test_completion_trait_scope_filtering() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_trait_scope_{}", std::process::id()));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    let code = "fn main() {\n    let val = MyType;\n    val.\n}\n";
    let _ = fs::write(&src_file, code);

    let src_path = src_file.to_str().unwrap().to_string();

    let build_index = |include_ref: bool| -> WorkspaceIndex {
        let mut writer = IndexWriter::new("test_traits", "0.1.0");

        // 1. Inherent method on test_traits::MyType with &self
        writer.add_definition_with_owners(
            RawDefinition {
                def_id: 1,
                kind: DefKind::AssocFn,
                visibility: 0,
                is_macro_expansion: false,
                file: src_path.clone(),
                name: "inherent_op".to_string(),
                start_line: 10,
                start_col: 5,
                end_line: 10,
                end_col: 16,
                path: "test_traits::MyType::inherent_op".to_string(),
                signature: Some("fn(&self)".to_string()),
            },
            vec!["test_traits::MyType".to_string()],
        );

        // 2. Static associated function without self receiver (e.g. MyType::new)
        writer.add_definition_with_owners(
            RawDefinition {
                def_id: 2,
                kind: DefKind::AssocFn,
                visibility: 0,
                is_macro_expansion: false,
                file: src_path.clone(),
                name: "new".to_string(),
                start_line: 12,
                start_col: 5,
                end_line: 12,
                end_col: 8,
                path: "test_traits::MyType::new".to_string(),
                signature: Some("fn() -> MyType".to_string()),
            },
            vec!["test_traits::MyType".to_string()],
        );

        // 3. Prelude Trait Clone (canonical path core::clone::Clone)
        writer.add_definition_with_owners(
            RawDefinition {
                def_id: 3,
                kind: DefKind::Trait,
                visibility: 0,
                is_macro_expansion: false,
                file: src_path.clone(),
                name: "Clone".to_string(),
                start_line: 20,
                start_col: 1,
                end_line: 20,
                end_col: 12,
                path: "core::clone::Clone".to_string(),
                signature: None,
            },
            Vec::new(),
        );
        writer.add_definition_with_owners(
            RawDefinition {
                def_id: 4,
                kind: DefKind::AssocFn,
                visibility: 0,
                is_macro_expansion: false,
                file: src_path.clone(),
                name: "clone".to_string(),
                start_line: 21,
                start_col: 5,
                end_line: 21,
                end_col: 10,
                path: "core::clone::Clone::clone".to_string(),
                signature: Some("fn(&self) -> Self".to_string()),
            },
            vec!["core::clone::Clone".to_string()],
        );

        // 4. Non-prelude Trait SpecialTrait
        writer.add_definition_with_owners(
            RawDefinition {
                def_id: 5,
                kind: DefKind::Trait,
                visibility: 0,
                is_macro_expansion: false,
                file: src_path.clone(),
                name: "SpecialTrait".to_string(),
                start_line: 30,
                start_col: 1,
                end_line: 30,
                end_col: 19,
                path: "test_traits::SpecialTrait".to_string(),
                signature: None,
            },
            Vec::new(),
        );
        writer.add_definition_with_owners(
            RawDefinition {
                def_id: 6,
                kind: DefKind::AssocFn,
                visibility: 0,
                is_macro_expansion: false,
                file: src_path.clone(),
                name: "special_op".to_string(),
                start_line: 31,
                start_col: 5,
                end_line: 31,
                end_col: 15,
                path: "test_traits::SpecialTrait::special_op".to_string(),
                signature: Some("fn(&self)".to_string()),
            },
            vec!["test_traits::SpecialTrait".to_string()],
        );

        // 5. Custom Trait with short name Iterator in user crate (must NOT match prelude)
        writer.add_definition_with_owners(
            RawDefinition {
                def_id: 7,
                kind: DefKind::Trait,
                visibility: 0,
                is_macro_expansion: false,
                file: src_path.clone(),
                name: "Iterator".to_string(),
                start_line: 40,
                start_col: 1,
                end_line: 40,
                end_col: 15,
                path: "test_traits::custom::Iterator".to_string(),
                signature: None,
            },
            Vec::new(),
        );
        writer.add_definition_with_owners(
            RawDefinition {
                def_id: 8,
                kind: DefKind::AssocFn,
                visibility: 0,
                is_macro_expansion: false,
                file: src_path.clone(),
                name: "custom_step".to_string(),
                start_line: 41,
                start_col: 5,
                end_line: 41,
                end_col: 16,
                path: "test_traits::custom::Iterator::custom_step".to_string(),
                signature: Some("fn(&mut self)".to_string()),
            },
            vec!["test_traits::custom::Iterator".to_string()],
        );

        // Deref/trait links for MyType
        writer.add_deref("test_traits::MyType", "core::clone::Clone");
        writer.add_deref("test_traits::MyType", "test_traits::SpecialTrait");
        writer.add_deref("test_traits::MyType", "test_traits::custom::Iterator");

        // Type for `val` at line 3, col 5
        writer.add_type_with_owner(
            RawType {
                file: src_path.clone(),
                start_line: 3,
                start_col: 5,
                end_line: 3,
                end_col: 8,
                type_name: "MyType".to_string(),
            },
            Some("test_traits::MyType".to_string()),
        );

        if include_ref {
            writer.add_reference(RawReference {
                file: src_path.clone(),
                start_line: 1,
                start_col: 1,
                end_line: 1,
                end_col: 20,
                target_def_id: Some(5),
                target_path: "test_traits::SpecialTrait".to_string(),
                target_file: Some(src_path.clone()),
                target_unit: Some("test_traits".to_string()),
                target_line: 30,
                target_col: 1,
                kind: DefKind::Trait,
                is_write: false,
                is_macro_expansion: false,
            });
        }

        let ridx_file = temp_dir.join(format!("index_{include_ref}.ridx"));
        let mut f = fs::File::create(&ridx_file).unwrap();
        writer.write_to(&mut f).unwrap();
        drop(f);

        let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
        ws.set_workspace_root(Some(temp_dir.clone()));
        ws
    };

    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = format!("file://{}", src_file.to_str().unwrap());
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 2, "character": 8 }
        }
    });

    // 1. Without reference:
    // - inherent_op is present (inherent method)
    // - new is NOT present (no self receiver)
    // - clone is present (canonical prelude trait core::clone::Clone)
    // - special_op is NOT present (non-prelude trait without import)
    // - custom_step is NOT present (custom user trait test_traits::custom::Iterator without import)
    let ws_no_ref = build_index(false);
    let res = handle_completion(&req, Some(&ws_no_ref), &overlay_mgr, &mut conv).unwrap();
    let items = res["items"].as_array().unwrap();
    let labels: HashSet<&str> = items.iter().filter_map(|i| i["label"].as_str()).collect();

    assert!(labels.contains("inherent_op"), "must contain inherent_op");
    assert!(
        !labels.contains("new"),
        "must NOT contain function without self receiver"
    );
    assert!(
        labels.contains("clone"),
        "must contain canonical prelude trait method clone"
    );
    assert!(
        !labels.contains("special_op"),
        "must NOT contain non-prelude trait method without import"
    );
    assert!(
        !labels.contains("custom_step"),
        "must NOT treat custom user Iterator trait as prelude without canonical core path"
    );

    // 2. With reference to SpecialTrait: special_op is now present
    let ws_with_ref = build_index(true);
    let res_with = handle_completion(&req, Some(&ws_with_ref), &overlay_mgr, &mut conv).unwrap();
    let items_with = res_with["items"].as_array().unwrap();
    let labels_with: HashSet<&str> = items_with
        .iter()
        .filter_map(|i| i["label"].as_str())
        .collect();

    assert!(labels_with.contains("inherent_op"));
    assert!(!labels_with.contains("new"));
    assert!(labels_with.contains("clone"));
    assert!(
        labels_with.contains("special_op"),
        "must contain non-prelude trait method when imported/referenced"
    );
    assert!(
        !labels_with.contains("custom_step"),
        "custom trait still not in scope"
    );

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_signature_takes_receiver_lifetimes() {
    use crate::lsp::intelligence::owner_completion::signature_takes_receiver;

    assert!(signature_takes_receiver("fn(&self)"));
    assert!(signature_takes_receiver("fn(&self, x: i32)"));
    assert!(signature_takes_receiver("fn(&mut self)"));
    assert!(signature_takes_receiver("fn(self)"));
    assert!(signature_takes_receiver("fn(mut self)"));
    assert!(signature_takes_receiver("fn(&'a self)"));
    assert!(signature_takes_receiver("fn(&'a mut self)"));
    assert!(signature_takes_receiver("fn(&'_ self)"));
    assert!(signature_takes_receiver("fn(&'static self)"));
    assert!(signature_takes_receiver("fn(self: Box<Self>)"));
    assert!(signature_takes_receiver("fn(self: &Self)"));

    assert!(!signature_takes_receiver("fn()"));
    assert!(!signature_takes_receiver("fn(x: i32)"));
    assert!(!signature_takes_receiver("fn(&Other)"));
    assert!(!signature_takes_receiver("fn(&'a Other)"));
    assert!(!signature_takes_receiver("fn(new_val: i32)"));
}

