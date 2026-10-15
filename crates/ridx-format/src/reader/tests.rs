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
fn test_resolved_definition() {
    let def = ResolvedDefinition {
        name: "test_fn",
        path: "crate::test_fn",
        kind: DefKind::Fn,
        signature: Some("pub fn test_fn()"),
        file: "src/lib.rs",
        start_line: 1,
        start_col: 1,
        end_line: 3,
        end_col: 2,
        is_macro_expansion: false,
        is_reexport: false,
        source_root: Some(Path::new("/workspace")),
        unit_key: Some("unit1"),
        reader_idx: None,
    };

    assert_eq!(def.resolved_path(), Path::new("/workspace/src/lib.rs"));

    let abs_def = ResolvedDefinition {
        file: "/absolute/path/src/lib.rs",
        source_root: Some(Path::new("/other")),
        ..def
    };
    assert_eq!(
        abs_def.resolved_path(),
        Path::new("/absolute/path/src/lib.rs")
    );

    let no_root_def = ResolvedDefinition {
        source_root: None,
        ..def
    };
    assert_eq!(no_root_def.resolved_path(), Path::new("src/lib.rs"));
}

#[test]
fn test_reader_validation_errors() {
    // 1. Buffer too small
    let small_buf = vec![0u8; 10];
    let err = IndexReader::from_buffer(small_buf).unwrap_err();
    assert!(err.to_string().contains("File too small"));

    // 2. Unaligned buffer
    #[repr(align(8))]
    struct AlignedBuf([u8; 1024]);
    let aligned = AlignedBuf([0u8; 1024]);
    let unaligned_slice = &aligned.0[1..512];
    let err = IndexReader::from_buffer(unaligned_slice).unwrap_err();
    assert!(err.to_string().contains("8-byte aligned"));

    // 3. Invalid magic
    let mut bad_magic = vec![0u8; std::mem::size_of::<Header>() + 100];
    bad_magic[0..4].copy_from_slice(b"BAD!");
    let err = IndexReader::from_buffer(bad_magic).unwrap_err();
    assert!(err.to_string().contains("Invalid RIDX magic"));

    // 4. Unsupported version
    let mut bad_version = vec![0u8; std::mem::size_of::<Header>() + 100];
    bad_version[0..4].copy_from_slice(&MAGIC);
    bad_version[4..8].copy_from_slice(&999u32.to_le_bytes());
    let err = IndexReader::from_buffer(bad_version).unwrap_err();
    assert!(err.to_string().contains("Unsupported RIDX version"));
}

#[test]
fn test_check_section_errors() {
    // Unaligned offset
    let err = check_section(1000, 3, 10, 4, 8, "TestSec").unwrap_err();
    assert!(err.to_string().contains("is not aligned to 8 bytes"));

    // Size calculation overflow
    let err = check_section(1000, 8, u32::MAX, usize::MAX / 2, 8, "TestSec").unwrap_err();
    assert!(err.to_string().contains("overflow"));

    // Extends beyond file bounds
    let err = check_section(50, 16, 10, 8, 8, "TestSec").unwrap_err();
    assert!(err.to_string().contains("extends beyond file bounds"));

    // Empty section offset beyond file bounds
    let err = check_section(100, 200, 0, 8, 8, "TestSec").unwrap_err();
    assert!(err.to_string().contains("extends beyond file bounds"));

    // End offset overflow
    let err =
        check_section(1000, ((usize::MAX - 15) & !7) as u64, 2, 20, 8, "TestSec").unwrap_err();
    assert!(err.to_string().contains("overflow"));
}

#[test]
fn test_reader_accessors_out_of_bounds() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_rdr_bounds_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let idx_path = temp_dir.join("test.ridx");

    let mut writer = crate::writer::IndexWriter::new("rdr_crate", "0.1.0");
    writer.add_definition(crate::writer::RawDefinition {
        def_id: 1,
        kind: crate::format::DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "fn1".to_string(),
        path: "rdr_crate::fn1".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 10,
    });
    let mut f = std::fs::File::create(&idx_path).unwrap();
    writer.write_to(&mut f).unwrap();
    drop(f);

    let reader = IndexReader::open(&idx_path).unwrap();

    // 1. get_str with invalid offset
    assert_eq!(reader.get_str(u32::MAX), "");
    assert_eq!(
        reader.get_str((reader.header.string_pool_len as u32).saturating_add(10)),
        ""
    );

    // 2. get_file with invalid file_id
    assert_eq!(reader.get_file(u32::MAX), "");

    // 3. file_source_hash with invalid file_id
    assert_eq!(reader.file_source_hash(u32::MAX), 0);

    // 4. find_definition_by_path nonexistent
    assert!(reader.find_definition_by_path("nonexistent").is_none());

    // 5. find_definition_at outside of file
    assert!(reader.find_definition_at("nonexistent.rs", 1, 1).is_none());

    // Truncated buffer bounds checks (lines 165, 185, 204)
    let dummy_hdr = Header {
        string_pool_offset: 1000,
        string_pool_len: 100,
        files_offset: 2000,
        files_count: 5,
        ..reader.header
    };
    let dummy_reader = IndexReader {
        buffer: vec![0u8; 100],
        header: dummy_hdr,
    };
    assert_eq!(dummy_reader.get_str(5), "");
    assert_eq!(dummy_reader.get_file(0), "");
    assert_eq!(dummy_reader.file_source_hash(0), 0);

    let _ = std::fs::remove_dir_all(&temp_dir);
}
