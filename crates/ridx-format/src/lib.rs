//! # ridx-format
//!
//! Compact, zero-copy binary index format and reader/writer for Rust Intelligence.
//!
//! Provides ultra-low latency (<50 µs) code navigation (go to definition, find references,
//! hover/type-at) using memory-mapped `.ridx` files without persistent background daemon RSS.

pub mod format;
pub mod reader;
pub mod workspace;
pub mod writer;

pub use format::*;
pub use reader::{IndexReader, ResolvedDefinition};
pub use workspace::*;
pub use writer::{IndexWriter, RawDefinition, RawReference, RawType};

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_roundtrip_basic() {
        let mut writer = IndexWriter::new("test_crate", "0.1.0");

        // Add definitions
        writer.add_definition(RawDefinition {
            def_id: 100,
            kind: DefKind::Fn,
            visibility: 0,
            name: "calculate".to_string(),
            path: "test_crate::math::calculate".to_string(),
            signature: Some("fn(i32, i32) -> i32".to_string()),
            file: "src/math.rs".to_string(),
            start_line: 10,
            start_col: 1,
            end_line: 10,
            end_col: 30,
        });

        writer.add_definition(RawDefinition {
            def_id: 101,
            kind: DefKind::Struct,
            visibility: 0,
            name: "Calculator".to_string(),
            path: "test_crate::math::Calculator".to_string(),
            signature: None,
            file: "src/math.rs".to_string(),
            start_line: 5,
            start_col: 1,
            end_line: 8,
            end_col: 2,
        });

        // Add reference (calling calculate from main.rs)
        writer.add_reference(RawReference {
            file: "src/main.rs".to_string(),
            start_line: 15,
            start_col: 18,
            end_line: 15,
            end_col: 27,
            target_def_id: Some(100),
            target_path: "test_crate::math::calculate".to_string(),
            target_file: Some("src/math.rs".to_string()),
            target_line: 10,
            target_col: 1,
            kind: DefKind::Fn,
            is_write: false,
        });

        // Add expression types (outer and inner)
        writer.add_type(RawType {
            file: "src/main.rs".to_string(),
            start_line: 15,
            start_col: 18,
            end_line: 15,
            end_col: 35,
            type_name: "i32".to_string(),
        });

        writer.add_type(RawType {
            file: "src/main.rs".to_string(),
            start_line: 15,
            start_col: 5,
            end_line: 15,
            end_col: 36,
            type_name: "std::result::Result<i32, String>".to_string(),
        });

        let mut buffer = Cursor::new(Vec::new());
        writer.write_to(&mut buffer).expect("write failed");
        let bytes = buffer.into_inner();

        let reader = IndexReader::from_buffer(&bytes).expect("reader parse failed");
        assert_eq!(reader.crate_name(), "test_crate");
        assert_eq!(reader.crate_version(), "0.1.0");

        // Verify definitions lookup by path
        let def = reader
            .find_definition_by_path("test_crate::math::calculate")
            .expect("def not found");
        assert_eq!(reader.get_str(def.name_offset), "calculate");
        assert_eq!(reader.get_str(def.sig_offset), "fn(i32, i32) -> i32");

        // Verify go-to-definition at reference site
        let target = reader
            .find_definition_at("src/main.rs", 15, 20)
            .expect("ref definition not found");
        assert_eq!(target.name, "calculate");
        assert_eq!(target.path, "test_crate::math::calculate");
        assert_eq!(target.file, "src/math.rs");
        assert_eq!(target.start_line, 10);
        assert_eq!(target.signature, Some("fn(i32, i32) -> i32"));

        // Verify find_references
        let refs = reader.find_references("test_crate::math::calculate");
        assert_eq!(refs.len(), 1);
        assert_eq!(reader.get_file(refs[0].span.file_id), "src/main.rs");
        assert_eq!(refs[0].span.start_line, 15);

        // Verify type_at selects innermost type
        let ty_inner = reader.type_at("src/main.rs", 15, 22).expect("type not found");
        assert_eq!(ty_inner, "i32");

        let ty_outer = reader.type_at("src/main.rs", 15, 10).expect("type not found");
        assert_eq!(ty_outer, "std::result::Result<i32, String>");
    }

    #[test]
    fn test_string_deduplication() {
        let mut writer = IndexWriter::new("dedup_crate", "0.1.0");
        let off1 = writer.intern_string("std::option::Option<T>");
        let off2 = writer.intern_string("std::option::Option<T>");
        let off3 = writer.intern_string("std::option::Option<T>");
        assert_eq!(off1, off2);
        assert_eq!(off2, off3);
    }

    #[test]
    fn test_structural_validation_guards() {
        // 1. Buffer too small
        let small_buf = vec![0u8; 64];
        assert!(IndexReader::from_buffer(&small_buf).is_err());

        // 2. Invalid magic
        let mut bad_magic = vec![0u8; 256];
        bad_magic[0..4].copy_from_slice(b"NOPE");
        assert!(IndexReader::from_buffer(&bad_magic).is_err());

        // 3. Out-of-bounds table offset in valid header
        let mut writer = IndexWriter::new("test", "0.1.0");
        let mut buf = std::io::Cursor::new(Vec::new());
        writer.write_to(&mut buf).unwrap();
        let mut bytes = buf.into_inner();

        // Corrupt defs_offset to point way beyond buffer
        let bad_offset = (bytes.len() as u64) + 10000;
        // defs_offset is at byte 32 of Header
        bytes[32..40].copy_from_slice(&bad_offset.to_le_bytes());
        assert!(IndexReader::from_buffer(&bytes).is_err());
    }
}
