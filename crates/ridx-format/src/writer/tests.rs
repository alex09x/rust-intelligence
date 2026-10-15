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
use crate::reader::IndexReader;

#[test]
fn test_align_up_and_helpers() {
    assert_eq!(align_up(0, 8), 0);
    assert_eq!(align_up(1, 8), 8);
    assert_eq!(align_up(7, 8), 8);
    assert_eq!(align_up(8, 8), 8);
    assert_eq!(align_up(9, 8), 16);

    let writer = IndexWriter::new("test", "0.1.0");
    assert_eq!(writer.get_interned_str(99999), b"");
}

#[test]
fn test_writer_reader_roundtrip() {
    let mut writer = IndexWriter::new("roundtrip_crate", "1.2.3");
    writer.set_unit_key("unit-xyz");
    writer.set_generation("gen-abc");
    writer.add_file_hash("src/lib.rs", 123456789);

    writer.get_or_insert_file("src/lib.rs");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 1,
        is_macro_expansion: false,
        name: "my_fn".to_string(),
        path: "roundtrip_crate::my_fn".to_string(),
        signature: Some("pub fn my_fn()".to_string()),
        file: "src/lib.rs".to_string(),
        start_line: 10,
        start_col: 1,
        end_line: 12,
        end_col: 2,
    });

    writer.add_reference(RawReference {
        file: "src/lib.rs".to_string(),
        start_line: 20,
        start_col: 5,
        end_line: 20,
        end_col: 10,
        target_def_id: Some(1),
        target_path: "roundtrip_crate::my_fn".to_string(),
        target_file: Some("src/lib.rs".to_string()),
        target_unit: Some("unit-xyz".to_string()),
        target_line: 10,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });

    writer.add_type(RawType {
        file: "src/lib.rs".to_string(),
        start_line: 20,
        start_col: 5,
        end_line: 20,
        end_col: 10,
        type_name: "()".to_string(),
    });

    assert_eq!(writer.unit_key(), Some("unit-xyz"));
    assert_eq!(writer.generation(), Some("gen-abc"));
    writer.get_or_insert_file("extra/file.rs");
    assert!(writer
        .indexed_files()
        .contains(&"extra/file.rs".to_string()));

    // Test macro expansion definition (line 249)
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Macro,
        visibility: 0,
        is_macro_expansion: true,
        name: "macro_def".to_string(),
        path: "roundtrip_crate::macro_def".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 20,
        start_col: 1,
        end_line: 20,
        end_col: 10,
    });

    let mut buf = std::io::Cursor::new(Vec::new());
    writer.write_to(&mut buf).unwrap();

    let reader = IndexReader::from_buffer(buf.into_inner()).unwrap();
    assert_eq!(reader.crate_name(), "roundtrip_crate");
    assert_eq!(reader.crate_version(), "1.2.3");
    assert_eq!(reader.unit_key(), "unit-xyz");
    assert_eq!(reader.generation(), "gen-abc");
    assert_eq!(reader.file_source_hash(0), 123456789);

    let defs = reader.definitions();
    assert_eq!(defs.len(), 2);
    assert_eq!(reader.get_str(defs[0].name_offset()), "my_fn");
    assert!(defs[1].is_macro_expansion());

    let refs = reader.references();
    assert_eq!(refs.len(), 1);

    let types = reader.types();
    assert_eq!(types.len(), 1);
    assert_eq!(reader.get_str(types[0].type_offset()), "()");
}
