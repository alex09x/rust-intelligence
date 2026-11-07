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
fn test_reader_overlapping_spans_and_hash_collisions() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_rdr_overlap_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let idx_path = temp_dir.join("overlap.ridx");

    let mut writer = crate::writer::IndexWriter::new("overlap_crate", "0.1.0");
    // Outer definition
    writer.add_definition(crate::writer::RawDefinition {
        def_id: 1,
        kind: crate::format::DefKind::Mod,
        visibility: 0,
        is_macro_expansion: false,
        name: "outer_mod".to_string(),
        path: "overlap_crate::outer_mod".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 50,
        end_col: 1,
    });
    // Inner definition (smaller span)
    writer.add_definition(crate::writer::RawDefinition {
        def_id: 2,
        kind: crate::format::DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "inner_fn".to_string(),
        path: "overlap_crate::outer_mod::inner_fn".to_string(),
        signature: Some("fn() -> i32".to_string()),
        file: "src/lib.rs".to_string(),
        start_line: 10,
        start_col: 5,
        end_line: 15,
        end_col: 6,
    });

    // Overlapping references: outer ref and inner ref
    writer.add_reference(crate::writer::RawReference {
        file: "src/lib.rs".to_string(),
        start_line: 20,
        start_col: 1,
        end_line: 20,
        end_col: 30,
        target_def_id: Some(1),
        target_path: "overlap_crate::outer_mod".to_string(),
        target_unit: None,
        target_file: Some("src/lib.rs".to_string()),
        target_line: 1,
        target_col: 1,
        kind: crate::format::DefKind::Mod,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.add_reference(crate::writer::RawReference {
        file: "src/lib.rs".to_string(),
        start_line: 20,
        start_col: 10,
        end_line: 20,
        end_col: 20,
        target_def_id: Some(2),
        target_path: "overlap_crate::outer_mod::inner_fn".to_string(),
        target_unit: None,
        target_file: Some("src/lib.rs".to_string()),
        target_line: 10,
        target_col: 5,
        kind: crate::format::DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    // External reference (target_def_id == u32::MAX)
    writer.add_reference(crate::writer::RawReference {
        file: "src/lib.rs".to_string(),
        start_line: 30,
        start_col: 5,
        end_line: 30,
        end_col: 15,
        target_def_id: None,
        target_path: "ext_crate::ext_fn".to_string(),
        target_unit: Some("ext_crate:1.0.0".to_string()),
        target_file: Some("ext.rs".to_string()),
        target_line: 5,
        target_col: 1,
        kind: crate::format::DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });

    // Overlapping types
    writer.add_type(crate::writer::RawType { file: "src/lib.rs".to_string(), start_line: 25, start_col: 1, end_line: 25, end_col: 30, type_name: "OuterType".to_string() });
    writer.add_type(crate::writer::RawType { file: "src/lib.rs".to_string(), start_line: 25, start_col: 5, end_line: 25, end_col: 15, type_name: "InnerType".to_string() });

    // Missing def_id target in reference (line 386)
    writer.add_reference(crate::writer::RawReference {
        file: "src/lib.rs".to_string(),
        start_line: 40,
        start_col: 1,
        end_line: 40,
        end_col: 10,
        target_def_id: Some(9999), // does not exist in defs!
        target_path: "unresolved::missing".to_string(),
        target_unit: None,
        target_file: Some("src/missing.rs".to_string()),
        target_line: 1,
        target_col: 1,
        kind: crate::format::DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });

    // Add inner items FIRST then outer items SECOND on src/other.rs to hit `_ => {}` (lines 357, 427, 493)
    // 1. Definition: inner first, outer second
    writer.add_definition(crate::writer::RawDefinition {
        def_id: 3,
        kind: crate::format::DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "inner_def".to_string(),
        path: "overlap_crate::inner_def".to_string(),
        signature: None,
        file: "src/other.rs".to_string(),
        start_line: 5,
        start_col: 5,
        end_line: 5,
        end_col: 10,
    });
    writer.add_definition(crate::writer::RawDefinition {
        def_id: 4,
        kind: crate::format::DefKind::Mod,
        visibility: 0,
        is_macro_expansion: false,
        name: "outer_mod2".to_string(),
        path: "overlap_crate::outer_mod2".to_string(),
        signature: None,
        file: "src/other.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 10,
        end_col: 1,
    });

    // 2. Reference: inner first, outer second
    writer.add_reference(crate::writer::RawReference {
        file: "src/other.rs".to_string(),
        start_line: 15,
        start_col: 5,
        end_line: 15,
        end_col: 10,
        target_def_id: Some(3),
        target_path: "overlap_crate::inner_def".to_string(),
        target_unit: None,
        target_file: Some("src/other.rs".to_string()),
        target_line: 5,
        target_col: 5,
        kind: crate::format::DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer.add_reference(crate::writer::RawReference {
        file: "src/other.rs".to_string(),
        start_line: 15,
        start_col: 1,
        end_line: 15,
        end_col: 20,
        target_def_id: Some(4),
        target_path: "overlap_crate::outer_mod2".to_string(),
        target_unit: None,
        target_file: Some("src/other.rs".to_string()),
        target_line: 1,
        target_col: 1,
        kind: crate::format::DefKind::Mod,
        is_write: false,
        is_macro_expansion: false,
    });

    // 3. Type: inner first, outer second
    writer.add_type(crate::writer::RawType { file: "src/other.rs".to_string(), start_line: 20, start_col: 5, end_line: 20, end_col: 10, type_name: "InnerOther".to_string() });
    writer.add_type(crate::writer::RawType { file: "src/other.rs".to_string(), start_line: 20, start_col: 1, end_line: 20, end_col: 20, type_name: "OuterOther".to_string() });

    let mut f = std::fs::File::create(&idx_path).unwrap();
    writer.write_to(&mut f).unwrap();
    drop(f);

    let reader = IndexReader::open(&idx_path).unwrap();

    // 1. Definition cursor picking innermost def
    let def = reader.find_definition_at("src/lib.rs", 12, 6).unwrap();
    assert_eq!(def.name, "inner_fn");
    assert_eq!(def.signature, Some("fn() -> i32"));

    let def_outer = reader.find_definition_at("src/lib.rs", 2, 1).unwrap();
    assert_eq!(def_outer.name, "outer_mod");
    assert_eq!(def_outer.signature, None);

    // 2. Reference cursor picking innermost ref with local def_id
    let ref_inner = reader.find_definition_at("src/lib.rs", 20, 15).unwrap();
    assert_eq!(ref_inner.name, "inner_fn");

    // Reference cursor with external def
    let ref_ext = reader.find_definition_at("src/lib.rs", 30, 8).unwrap();
    assert_eq!(ref_ext.path, "ext_crate::ext_fn");
    assert_eq!(ref_ext.file, "ext.rs");

    // Reference cursor with nonexistent target_def_id (line 386)
    let ref_unresolved = reader.find_definition_at("src/lib.rs", 40, 5).unwrap();
    assert_eq!(ref_unresolved.path, "unresolved::missing");

    // 3. Type at cursor picking innermost type
    assert_eq!(reader.type_at("src/lib.rs", 25, 10), Some("InnerType"));
    assert_eq!(reader.type_at("src/lib.rs", 25, 2), Some("OuterType"));
    assert_eq!(reader.type_at("src/lib.rs", 99, 99), None);
    assert_eq!(reader.type_at("missing.rs", 1, 1), None);

    // 4. Reverse ordering tests (lines 357, 427, 493)
    assert_eq!(reader.find_definition_at("src/other.rs", 5, 6).unwrap().name, "inner_def");
    assert_eq!(reader.find_definition_at("src/other.rs", 15, 6).unwrap().name, "inner_def");
    assert_eq!(reader.type_at("src/other.rs", 20, 6), Some("InnerOther"));

    // 5. find_references
    let refs = reader.find_references("overlap_crate::outer_mod::inner_fn");
    assert_eq!(refs.len(), 1);
    let missing_refs = reader.find_references("missing::path");
    assert!(missing_refs.is_empty());

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_validate_sections_comprehensive() {
    // String pool offset not aligned
    let mut hdr = Header {
        magic: crate::format::MAGIC,
        version: crate::format::CURRENT_FORMAT_VERSION,
        string_pool_offset: 7, // not aligned to 8
        string_pool_len: 16,
        files_offset: 16,
        defs_offset: 16,
        defs_by_hash_offset: 16,
        refs_offset: 16,
        refs_by_target_offset: 16,
        types_offset: 16,
        defs_by_owner_offset: 16,
        derefs_offset: 16,
        notes_offset: 16,
        ty_defs_offset: 16,
        ty_entries_offset: 16,
        ty_bounds_offset: 16,
        ty_args_offset: 16,
        fn_sigs_offset: 16,
        impls_offset: 16,
        assoc_tys_offset: 16,
        ..Header::default()
    };
    assert!(validate_sections(1000, &hdr).is_err());

    // String pool length overflow
    hdr.string_pool_offset = 16;
    hdr.string_pool_len = u64::MAX;
    assert!(validate_sections(1000, &hdr).is_err());

    // String pool beyond file bounds
    hdr.string_pool_len = 2000;
    assert!(validate_sections(1000, &hdr).is_err());

    // Crate name offset beyond pool
    hdr.string_pool_len = 100;
    hdr.crate_name_offset = 200;
    assert!(validate_sections(1000, &hdr).is_err());

    hdr.crate_name_offset = 0;
    hdr.crate_version_offset = 200;
    assert!(validate_sections(1000, &hdr).is_err());

    // Test section error paths (lines 563, 573, 583, 593, 603)
    hdr.crate_version_offset = 0;
    hdr.defs_offset = 3; // unaligned
    assert!(validate_sections(1000, &hdr).is_err());
    hdr.defs_offset = 16;
    hdr.defs_by_hash_offset = 3;
    assert!(validate_sections(1000, &hdr).is_err());
    hdr.defs_by_hash_offset = 16;
    hdr.refs_offset = 3;
    assert!(validate_sections(1000, &hdr).is_err());
    hdr.refs_offset = 16;
    hdr.refs_by_target_offset = 3;
    assert!(validate_sections(1000, &hdr).is_err());
    hdr.refs_by_target_offset = 16;
    hdr.types_offset = 3;
    assert!(validate_sections(1000, &hdr).is_err());

    // Open non-existent file
    assert!(IndexReader::open("/nonexistent/ridx/file.ridx").is_err());
}
