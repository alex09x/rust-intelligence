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
        is_macro_expansion: false,
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
        is_macro_expansion: false,
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
        is_macro_expansion: false,
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
        is_macro_expansion: false,
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
    assert_eq!(def_a.end_line, 15);

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
    assert_eq!(cross_def.end_line, 15);
    assert_eq!(cross_def.end_col, 2);

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

    // 9. Test find_references_at cursor location in crate_b
    let loc_refs = ws
        .find_references_at("crates/crate_b/src/lib.rs", 12, 12)
        .expect("should find references at cursor");
    assert_eq!(loc_refs.target_symbol, "TypeA");
    assert_eq!(loc_refs.target_end_line, 15);
    assert_eq!(loc_refs.target_end_col, 2);
    assert_eq!(loc_refs.references.len(), 1);
    assert_eq!(loc_refs.references[0].crate_name, "crate_b");

    // 10. Test hover_at combining definition and inferred type
    let hover = ws
        .hover_at("crates/crate_b/src/lib.rs", 12, 12)
        .expect("should get hover");
    assert!(hover.definition.is_some());
    assert_eq!(hover.definition.unwrap().name, "TypeA");
    assert_eq!(hover.inferred_type, Some("crate_a::types::TypeA"));
}

#[test]
fn test_variant_aware_workspace_and_reload() {
    let base_dir = PathBuf::from("target/test_variants_ws");
    let _ = fs::remove_dir_all(&base_dir);
    fs::create_dir_all(&base_dir).unwrap();

    let var1_dir = base_dir.join("variants/var1");
    fs::create_dir_all(&var1_dir).unwrap();

    // 1. Write crate_a in var1
    let mut writer1 = IndexWriter::new("crate_a", "0.1.0");
    writer1.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "func_var1".to_string(),
        path: "crate_a::func_var1".to_string(),
        signature: Some("fn()".to_string()),
        file: "crates/crate_a/src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 3,
        end_col: 2,
    });
    let f1 = File::create(var1_dir.join("crate_a.ridx")).unwrap();
    writer1.write_to(&mut BufWriter::new(f1)).unwrap();

    // 2. Write manifest pointing to var1
    let manifest1 = serde_json::json!({
        "version": "2.0",
        "active_variant_id": "var1",
        "variant_dir": "variants/var1",
        "workspace_root": base_dir.display().to_string(),
        "manifest_path": base_dir.join("Cargo.toml").display().to_string(),
        "profile": "dev",
        "total_crates": 1,
        "total_defs": 1,
        "total_refs": 0,
        "total_types": 0,
    });
    fs::write(base_dir.join("ridx_manifest.json"), serde_json::to_string_pretty(&manifest1).unwrap()).unwrap();

    // 3. Open workspace
    let mut ws = WorkspaceIndex::open(&base_dir).expect("should open variant workspace");
    assert_eq!(ws.active_variant_id(), Some("var1"));
    assert_eq!(ws.stats().active_variant.as_deref(), Some("var1"));
    assert!(ws.find_definition_by_path("crate_a::func_var1").is_some());

    // 4. Create var2 with different definition
    let var2_dir = base_dir.join("variants/var2");
    fs::create_dir_all(&var2_dir).unwrap();

    let mut writer2 = IndexWriter::new("crate_a", "0.2.0");
    writer2.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "func_var2".to_string(),
        path: "crate_a::func_var2".to_string(),
        signature: Some("fn() -> i32".to_string()),
        file: "crates/crate_a/src/lib.rs".to_string(),
        start_line: 10,
        start_col: 1,
        end_line: 12,
        end_col: 2,
    });
    let f2 = File::create(var2_dir.join("crate_a.ridx")).unwrap();
    writer2.write_to(&mut BufWriter::new(f2)).unwrap();

    // 5. Update manifest atomically
    std::thread::sleep(std::time::Duration::from_millis(50));
    let manifest2 = serde_json::json!({
        "version": "2.0",
        "active_variant_id": "var2",
        "variant_dir": "variants/var2",
        "workspace_root": base_dir.display().to_string(),
        "manifest_path": base_dir.join("Cargo.toml").display().to_string(),
        "profile": "release",
        "total_crates": 1,
        "total_defs": 1,
        "total_refs": 0,
        "total_types": 0,
    });
    fs::write(base_dir.join("ridx_manifest.json"), serde_json::to_string_pretty(&manifest2).unwrap()).unwrap();

    // 6. Reload workspace index
    let changed = ws.reload_if_modified().expect("reload should succeed");
    assert!(changed);
    assert_eq!(ws.active_variant_id(), Some("var2"));
    assert_eq!(ws.stats().active_variant.as_deref(), Some("var2"));
    assert!(ws.find_definition_by_path("crate_a::func_var1").is_none());
    let def2 = ws.find_definition_by_path("crate_a::func_var2").expect("should find func_var2 in var2");
    assert_eq!(def2.signature, Some("fn() -> i32"));
}
