/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::*;

#[test]
fn test_v3_defs_by_owner_and_derefs_roundtrip() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_v3_roundtrip_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let idx_path = temp_dir.join("test_v3.ridx");

    let mut writer = crate::writer::IndexWriter::new("test_v3_crate", "0.1.0");

    // Add struct definition
    writer.add_definition(crate::writer::RawDefinition {
        def_id: 1,
        kind: crate::format::DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "MyStruct".to_string(),
        path: "test_v3_crate::MyStruct".to_string(),
        file: "src/lib.rs".to_string(),
        start_line: 10,
        start_col: 1,
        end_line: 15,
        end_col: 2,
        signature: None,
    });

    // Add method with canonical owner
    writer.add_definition_with_owners(
        crate::writer::RawDefinition {
            def_id: 2,
            kind: crate::format::DefKind::AssocFn,
            visibility: 0,
            is_macro_expansion: false,
            name: "do_work".to_string(),
            path: "test_v3_crate::MyStruct::do_work".to_string(),
            file: "src/lib.rs".to_string(),
            start_line: 20,
            start_col: 5,
            end_line: 22,
            end_col: 6,
            signature: Some("pub fn do_work(&self)".to_string()),
        },
        vec!["test_v3_crate::MyStruct".to_string()],
    );

    // Add field with canonical owner
    writer.add_definition_with_owners(
        crate::writer::RawDefinition {
            def_id: 3,
            kind: crate::format::DefKind::Field,
            visibility: 0,
            is_macro_expansion: false,
            name: "count".to_string(),
            path: "test_v3_crate::MyStruct::count".to_string(),
            file: "src/lib.rs".to_string(),
            start_line: 11,
            start_col: 5,
            end_line: 11,
            end_col: 15,
            signature: None,
        },
        vec!["test_v3_crate::MyStruct".to_string()],
    );

    // Add type with canonical owner
    writer.add_type_with_owner(
        crate::writer::RawType { file: "src/lib.rs".to_string(), start_line: 30, start_col: 5, end_line: 30, end_col: 15, type_name: "MyStruct".to_string() },
        Some("test_v3_crate::MyStruct".to_string()),
    );

    // Add deref mapping: String -> str
    writer.add_deref("alloc::string::String", "core::primitive::str");

    let mut f = std::fs::File::create(&idx_path).unwrap();
    writer.write_to(&mut f).unwrap();
    drop(f);

    let reader = IndexReader::open(&idx_path).unwrap();
    assert_eq!(reader.header().version, crate::format::CURRENT_VERSION);
    assert!(reader.header().defs_by_owner_count >= 2);
    assert_eq!(reader.header().derefs_count, 1);

    // Verify find_definitions_by_owner
    let defs = reader.find_definitions_by_owner("test_v3_crate::MyStruct");
    assert_eq!(defs.len(), 2);
    let names: Vec<&str> = defs.iter().map(|d| reader.get_str(d.name_offset())).collect();
    assert!(names.contains(&"do_work"));
    assert!(names.contains(&"count"));

    // Verify deref lookup
    assert_eq!(reader.find_deref_target("alloc::string::String"), Some("core::primitive::str"));
    assert_eq!(reader.find_deref_target("core::primitive::str"), None);
    assert_eq!(reader.resolve_deref_chain("alloc::string::String", 3), vec!["core::primitive::str"]);

    // Verify type_entry_at
    let (ty, owner) = reader.type_entry_at("src/lib.rs", 30, 8).unwrap();
    assert_eq!(ty, "MyStruct");
    assert_eq!(owner, Some("test_v3_crate::MyStruct"));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_v3_canonical_owner_collision_protection() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_v3_collision_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let idx_path = temp_dir.join("test_collision.ridx");

    let mut writer = crate::writer::IndexWriter::new("collision_crate", "0.1.0");

    // Two types with same short name "File" in different modules
    writer.add_definition_with_owners(
        crate::writer::RawDefinition {
            def_id: 1,
            kind: crate::format::DefKind::AssocFn,
            visibility: 0,
            is_macro_expansion: false,
            name: "read_a".to_string(),
            path: "collision_crate::module_a::File::read_a".to_string(),
            file: "src/a.rs".to_string(),
            start_line: 10,
            start_col: 5,
            end_line: 12,
            end_col: 6,
            signature: None,
        },
        vec!["collision_crate::module_a::File".to_string()],
    );

    writer.add_definition_with_owners(
        crate::writer::RawDefinition {
            def_id: 2,
            kind: crate::format::DefKind::AssocFn,
            visibility: 0,
            is_macro_expansion: false,
            name: "read_b".to_string(),
            path: "collision_crate::module_b::File::read_b".to_string(),
            file: "src/b.rs".to_string(),
            start_line: 10,
            start_col: 5,
            end_line: 12,
            end_col: 6,
            signature: None,
        },
        vec!["collision_crate::module_b::File".to_string()],
    );

    let mut f = std::fs::File::create(&idx_path).unwrap();
    writer.write_to(&mut f).unwrap();
    drop(f);

    let reader = IndexReader::open(&idx_path).unwrap();

    let a_defs = reader.find_definitions_by_owner("collision_crate::module_a::File");
    assert_eq!(a_defs.len(), 1);
    assert_eq!(reader.get_str(a_defs[0].name_offset()), "read_a");

    let b_defs = reader.find_definitions_by_owner("collision_crate::module_b::File");
    assert_eq!(b_defs.len(), 1);
    assert_eq!(reader.get_str(b_defs[0].name_offset()), "read_b");

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_v3_autoderef_chain_and_cycles() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_v3_deref_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let idx_path = temp_dir.join("test_deref.ridx");

    let mut writer = crate::writer::IndexWriter::new("deref_crate", "0.1.0");

    // Chain: TypeA -> TypeB -> TypeC
    writer.add_deref("pkg::TypeA", "pkg::TypeB");
    writer.add_deref("pkg::TypeB", "pkg::TypeC");

    // Cycle: CycleX -> CycleY -> CycleX
    writer.add_deref("pkg::CycleX", "pkg::CycleY");
    writer.add_deref("pkg::CycleY", "pkg::CycleX");

    let mut f = std::fs::File::create(&idx_path).unwrap();
    writer.write_to(&mut f).unwrap();
    drop(f);

    let reader = IndexReader::open(&idx_path).unwrap();

    let chain = reader.resolve_deref_chain("pkg::TypeA", 5);
    assert_eq!(chain, vec!["pkg::TypeB", "pkg::TypeC"]);

    let chain_limited = reader.resolve_deref_chain("pkg::TypeA", 1);
    assert_eq!(chain_limited, vec!["pkg::TypeB"]);

    // Cycle does not loop infinitely
    let cycle_chain = reader.resolve_deref_chain("pkg::CycleX", 10);
    assert_eq!(cycle_chain, vec!["pkg::CycleY"]);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_v2_backwards_compatibility() {
    // Construct a raw v2 buffer with 128-byte HeaderV2
    let header_v2 = HeaderV2 {
        magic: MAGIC,
        version: 2,
        crate_name_offset: 0,
        crate_version_offset: 0,
        string_pool_offset: 128,
        string_pool_len: 8,
        files_offset: 136,
        files_count: 0,
        unit_key_offset: 0,
        defs_offset: 136,
        defs_count: 0,
        generation_offset: 0,
        defs_by_hash_offset: 136,
        defs_by_hash_count: 0,
        _pad2: 0,
        refs_offset: 136,
        refs_count: 0,
        _pad3: 0,
        refs_by_target_offset: 136,
        refs_by_target_count: 0,
        _pad4: 0,
        types_offset: 136,
        types_count: 0,
        _pad5: 0,
    };

    let mut buf = Vec::new();
    let header_bytes: [u8; 128] = unsafe { std::mem::transmute(header_v2.to_le()) };
    buf.extend_from_slice(&header_bytes);
    buf.extend_from_slice(b"\0crate\0\0"); // 8 bytes string pool at offset 128

    // Pad to 8-byte alignment
    while buf.len() % 8 != 0 {
        buf.push(0);
    }

    let reader = IndexReader::from_buffer(buf).unwrap();
    assert_eq!(reader.header().version, 2);
    assert_eq!(reader.header().defs_by_owner_count, 0);
    assert_eq!(reader.header().derefs_count, 0);
    assert!(reader.defs_by_owner().is_empty());
    assert!(reader.deref_entries().is_empty());
    assert_eq!(reader.find_definitions_by_owner("anything"), Vec::<&DefEntry>::new());
    assert_eq!(reader.find_deref_target("anything"), None);
}

#[test]
fn test_v3_owner_matching_first_interned_string() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_first_string_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let idx_path = temp_dir.join("test_first.ridx");

    let mut writer = crate::writer::IndexWriter::new("first_crate", "0.1.0");
    // "first_crate" is the first non-empty string interned in the pool.
    // Add a type whose owner is "first_crate".
    writer.add_type_with_owner(
        crate::writer::RawType { file: "src/lib.rs".to_string(), start_line: 1, start_col: 1, end_line: 1, end_col: 10, type_name: "RootItem".to_string() },
        Some("first_crate".to_string()),
    );

    let mut f = std::fs::File::create(&idx_path).unwrap();
    writer.write_to(&mut f).unwrap();
    drop(f);

    let reader = IndexReader::open(&idx_path).unwrap();
    let (ty, owner) = reader.type_entry_at("src/lib.rs", 1, 5).unwrap();
    assert_eq!(ty, "RootItem");
    // Must NOT be None (even though first_crate is the first string in pool, offset 0 is reserved for empty string)
    assert_eq!(owner, Some("first_crate"));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_add_definition_unowned_skips_defs_by_owner() {
    let mut writer = crate::writer::IndexWriter::new("unowned_crate", "0.1.0");
    let idx = writer.add_definition_unowned(crate::writer::RawDefinition {
        def_id: 7,
        kind: crate::format::DefKind::AssocTy,
        visibility: 0,
        is_macro_expansion: false,
        name: "Target".to_string(),
        path: "core::ops::deref::Receiver::Target".to_string(),
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 15,
        signature: None,
    });
    assert_eq!(idx, 0);

    let mut buf = Vec::new();
    writer.write_to(&mut std::io::Cursor::new(&mut buf)).unwrap();
    let reader = IndexReader::from_buffer(buf).unwrap();

    assert_eq!(reader.definitions().len(), 1);
    assert!(reader.find_definition_by_path("core::ops::deref::Receiver::Target").is_some());
    assert!(reader.defs_by_owner().is_empty());
    assert!(reader.find_definitions_by_owner("core::ops::deref::Receiver").is_empty());
}
