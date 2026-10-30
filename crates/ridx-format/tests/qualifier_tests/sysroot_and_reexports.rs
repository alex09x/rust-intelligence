/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use ridx_format::{DefKind, IndexWriter, RawDefinition, WorkspaceIndex};
use std::fs;
use std::io::Cursor;
use tempfile::TempDir;

#[test]
fn test_resolve_qualifier_sysroot_not_shadowed_by_use() {
    let temp_dir = TempDir::new().unwrap();
    let target_dir = temp_dir.path();

    let mut writer = IndexWriter::new("test_crate", "0.1.0");
    writer.set_unit_key("test_crate-0.1.0");

    // In src/system_registry.rs: `use core::cell::Cell;`
    // Indexed as DefKind::Use with name "core" and path "test_crate::system::system_registry::core"
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Use,
        visibility: 0,
        is_macro_expansion: false,
        name: "core".to_string(),
        path: "test_crate::system::system_registry::core".to_string(),
        signature: None,
        file: "src/system_registry.rs".to_string(),
        start_line: 10,
        start_col: 9,
        end_line: 10,
        end_col: 13,
    });

    let mut buf = Vec::new();
    writer.write_to(&mut Cursor::new(&mut buf)).unwrap();
    fs::write(target_dir.join("test_crate.ridx"), buf).unwrap();

    let ws = WorkspaceIndex::open(target_dir).unwrap();

    // "core" qualifier must NOT resolve to the local Use entry
    let res = ws.resolve_qualifier("src/system_registry.rs", "core");
    assert!(res.is_none() || res.as_ref().unwrap().path != "test_crate::system::system_registry::core", "sysroot qualifier 'core' must not be shadowed by local Use entry");
}

#[test]
fn test_sysroot_reexport_alias_and_owner_completion() {
    let temp_dir = TempDir::new().unwrap();
    let target_dir = temp_dir.path();

    // 1. Build core.ridx with canonical SocketAddr and inherent methods
    let mut core_writer = IndexWriter::new("core", "1.98.0");
    core_writer.add_definition_with_owners(
        RawDefinition {
            def_id: 1,
            kind: DefKind::Struct,
            visibility: 0,
            is_macro_expansion: false,
            name: "SocketAddr".to_string(),
            path: "core::net::socket_addr::SocketAddr".to_string(),
            signature: None,
            file: "library/core/src/net/socket_addr.rs".to_string(),
            start_line: 10,
            start_col: 1,
            end_line: 20,
            end_col: 2,
        },
        vec!["core::net::socket_addr".to_string()],
    );
    core_writer.add_definition_with_owners(
        RawDefinition {
            def_id: 2,
            kind: DefKind::AssocFn,
            visibility: 0,
            is_macro_expansion: false,
            name: "is_ipv4".to_string(),
            path: "core::net::socket_addr::SocketAddr::is_ipv4".to_string(),
            signature: Some("fn(&self) -> bool".to_string()),
            file: "library/core/src/net/socket_addr.rs".to_string(),
            start_line: 25,
            start_col: 5,
            end_line: 27,
            end_col: 6,
        },
        vec!["core::net::socket_addr::SocketAddr".to_string()],
    );
    let mut core_buf = Vec::new();
    core_writer.write_to(&mut Cursor::new(&mut core_buf)).unwrap();
    fs::write(target_dir.join("core.ridx"), core_buf).unwrap();

    // 2. Build std.ridx with native TcpStream and re-exported SocketAddr alias
    let mut std_writer = IndexWriter::new("std", "1.98.0");
    std_writer.add_definition_with_owners(
        RawDefinition {
            def_id: 1,
            kind: DefKind::Struct,
            visibility: 0,
            is_macro_expansion: false,
            name: "TcpStream".to_string(),
            path: "std::net::tcp::TcpStream".to_string(),
            signature: None,
            file: "library/std/src/net/tcp.rs".to_string(),
            start_line: 10,
            start_col: 1,
            end_line: 20,
            end_col: 2,
        },
        vec!["std::net".to_string()],
    );
    // Re-export alias: std::net::SocketAddr -> core::net::socket_addr::SocketAddr
    std_writer.add_reexport_with_owners(
        RawDefinition {
            def_id: 2,
            kind: DefKind::Struct,
            visibility: 0,
            is_macro_expansion: false,
            name: "SocketAddr".to_string(),
            path: "std::net::SocketAddr".to_string(),
            signature: Some("core::net::socket_addr::SocketAddr".to_string()),
            file: "library/core/src/net/socket_addr.rs".to_string(),
            start_line: 10,
            start_col: 1,
            end_line: 20,
            end_col: 2,
        },
        vec!["std::net".to_string()],
    );
    let mut std_buf = Vec::new();
    std_writer.write_to(&mut Cursor::new(&mut std_buf)).unwrap();
    fs::write(target_dir.join("std.ridx"), std_buf).unwrap();

    let ws = WorkspaceIndex::open(target_dir).unwrap();

    // Verify find_definitions_by_owner("std::net") returns both items without duplicates
    let defs = ws.find_definitions_by_owner("std::net");
    assert_eq!(defs.len(), 2, "std::net should contain exactly TcpStream and SocketAddr");
    let names: Vec<&str> = defs.iter().map(|d| d.name).collect();
    assert!(names.contains(&"TcpStream"));
    assert!(names.contains(&"SocketAddr"));

    // Verify alias resolves via path lookup
    let socket_def = ws.find_definition_by_path("std::net::SocketAddr");
    assert!(socket_def.is_some());
    let socket_def = socket_def.unwrap();
    assert!(socket_def.is_reexport, "SocketAddr alias must be marked as is_reexport");
    assert_eq!(socket_def.signature, Some("core::net::socket_addr::SocketAddr"));

    // Verify canonical methods are found under the canonical owner
    let target_owner = socket_def.signature.unwrap();
    let methods = ws.find_definitions_by_owner(target_owner);
    assert_eq!(methods.len(), 1);
    assert_eq!(methods[0].name, "is_ipv4");
}

#[test]
fn test_type_alias_not_treated_as_reexport() {
    let temp_dir = TempDir::new().unwrap();
    let target_dir = temp_dir.path();

    let mut writer = IndexWriter::new("my_crate", "0.1.0");
    writer.add_definition_with_owners(
        RawDefinition {
            def_id: 1,
            kind: DefKind::TyAlias,
            visibility: 0,
            is_macro_expansion: false,
            name: "Foo".to_string(),
            path: "my_crate::Foo".to_string(),
            signature: Some("core::cell::Cell<u8>".to_string()),
            file: "src/lib.rs".to_string(),
            start_line: 5,
            start_col: 1,
            end_line: 5,
            end_col: 36,
        },
        vec!["my_crate".to_string()],
    );
    let mut buf = Vec::new();
    writer.write_to(&mut Cursor::new(&mut buf)).unwrap();
    fs::write(target_dir.join("my_crate.ridx"), buf).unwrap();

    let ws = WorkspaceIndex::open(target_dir).unwrap();
    let foo_def = ws.find_definition_by_path("my_crate::Foo").expect("Foo should exist");
    assert!(!foo_def.is_reexport, "Type alias must NOT have is_reexport set");
    assert_eq!(foo_def.signature, Some("core::cell::Cell<u8>"));
}
