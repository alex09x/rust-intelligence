/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

pub mod call_tests;
pub mod call_trait_tests;
pub mod impl_table_tests;
pub mod type_tests;

use crate::lsp::protocol::PositionConverter;
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter, RawDefinition, RawReference, WorkspaceIndex};
use std::fs;
use std::path::PathBuf;

static HIERARCHY_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn setup_test_hierarchy() -> (PathBuf, WorkspaceIndex, OverlayManager, PositionConverter) {
    let id = HIERARCHY_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temp_dir =
        std::env::temp_dir().join(format!("ridx_test_hierarchy_{}_{}", std::process::id(), id));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    let code = "pub fn add(a: i32, b: i32) -> i32 { a + b }\npub struct Foo { pub x: i32 }\npub trait Bar { fn bar(&self); }\nimpl Bar for Foo { fn bar(&self) {} }\n";
    let _ = fs::write(&src_file, code);

    let src_file_str = src_file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("test_crate", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        file: src_file_str.clone(),
        name: "add".to_string(),
        start_line: 1,
        start_col: 8,
        end_line: 1,
        end_col: 11,
        path: "test_crate::add".to_string(),
        signature: Some("fn(i32, i32) -> i32".to_string()),
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        file: src_file_str.clone(),
        name: "Foo".to_string(),
        start_line: 2,
        start_col: 12,
        end_line: 2,
        end_col: 15,
        path: "test_crate::Foo".to_string(),
        signature: None,
    });
    writer.add_definition(RawDefinition {
        def_id: 3,
        kind: DefKind::Trait,
        visibility: 0,
        is_macro_expansion: false,
        file: src_file_str.clone(),
        name: "Bar".to_string(),
        start_line: 3,
        start_col: 11,
        end_line: 3,
        end_col: 14,
        path: "test_crate::Bar".to_string(),
        signature: None,
    });

    let caller_file = temp_dir.join("src/caller.rs");
    let caller_code =
        "pub fn caller() {\n    add(1, 2);\n    let callback = add;\n    Foo(1);\n}\n";
    let _ = fs::write(&caller_file, caller_code);
    let caller_str = caller_file.to_str().unwrap().to_string();

    writer.add_definition(RawDefinition {
        def_id: 4,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        file: caller_str.clone(),
        name: "caller".to_string(),
        start_line: 1,
        start_col: 8,
        end_line: 5,
        end_col: 2,
        path: "test_crate::caller".to_string(),
        signature: Some("fn()".to_string()),
    });

    // Reference 1: actual call add(1, 2) in caller.rs (cols 5..8)
    writer.add_reference(RawReference {
        file: caller_str.clone(),
        start_line: 2,
        start_col: 5,
        end_line: 2,
        end_col: 8,
        target_def_id: Some(1),
        target_path: "test_crate::add".to_string(),
        target_unit: None,
        target_file: Some(src_file_str.clone()),
        target_line: 1,
        target_col: 8,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });

    // Reference 2: function value assignment let callback = add; (cols 20..23)
    writer.add_reference(RawReference {
        file: caller_str.clone(),
        start_line: 3,
        start_col: 20,
        end_line: 3,
        end_col: 23,
        target_def_id: Some(1),
        target_path: "test_crate::add".to_string(),
        target_unit: None,
        target_file: Some(src_file_str.clone()),
        target_line: 1,
        target_col: 8,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });

    // Reference 3: Call to struct constructor Foo(1) (cols 5..8)
    writer.add_reference(RawReference {
        file: caller_str,
        start_line: 4,
        start_col: 5,
        end_line: 4,
        end_col: 8,
        target_def_id: Some(2),
        target_path: "test_crate::Foo".to_string(),
        target_unit: None,
        target_file: Some(src_file_str.clone()),
        target_line: 2,
        target_col: 12,
        kind: DefKind::Struct,
        is_write: false,
        is_macro_expansion: false,
    });

    // Reference 4: Impl reference for Foo (supertypes)
    writer.add_reference(RawReference {
        file: src_file_str.clone(),
        start_line: 4,
        start_col: 13,
        end_line: 4,
        end_col: 16,
        target_def_id: Some(2),
        target_path: "test_crate::Foo".to_string(),
        target_unit: None,
        target_file: Some(src_file_str.clone()),
        target_line: 2,
        target_col: 12,
        kind: DefKind::Impl,
        is_write: false,
        is_macro_expansion: false,
    });

    // Reference 5: Impl reference for Bar (subtypes)
    writer.add_reference(RawReference {
        file: src_file_str.clone(),
        start_line: 4,
        start_col: 5,
        end_line: 4,
        end_col: 8,
        target_def_id: Some(3),
        target_path: "test_crate::Bar".to_string(),
        target_unit: None,
        target_file: Some(src_file_str.clone()),
        target_line: 3,
        target_col: 11,
        kind: DefKind::Impl,
        is_write: false,
        is_macro_expansion: false,
    });

    let turbo_file = temp_dir.join("src/turbo.rs");
    let turbo_code = "pub fn turbo_caller() { add::<i32>(1, 2); }\n";
    let _ = fs::write(&turbo_file, turbo_code);
    let turbo_str = turbo_file.to_str().unwrap().to_string();

    writer.add_definition(RawDefinition {
        def_id: 5,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        file: turbo_str.clone(),
        name: "turbo_caller".to_string(),
        start_line: 1,
        start_col: 8,
        end_line: 1,
        end_col: 20,
        path: "test_crate::turbo_caller".to_string(),
        signature: Some("fn()".to_string()),
    });

    writer.add_reference(RawReference {
        file: turbo_str,
        start_line: 1,
        start_col: 25,
        end_line: 1,
        end_col: 28,
        target_def_id: Some(1),
        target_path: "test_crate::add".to_string(),
        target_unit: None,
        target_file: Some(src_file_str.clone()),
        target_line: 1,
        target_col: 8,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });

    // Reference 6: Call site in non-function context (static _C)
    let orphan_file = temp_dir.join("src/orphan.rs");
    let orphan_code = "static _C: () = { add(1, 2); };\n";
    let _ = fs::write(&orphan_file, orphan_code);
    let orphan_str = orphan_file.to_str().unwrap().to_string();

    writer.add_reference(RawReference {
        file: orphan_str,
        start_line: 1,
        start_col: 19,
        end_line: 1,
        end_col: 22,
        target_def_id: Some(1),
        target_path: "test_crate::add".to_string(),
        target_unit: None,
        target_file: Some(src_file_str),
        target_line: 1,
        target_col: 8,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });

    let ridx_file = temp_dir.join("index.ridx");
    let mut f = fs::File::create(&ridx_file).unwrap();
    writer.write_to(&mut f).unwrap();

    let ws = WorkspaceIndex::open(&ridx_file).unwrap();
    let overlay_mgr = OverlayManager::new();
    let conv = PositionConverter::new();

    (temp_dir, ws, overlay_mgr, conv)
}
