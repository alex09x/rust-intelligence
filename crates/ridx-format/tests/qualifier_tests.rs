/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for scoped qualifier resolution in `WorkspaceIndex`.

#[path = "qualifier_tests/sysroot_and_reexports.rs"]
mod sysroot_and_reexports;

use ridx_format::{DefKind, IndexWriter, RawDefinition, RawReference, WorkspaceIndex};
use std::fs;
use std::io::Cursor;
use tempfile::TempDir;

#[test]
fn test_resolve_qualifier_file_scoping() {
    let temp_dir = TempDir::new().unwrap();
    let target_dir = temp_dir.path();

    let mut writer = IndexWriter::new("test_crate", "0.1.0");
    writer.set_unit_key("test_crate-0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Mod,
        visibility: 0,
        is_macro_expansion: false,
        name: "range".to_string(),
        path: "test_crate::local::range".to_string(),
        signature: None,
        file: "src/local.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 5,
        end_col: 2,
    });

    // Add unrelated item in another module
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Mod,
        visibility: 0,
        is_macro_expansion: false,
        name: "fmt".to_string(),
        path: "test_crate::unrelated::fmt".to_string(),
        signature: None,
        file: "src/unrelated.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 5,
        end_col: 2,
    });

    // Add parent module item
    writer.add_definition(RawDefinition {
        def_id: 3,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "Error".to_string(),
        path: "test_crate::de::Error".to_string(),
        signature: None,
        file: "src/de/mod.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 5,
        end_col: 2,
    });

    let mut buf = Vec::new();
    writer.write_to(&mut Cursor::new(&mut buf)).unwrap();
    fs::write(target_dir.join("test_crate.ridx"), buf).unwrap();

    let ws = WorkspaceIndex::open(target_dir).unwrap();

    // 1. Local mod range in src/local.rs resolves to test_crate::local::range
    let res = ws.resolve_qualifier("src/local.rs", "range");
    assert!(res.is_some());
    assert_eq!(res.unwrap().path, "test_crate::local::range");

    // 2. Query in another file does not pick up test_crate::local::range
    let none_res = ws.resolve_qualifier("src/other.rs", "range");
    assert!(none_res.is_none());

    // 3. Query in src/de/impls.rs with super::Error resolves to test_crate::de::Error
    let super_res = ws.resolve_qualifier("src/de/impls.rs", "super::Error");
    assert!(super_res.is_some());
    assert_eq!(super_res.unwrap().path, "test_crate::de::Error");

    // 4. Query in src/de/impls.rs with fmt does NOT pick up test_crate::unrelated::fmt
    let fmt_res = ws.resolve_qualifier("src/de/impls.rs", "fmt");
    assert!(fmt_res.is_none());
}

#[test]
fn test_resolve_qualifier_glob_import() {
    let temp_dir = TempDir::new().unwrap();
    let target_dir = temp_dir.path();

    // 1. Build sysroot `core` with core::fmt and core::fmt::Display
    let mut core_writer = IndexWriter::new("core", "1.97.1");
    core_writer.set_unit_key("core-1.97.1");
    core_writer.add_definition(RawDefinition {
        def_id: 100,
        kind: DefKind::Mod,
        visibility: 0,
        is_macro_expansion: false,
        name: "fmt".to_string(),
        path: "core::fmt".to_string(),
        signature: None,
        file: "library/core/src/fmt/mod.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 100,
        end_col: 2,
    });
    core_writer.add_definition(RawDefinition {
        def_id: 101,
        kind: DefKind::Trait,
        visibility: 0,
        is_macro_expansion: false,
        name: "Display".to_string(),
        path: "core::fmt::Display".to_string(),
        signature: None,
        file: "library/core/src/fmt/mod.rs".to_string(),
        start_line: 10,
        start_col: 1,
        end_line: 20,
        end_col: 2,
    });
    let mut core_buf = Vec::new();
    core_writer
        .write_to(&mut Cursor::new(&mut core_buf))
        .unwrap();
    fs::write(target_dir.join("core.ridx"), core_buf).unwrap();

    // 2. Build user crate:
    // - module `lib` in src/lib.rs has `pub use core::fmt;`
    // - file src/worker.rs has `use crate::lib::*;`
    // - file src/de/impls.rs has `use super::*;` where src/de/mod.rs has struct Error
    let mut writer = IndexWriter::new("test_crate", "0.1.0");
    writer.set_unit_key("test_crate-0.1.0");

    // In src/lib.rs: module `lib` has `pub use core::fmt;`
    writer.add_definition(RawDefinition {
        def_id: 10,
        kind: DefKind::Use,
        visibility: 0,
        is_macro_expansion: false,
        name: "fmt".to_string(),
        path: "test_crate::lib::fmt".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 9,
        end_line: 1,
        end_col: 18,
    });
    writer.add_reference(RawReference {
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 9,
        end_line: 1,
        end_col: 18,
        target_def_id: Some(100),
        target_path: "core::fmt".to_string(),
        target_unit: Some("core-1.97.1".to_string()),
        target_file: Some("library/core/src/fmt/mod.rs".to_string()),
        target_line: 1,
        target_col: 1,
        kind: DefKind::Mod,
        is_write: false,
        is_macro_expansion: false,
    });

    // In src/worker.rs: `use crate::lib::*;`
    writer.add_definition(RawDefinition {
        def_id: 20,
        kind: DefKind::Use,
        visibility: 0,
        is_macro_expansion: false,
        name: "*".to_string(),
        path: "test_crate::worker::*".to_string(),
        signature: Some("test_crate::lib".to_string()),
        file: "src/worker.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 20,
    });
    writer.add_reference(RawReference {
        file: "src/worker.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 20,
        target_def_id: Some(10),
        target_path: "test_crate::lib".to_string(),
        target_unit: Some("test_crate-0.1.0".to_string()),
        target_file: Some("src/lib.rs".to_string()),
        target_line: 1,
        target_col: 1,
        kind: DefKind::Mod,
        is_write: false,
        is_macro_expansion: false,
    });

    // In src/de/mod.rs: struct Error
    writer.add_definition(RawDefinition {
        def_id: 30,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "Error".to_string(),
        path: "test_crate::de::Error".to_string(),
        signature: None,
        file: "src/de/mod.rs".to_string(),
        start_line: 5,
        start_col: 1,
        end_line: 8,
        end_col: 2,
    });

    // In src/de/impls.rs: `use super::*;`
    writer.add_definition(RawDefinition {
        def_id: 40,
        kind: DefKind::Use,
        visibility: 0,
        is_macro_expansion: false,
        name: "*".to_string(),
        path: "test_crate::de::impls::*".to_string(),
        signature: Some("super".to_string()),
        file: "src/de/impls.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 14,
    });

    let mut buf = Vec::new();
    writer.write_to(&mut Cursor::new(&mut buf)).unwrap();
    fs::write(target_dir.join("test_crate.ridx"), buf).unwrap();

    let ws = WorkspaceIndex::open(target_dir).unwrap();

    // 1. Query in src/worker.rs with "fmt" -> resolves via glob import to core::fmt
    let res = ws.resolve_qualifier("src/worker.rs", "fmt");
    assert!(res.is_some(), "fmt should resolve via use crate::lib::*;");
    assert_eq!(res.unwrap().path, "core::fmt");

    // 2. Query in src/worker.rs with "fmt::Display" -> resolves to core::fmt::Display
    let res2 = ws.resolve_qualifier("src/worker.rs", "fmt::Display");
    assert!(
        res2.is_some(),
        "fmt::Display should resolve via glob import"
    );
    assert_eq!(res2.unwrap().path, "core::fmt::Display");

    // 3. Query in src/de/impls.rs with "Error" (no super:: prefix) -> resolves via use super::*; to test_crate::de::Error
    let res3 = ws.resolve_qualifier("src/de/impls.rs", "Error");
    assert!(res3.is_some(), "Error should resolve via use super::*;");
    assert_eq!(res3.unwrap().path, "test_crate::de::Error");
}
