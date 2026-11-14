/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

pub mod declaration_tests;
pub mod definition_tests;
pub mod highlight_self_tests;
pub mod implementation_head_tests;
pub mod implementation_header_tests;
pub mod implementation_tests;
pub mod ref_tests;
pub mod references_matrix_tests;
pub mod type_definition_compound_tests;
pub mod type_definition_tests;

use crate::lsp::protocol::PositionConverter;
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter, RawDefinition, RawReference, RawType, WorkspaceIndex};
use std::fs;
use std::path::PathBuf;

static NAV_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn setup_test_navigation() -> (PathBuf, WorkspaceIndex, OverlayManager, PositionConverter) {
    let id = NAV_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temp_dir = std::env::temp_dir().join(format!("ridx_test_nav_{}_{}", std::process::id(), id));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    let code = "pub fn foo() {}\npub struct Bar;\npub trait Baz {}\nimpl Baz for Bar {}\npub fn caller() { foo(); }\nunsafe impl Baz for () {}\nunsafe impl Baz for (u8,) where Self: Sized {}\nunsafe impl<T: Baz> for Option<T> {}\npub fn ext() { ext_fn(); }\npub fn zero_end() {}\n";
    let _ = fs::write(&src_file, code);

    let src_path_str = src_file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("test_nav", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 10,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        file: src_path_str.clone(),
        name: "zero_end".to_string(),
        start_line: 10,
        start_col: 8,
        end_line: 0,
        end_col: 0,
        path: "test_nav::zero_end".to_string(),
        signature: None,
    });
    writer.add_reference(RawReference {
        file: src_path_str.clone(),
        start_line: 10,
        start_col: 8,
        end_line: 10,
        end_col: 16,
        target_def_id: Some(10),
        target_path: "test_nav::zero_end".to_string(),
        target_unit: None,
        target_file: Some(src_path_str.clone()),
        target_line: 10,
        target_col: 8,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.add_reference(RawReference {
        file: src_path_str.clone(),
        start_line: 9,
        start_col: 16,
        end_line: 9,
        end_col: 22,
        target_def_id: None,
        target_path: "ext::ext_fn".to_string(),
        target_unit: None,
        target_file: Some(src_path_str.clone()),
        target_line: 1,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        file: src_path_str.clone(),
        name: "foo".to_string(),
        start_line: 1,
        start_col: 8,
        end_line: 1,
        end_col: 11,
        path: "test_nav::foo".to_string(),
        signature: Some("fn foo()".to_string()),
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        file: src_path_str.clone(),
        name: "Bar".to_string(),
        start_line: 2,
        start_col: 12,
        end_line: 2,
        end_col: 15,
        path: "test_nav::Bar".to_string(),
        signature: None,
    });
    writer.add_definition(RawDefinition {
        def_id: 3,
        kind: DefKind::Trait,
        visibility: 0,
        is_macro_expansion: false,
        file: src_path_str.clone(),
        name: "Baz".to_string(),
        start_line: 3,
        start_col: 11,
        end_line: 3,
        end_col: 14,
        path: "test_nav::Baz".to_string(),
        signature: None,
    });
    writer.add_reference(RawReference {
        file: src_path_str.clone(),
        start_line: 4,
        start_col: 6,
        end_line: 4,
        end_col: 9,
        target_def_id: Some(3),
        target_path: "test_nav::Baz".to_string(),
        target_unit: None,
        target_file: Some(src_path_str.clone()),
        target_line: 3,
        target_col: 11,
        kind: DefKind::Trait,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.add_reference(RawReference {
        file: src_path_str.clone(),
        start_line: 5,
        start_col: 19,
        end_line: 5,
        end_col: 22,
        target_def_id: Some(1),
        target_path: "test_nav::foo".to_string(),
        target_unit: None,
        target_file: Some(src_path_str.clone()),
        target_line: 1,
        target_col: 8,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.add_reference(RawReference {
        file: src_path_str.clone(),
        start_line: 4,
        start_col: 1,
        end_line: 4,
        end_col: 5,
        target_def_id: Some(3),
        target_path: "test_nav::Baz".to_string(),
        target_unit: None,
        target_file: Some(src_path_str.clone()),
        target_line: 3,
        target_col: 11,
        kind: DefKind::Impl,
        is_write: false,
        is_macro_expansion: false,
    });
    for line in [6, 7, 8] {
        writer.add_reference(RawReference {
            file: src_path_str.clone(),
            start_line: line,
            start_col: 13,
            end_line: line,
            end_col: 16,
            target_def_id: Some(3),
            target_path: "test_nav::Baz".to_string(),
            target_unit: None,
            target_file: Some(src_path_str.clone()),
            target_line: 3,
            target_col: 11,
            kind: DefKind::Trait,
            is_write: false,
            is_macro_expansion: false,
        });
    }
    writer.add_reference(RawReference {
        file: "nonexistent_nav_file.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 5,
        target_def_id: Some(3),
        target_path: "test_nav::Baz".to_string(),
        target_unit: None,
        target_file: Some(src_path_str.clone()),
        target_line: 3,
        target_col: 11,
        kind: DefKind::Trait,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.add_type(RawType { file: src_path_str, start_line: 2, start_col: 12, end_line: 2, end_col: 15, type_name: "Bar".to_string() });

    let ridx_file = temp_dir.join("index.ridx");
    let mut f = fs::File::create(&ridx_file).unwrap();
    writer.write_to(&mut f).unwrap();

    let ws = WorkspaceIndex::open(&ridx_file).unwrap();
    let overlay_mgr = OverlayManager::new();
    let conv = PositionConverter::new();

    (temp_dir, ws, overlay_mgr, conv)
}
