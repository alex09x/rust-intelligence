use ridx_format::{DefKind, IndexReader, IndexWriter, RawDefinition, RawReference, RawType};
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::PathBuf;

#[test]
fn test_cli_fixture_generation_and_query() {
    let out_dir = PathBuf::from("target/test_indices");
    fs::create_dir_all(&out_dir).unwrap();
    let index_path = out_dir.join("sample_crate.ridx");

    let mut writer = IndexWriter::new("sample_crate", "1.0.0");

    // Add function definition
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "process_data".to_string(),
        path: "sample_crate::processor::process_data".to_string(),
        signature: Some("fn(&[u8]) -> Result<usize, String>".to_string()),
        file: "src/processor.rs".to_string(),
        start_line: 25,
        start_col: 1,
        end_line: 25,
        end_col: 48,
    });

    // Add struct definition
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "DataProcessor".to_string(),
        path: "sample_crate::processor::DataProcessor".to_string(),
        signature: None,
        file: "src/processor.rs".to_string(),
        start_line: 10,
        start_col: 1,
        end_line: 14,
        end_col: 2,
    });

    // Add reference from main.rs calling process_data
    writer.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 42,
        start_col: 9,
        end_line: 42,
        end_col: 21,
        target_def_id: Some(1),
        target_path: "sample_crate::processor::process_data".to_string(),
        target_unit: None,
        target_file: Some("src/processor.rs".to_string()),
        target_line: 25,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });

    // Add expression type for the call
    writer.add_type(RawType {
        file: "src/main.rs".to_string(),
        start_line: 42,
        start_col: 9,
        end_line: 42,
        end_col: 30,
        type_name: "Result<usize, String>".to_string(),
    });

    let f = File::create(&index_path).unwrap();
    let mut buf = BufWriter::new(f);
    writer.write_to(&mut buf).unwrap();

    // Verify opening and querying via IndexReader (Mmap)
    let reader = IndexReader::open(&index_path).unwrap();
    assert_eq!(reader.crate_name(), "sample_crate");
    assert_eq!(reader.header().defs_count, 2);
    assert_eq!(reader.header().refs_count, 1);
    assert_eq!(reader.header().types_count, 1);

    // Def query by symbol
    let def = reader
        .find_definition_by_path("sample_crate::processor::process_data")
        .unwrap();
    assert_eq!(reader.get_str(def.name_offset()), "process_data");
    assert_eq!(
        reader.get_str(def.sig_offset()),
        "fn(&[u8]) -> Result<usize, String>"
    );

    // Def query at cursor position
    let target = reader.find_definition_at("src/main.rs", 42, 12).unwrap();
    assert_eq!(target.name, "process_data");
    assert_eq!(target.file, "src/processor.rs");
    assert_eq!(target.start_line, 25);

    // Refs query
    let refs = reader.find_references("sample_crate::processor::process_data");
    assert_eq!(refs.len(), 1);
    assert_eq!(reader.get_file(refs[0].span().file_id()), "src/main.rs");
    assert_eq!(refs[0].span().start_line(), 42);

    // Type query
    let ty = reader.type_at("src/main.rs", 42, 15).unwrap();
    assert_eq!(ty, "Result<usize, String>");

    // Verify cursor on definition returns innermost definition (Struct rather than enclosing Mod)
    let def_cursor = reader
        .find_definition_at("src/processor.rs", 10, 5)
        .unwrap();
    assert_eq!(def_cursor.name, "DataProcessor");
    assert_eq!(def_cursor.start_line, 10);
}
