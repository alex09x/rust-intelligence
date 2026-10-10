/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

pub mod completion_resolve_tests;
pub mod completion_tests;
pub mod edge_tests;
pub mod extra_suite;
pub mod hover_tests;
pub mod inlay_hint_tests;
pub mod owner_colon_tests;
pub mod owner_dot_tests;
pub mod owner_fallback_tests;
pub mod qualifier_tests;
pub mod signature_help_tests;
pub mod suite;
pub mod trait_scope_tests;

use crate::lsp::protocol::PositionConverter;
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter, RawDefinition, RawType, WorkspaceIndex};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

pub fn setup_test_intelligence() -> (PathBuf, WorkspaceIndex, OverlayManager, PositionConverter) {
    let test_id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_dir = std::env::temp_dir().join(format!(
        "ridx_test_intel_{}_{}",
        std::process::id(),
        test_id
    ));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    let code =
        "pub fn calculate(a: i32, b: i32) -> i32 { a + b }\npub struct User { pub id: u64 }\n";
    let _ = fs::write(&src_file, code);

    let src_path_str = src_file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("test_intel", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        file: src_path_str.clone(),
        name: "calculate".to_string(),
        start_line: 1,
        start_col: 8,
        end_line: 1,
        end_col: 17,
        path: "test_intel::calculate".to_string(),
        signature: Some("fn calculate(a: i32, b: i32) -> i32".to_string()),
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        file: src_path_str.clone(),
        name: "User".to_string(),
        start_line: 2,
        start_col: 12,
        end_line: 2,
        end_col: 16,
        path: "test_intel::User".to_string(),
        signature: None,
    });
    writer.add_definition(RawDefinition {
        def_id: 3,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        file: src_path_str.clone(),
        name: "empty_sig_fn".to_string(),
        start_line: 3,
        start_col: 8,
        end_line: 3,
        end_col: 20,
        path: "test_intel::empty_sig_fn".to_string(),
        signature: None,
    });

    writer.add_type(RawType {
        file: src_path_str.clone(),
        start_line: 1,
        start_col: 8,
        end_line: 1,
        end_col: 11,
        type_name: "fn(i32, i32) -> i32".to_string(),
    });
    writer.add_type(RawType {
        file: src_path_str,
        start_line: 1,
        start_col: 30,
        end_line: 1,
        end_col: 35,
        type_name: "i32".to_string(),
    });

    let ridx_file = temp_dir.join("index.ridx");
    let mut f = fs::File::create(&ridx_file).unwrap();
    writer.write_to(&mut f).unwrap();

    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let overlay_mgr = OverlayManager::new();
    let conv = PositionConverter::new();

    (temp_dir, ws, overlay_mgr, conv)
}
