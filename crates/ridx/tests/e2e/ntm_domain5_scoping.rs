/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Hermetic unit tests for module visibility isolation, `use std::fmt` resolution,
//! and qualifier shadowing against sysroot.

use ridx_format::{DefKind, IndexWriter, RawDefinition, RawReference, WorkspaceIndex};
use std::fs;
use std::io::Cursor;
use tempfile::TempDir;

#[test]
fn test_hermetic_scoping_module_visibility_and_use_std_fmt() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();

    // 1. Build user crate:
    //    src/a.rs defines `mod fmt { pub struct AFmt; }` and `struct Error;`
    //    src/b.rs defines `mod fmt { pub struct BFmt; }` and `struct Error;`
    //    src/c.rs has `use std::fmt;`
    let mut user_writer = IndexWriter::new("user_crate", "0.1.0");
    user_writer.set_unit_key("user_crate-0.1.0");

    user_writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Mod,
        visibility: 0,
        is_macro_expansion: false,
        name: "fmt".to_string(),
        path: "user_crate::a::fmt".to_string(),
        signature: None,
        file: "src/a.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 5,
        end_col: 2,
    });
    user_writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "AFmt".to_string(),
        path: "user_crate::a::fmt::AFmt".to_string(),
        signature: None,
        file: "src/a.rs".to_string(),
        start_line: 2,
        start_col: 5,
        end_line: 2,
        end_col: 20,
    });

    user_writer.add_definition(RawDefinition {
        def_id: 3,
        kind: DefKind::Mod,
        visibility: 0,
        is_macro_expansion: false,
        name: "fmt".to_string(),
        path: "user_crate::b::fmt".to_string(),
        signature: None,
        file: "src/b.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 5,
        end_col: 2,
    });

    // In src/c.rs: `use std::fmt;`
    user_writer.add_definition(RawDefinition {
        def_id: 4,
        kind: DefKind::Use,
        visibility: 0,
        is_macro_expansion: false,
        name: "fmt".to_string(),
        path: "user_crate::c::fmt".to_string(),
        signature: None,
        file: "src/c.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 18,
    });
    user_writer.add_reference(RawReference {
        file: "src/c.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 18,
        target_def_id: Some(50),
        target_path: "core::fmt".to_string(),
        target_unit: Some("core-1.97.1".to_string()),
        target_file: Some("src/fmt/mod.rs".to_string()),
        target_line: 1,
        target_col: 1,
        kind: DefKind::Mod,
        is_write: false,
        is_macro_expansion: false,
    });

    let mut user_buf = Vec::new();
    user_writer.write_to(&mut Cursor::new(&mut user_buf)).unwrap();
    fs::write(root.join("user_crate.ridx"), &user_buf).unwrap();

    // 2. Build core sysroot shard with core::fmt
    let mut core_writer = IndexWriter::new("core", "1.97.1");
    core_writer.set_unit_key("core-1.97.1");
    core_writer.add_definition(RawDefinition {
        def_id: 50,
        kind: DefKind::Mod,
        visibility: 0,
        is_macro_expansion: false,
        name: "fmt".to_string(),
        path: "core::fmt".to_string(),
        signature: None,
        file: "src/fmt/mod.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 50,
        end_col: 2,
    });
    core_writer.add_definition(RawDefinition {
        def_id: 51,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "Formatter".to_string(),
        path: "core::fmt::Formatter".to_string(),
        signature: None,
        file: "src/fmt/mod.rs".to_string(),
        start_line: 10,
        start_col: 1,
        end_line: 15,
        end_col: 2,
    });

    let mut core_buf = Vec::new();
    core_writer.write_to(&mut Cursor::new(&mut core_buf)).unwrap();
    fs::write(root.join("core.ridx"), &core_buf).unwrap();

    let ws = WorkspaceIndex::open(root).unwrap();

    // In src/c.rs with `use std::fmt;`, resolving `fmt` must resolve to `core::fmt`,
    // and NOT to `user_crate::a::fmt` or `user_crate::b::fmt`.
    let c_res = ws.resolve_qualifier("src/c.rs", "fmt");
    assert!(c_res.is_some(), "Must resolve fmt in src/c.rs");
    let c_def = c_res.unwrap();
    assert_eq!(c_def.path, "core::fmt");

    // In src/d.rs without local fmt and without use, resolve_qualifier("src/d.rs", "fmt")
    // must NOT pick up user_crate::a::fmt or user_crate::b::fmt.
    let d_res = ws.resolve_qualifier("src/d.rs", "fmt");
    assert!(d_res.is_none() || d_res.as_ref().map(|d| d.path) == Some("core::fmt"), "Unqualified fmt in unrelated file must never pick up a/b module: got {d_res:?}");
    if let Some(d_def) = d_res {
        assert_ne!(d_def.path, "user_crate::a::fmt");
        assert_ne!(d_def.path, "user_crate::b::fmt");
    }
}

#[test]
fn test_hermetic_scoping_shadowing_and_reverse_sysroot() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();

    let mut user_writer = IndexWriter::new("user_crate", "0.1.0");
    user_writer.set_unit_key("user_crate-0.1.0");

    // Local mod range in src/local.rs
    user_writer.add_definition(RawDefinition {
        def_id: 10,
        kind: DefKind::Mod,
        visibility: 0,
        is_macro_expansion: false,
        name: "range".to_string(),
        path: "user_crate::local::range".to_string(),
        signature: None,
        file: "src/local.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 5,
        end_col: 2,
    });
    user_writer.add_definition(RawDefinition {
        def_id: 11,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "RangeVisitor".to_string(),
        path: "user_crate::local::range::RangeVisitor".to_string(),
        signature: None,
        file: "src/local.rs".to_string(),
        start_line: 2,
        start_col: 5,
        end_line: 2,
        end_col: 29,
    });

    // In src/remote.rs: `use core::ops::range;`
    user_writer.add_definition(RawDefinition {
        def_id: 20,
        kind: DefKind::Use,
        visibility: 0,
        is_macro_expansion: false,
        name: "range".to_string(),
        path: "user_crate::remote::range".to_string(),
        signature: None,
        file: "src/remote.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 25,
    });
    user_writer.add_reference(RawReference {
        file: "src/remote.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 25,
        target_def_id: Some(100),
        target_path: "core::ops::range".to_string(),
        target_unit: Some("core-1.97.1".to_string()),
        target_file: Some("src/ops/range.rs".to_string()),
        target_line: 1,
        target_col: 1,
        kind: DefKind::Mod,
        is_write: false,
        is_macro_expansion: false,
    });

    let mut user_buf = Vec::new();
    user_writer.write_to(&mut Cursor::new(&mut user_buf)).unwrap();
    fs::write(root.join("user_crate.ridx"), &user_buf).unwrap();

    // 2. Build sysroot `core` index with core::ops::range
    let mut core_writer = IndexWriter::new("core", "1.97.1");
    core_writer.set_unit_key("core-1.97.1");
    core_writer.add_definition(RawDefinition {
        def_id: 100,
        kind: DefKind::Mod,
        visibility: 0,
        is_macro_expansion: false,
        name: "range".to_string(),
        path: "core::ops::range".to_string(),
        signature: None,
        file: "src/ops/range.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 20,
        end_col: 2,
    });
    core_writer.add_definition(RawDefinition {
        def_id: 101,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "Range".to_string(),
        path: "core::ops::range::Range".to_string(),
        signature: None,
        file: "src/ops/range.rs".to_string(),
        start_line: 5,
        start_col: 1,
        end_line: 10,
        end_col: 2,
    });

    let mut core_buf = Vec::new();
    core_writer.write_to(&mut Cursor::new(&mut core_buf)).unwrap();
    fs::write(root.join("core.ridx"), &core_buf).unwrap();

    let ws = WorkspaceIndex::open(root).unwrap();

    // Case 1: In src/local.rs, resolving `range` MUST find local `mod range`, NOT sysroot
    let local_res = ws.resolve_qualifier("src/local.rs", "range");
    assert!(local_res.is_some(), "Must resolve range in src/local.rs");
    let local_def = local_res.unwrap();
    assert_eq!(local_def.path, "user_crate::local::range");
    let local_items = ws.find_definitions_by_owner(local_def.path);
    assert_eq!(local_items.len(), 1);
    assert_eq!(local_items[0].name, "RangeVisitor");

    // Case 2: In src/remote.rs, resolving `range` MUST find sysroot `core::ops::range` via `use`
    let remote_res = ws.resolve_qualifier("src/remote.rs", "range");
    assert!(remote_res.is_some(), "Must resolve range in src/remote.rs via use");
    let remote_def = remote_res.unwrap();
    assert_eq!(remote_def.path, "core::ops::range");
    let sysroot_items = ws.find_definitions_by_owner(remote_def.path);
    assert_eq!(sysroot_items.len(), 1);
    assert_eq!(sysroot_items[0].name, "Range");

    // Case 3: In an unassociated file without local range or use, resolve_qualifier returns None,
    // and find_definitions_by_owner("range") does NOT return sysroot items
    let other_res = ws.resolve_qualifier("src/other.rs", "range");
    assert!(other_res.is_none(), "Unqualified range with no local mod or use must not resolve");
    let empty_items = ws.find_definitions_by_owner("range");
    assert!(empty_items.is_empty(), "Sysroot must not match unqualified range: {empty_items:?}");
}
