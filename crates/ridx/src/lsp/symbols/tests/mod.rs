/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

pub mod document_symbols_tests;
pub mod workspace_symbol_resolve_tests;
pub mod workspace_symbols_tests;

use crate::lsp::protocol::PositionConverter;
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter, RawDefinition, WorkspaceIndex};
use std::fs;
use std::path::PathBuf;

static SYM_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub struct SymbolsTestHarness {
    pub temp_dir: PathBuf,
    pub ws: WorkspaceIndex,
    pub overlay_mgr: OverlayManager,
    pub conv: PositionConverter,
    pub src_file: PathBuf,
    pub uri: String,
}

impl Drop for SymbolsTestHarness {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.temp_dir);
    }
}

pub fn setup_test_symbols() -> SymbolsTestHarness {
    let id = SYM_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temp_dir = std::env::temp_dir().join(format!("ridx_test_syms_{}_{}", std::process::id(), id));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    let code =
        "pub fn my_symbol() {}\npub struct MyStruct;\npub enum MyEnum { VariantA, VariantB }\npub trait MyTrait { fn trait_method(&self); }\npub const MY_CONST: usize = 42;\n";
    let _ = fs::write(&src_file, code);

    let mut writer = IndexWriter::new("test_syms", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        file: "src/lib.rs".to_string(),
        name: "my_symbol".to_string(),
        start_line: 1,
        start_col: 8,
        end_line: 1,
        end_col: 17,
        path: "test_syms::my_symbol".to_string(),
        signature: Some("fn my_symbol()".to_string()),
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        file: "src/lib.rs".to_string(),
        name: "MyStruct".to_string(),
        start_line: 2,
        start_col: 12,
        end_line: 2,
        end_col: 20,
        path: "test_syms::MyStruct".to_string(),
        signature: None,
    });
    writer.add_definition(RawDefinition {
        def_id: 3,
        kind: DefKind::Enum,
        visibility: 0,
        is_macro_expansion: false,
        file: "src/lib.rs".to_string(),
        name: "MyEnum".to_string(),
        start_line: 3,
        start_col: 10,
        end_line: 3,
        end_col: 16,
        path: "test_syms::MyEnum".to_string(),
        signature: None,
    });
    writer.add_definition(RawDefinition {
        def_id: 4,
        kind: DefKind::Trait,
        visibility: 0,
        is_macro_expansion: false,
        file: "src/lib.rs".to_string(),
        name: "MyTrait".to_string(),
        start_line: 4,
        start_col: 11,
        end_line: 4,
        end_col: 18,
        path: "test_syms::MyTrait".to_string(),
        signature: None,
    });
    writer.add_definition(RawDefinition {
        def_id: 5,
        kind: DefKind::Const,
        visibility: 0,
        is_macro_expansion: false,
        file: "src/lib.rs".to_string(),
        name: "MY_CONST".to_string(),
        start_line: 5,
        start_col: 11,
        end_line: 5,
        end_col: 19,
        path: "test_syms::MY_CONST".to_string(),
        signature: Some("const MY_CONST: usize = 42;".to_string()),
    });

    for i in 10..140 {
        writer.add_definition(RawDefinition {
            def_id: i,
            kind: DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            file: "src/lib.rs".to_string(),
            name: format!("sym_{i}"),
            start_line: i,
            start_col: 1,
            end_line: i,
            end_col: 5,
            path: format!("test_syms::batch::sym_{i}"),
            signature: None,
        });
    }

    let ridx_file = temp_dir.join("index.ridx");
    let mut f = fs::File::create(&ridx_file).unwrap();
    writer.write_to(&mut f).unwrap();

    let ws = WorkspaceIndex::open(&ridx_file).unwrap();
    let overlay_mgr = OverlayManager::new();
    let conv = PositionConverter::new();
    let uri = format!("file://{}", src_file.to_str().unwrap());

    SymbolsTestHarness { temp_dir, ws, overlay_mgr, conv, src_file, uri }
}
