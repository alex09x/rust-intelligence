use ridx_format::{
    DefKind, IndexWriter, RawDefinition, RawReference, RawType, WorkspaceIndex,
};
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::PathBuf;

#[test]
fn test_multi_crate_workspace_index() {
    let ws_dir = PathBuf::from("target/test_workspace");
    fs::create_dir_all(&ws_dir).unwrap();

    // 1. Write crate_a: defines `TypeA` and `func_a`
    let mut writer_a = IndexWriter::new("crate_a", "0.1.0");
    writer_a.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 0,
        name: "TypeA".to_string(),
        path: "crate_a::types::TypeA".to_string(),
        signature: None,
        file: "crates/crate_a/src/types.rs".to_string(),
        start_line: 10,
        start_col: 1,
        end_line: 15,
        end_col: 2,
    });
    writer_a.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Fn,
        visibility: 0,
        name: "func_a".to_string(),
        path: "crate_a::func_a".to_string(),
        signature: Some("fn() -> TypeA".to_string()),
        file: "crates/crate_a/src/lib.rs".to_string(),
        start_line: 5,
        start_col: 1,
        end_line: 7,
        end_col: 2,
    });

    let path_a = ws_dir.join("crate_a.ridx");
    let f_a = File::create(&path_a).unwrap();
    writer_a.write_to(&mut BufWriter::new(f_a)).unwrap();

    // 2. Write crate_b: depends on crate_a, references `crate_a::types::TypeA`
    let mut writer_b = IndexWriter::new("crate_b", "0.2.0");
    writer_b.add_definition(RawDefinition {
        def_id: 10,
        kind: DefKind::Fn,
        visibility: 0,
        name: "use_a".to_string(),
        path: "crate_b::use_a".to_string(),
        signature: Some("fn(TypeA)".to_string()),
        file: "crates/crate_b/src/lib.rs".to_string(),
        start_line: 12,
        start_col: 1,
        end_line: 14,
        end_col: 2,
    });
    // Reference from crate_b to crate_a::types::TypeA
    writer_b.add_reference(RawReference {
        file: "crates/crate_b/src/lib.rs".to_string(),
        start_line: 12,
        start_col: 11,
        end_line: 12,
        end_col: 16,
        target_def_id: None, // Cross-crate reference, no local def_id
        target_path: "crate_a::types::TypeA".to_string(),
        target_file: Some("crates/crate_a/src/types.rs".to_string()),
        target_line: 10,
        target_col: 1,
        kind: DefKind::Struct,
        is_write: false,
    });
    // Type at position in crate_b
    writer_b.add_type(RawType {
        file: "crates/crate_b/src/lib.rs".to_string(),
        start_line: 12,
        start_col: 11,
        end_line: 12,
        end_col: 16,
        type_name: "crate_a::types::TypeA".to_string(),
    });

    let path_b = ws_dir.join("crate_b.ridx");
    let f_b = File::create(&path_b).unwrap();
    writer_b.write_to(&mut BufWriter::new(f_b)).unwrap();

    // 3. Load entire workspace from directory
    let ws = WorkspaceIndex::open(&ws_dir).expect("failed to open workspace directory");
    assert_eq!(ws.crate_count(), 2);

    let stats = ws.stats();
    assert_eq!(stats.total_crates, 2);
    assert_eq!(stats.total_defs, 3);
    assert_eq!(stats.total_refs, 1);
    assert_eq!(stats.total_types, 1);

    // 4. Test find_definition_by_path across crates
    let def_a = ws
        .find_definition_by_path("crate_a::types::TypeA")
        .expect("should find TypeA in crate_a");
    assert_eq!(def_a.name, "TypeA");
    assert_eq!(def_a.file, "crates/crate_a/src/types.rs");
    assert_eq!(def_a.start_line, 10);

    let def_b = ws
        .find_definition_by_path("crate_b::use_a")
        .expect("should find use_a in crate_b");
    assert_eq!(def_b.name, "use_a");
    assert_eq!(def_b.signature, Some("fn(TypeA)"));

    // 5. Test cross-crate go-to-definition at cursor site in crate_b
    let cross_def = ws
        .find_definition_at("crates/crate_b/src/lib.rs", 12, 12)
        .expect("should resolve cross-crate reference to TypeA");
    assert_eq!(cross_def.name, "TypeA");
    assert_eq!(cross_def.path, "crate_a::types::TypeA");
    assert_eq!(cross_def.file, "crates/crate_a/src/types.rs");
    assert_eq!(cross_def.start_line, 10);

    // 6. Test find_references across workspace
    let refs = ws.find_references("crate_a::types::TypeA");
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].crate_name, "crate_b");
    assert_eq!(refs[0].file, "crates/crate_b/src/lib.rs");
    assert_eq!(refs[0].span.start_line(), 12);

    // 7. Test type_at
    let ty = ws
        .type_at("crates/crate_b/src/lib.rs", 12, 13)
        .expect("should find inferred type");
    assert_eq!(ty, "crate_a::types::TypeA");

    // 8. Test workspace-wide search_symbols
    let matches = ws.search_symbols("func");
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].name, "func_a");
    assert_eq!(matches[0].crate_name, "crate_a");
}
