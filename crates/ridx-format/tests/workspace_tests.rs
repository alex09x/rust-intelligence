use ridx_format::{DefKind, IndexWriter, RawDefinition, RawReference, RawType, WorkspaceIndex};
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
        target_unit: None,
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
        "format_version": ridx_format::CURRENT_FORMAT_VERSION,
        "extractor_version": ridx_format::EXTRACTOR_VERSION,
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
    fs::write(
        base_dir.join("ridx_manifest.json"),
        serde_json::to_string_pretty(&manifest1).unwrap(),
    )
    .unwrap();

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
        "format_version": ridx_format::CURRENT_FORMAT_VERSION,
        "extractor_version": ridx_format::EXTRACTOR_VERSION,
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
    fs::write(
        base_dir.join("ridx_manifest.json"),
        serde_json::to_string_pretty(&manifest2).unwrap(),
    )
    .unwrap();

    // 6. Reload workspace index
    let changed = ws.reload_if_modified().expect("reload should succeed");
    assert!(changed);
    assert_eq!(ws.active_variant_id(), Some("var2"));
    assert_eq!(ws.stats().active_variant.as_deref(), Some("var2"));
    assert!(ws.find_definition_by_path("crate_a::func_var1").is_none());
    let def2 = ws
        .find_definition_by_path("crate_a::func_var2")
        .expect("should find func_var2 in var2");
    assert_eq!(def2.signature, Some("fn() -> i32"));
}

#[test]
fn test_missing_active_variant_returns_not_found() {
    let base_dir = PathBuf::from("target/test_missing_variant_ws");
    let _ = fs::remove_dir_all(&base_dir);
    fs::create_dir_all(&base_dir).unwrap();

    // Write a manifest pointing to a nonexistent variant dir
    let manifest = serde_json::json!({
        "version": "2.0",
        "active_variant_id": "nonexistent_var",
        "variant_dir": "variants/nonexistent_var",
        "workspace_root": base_dir.display().to_string(),
        "manifest_path": base_dir.join("Cargo.toml").display().to_string(),
        "profile": "dev",
    });
    fs::write(
        base_dir.join("ridx_manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    // Put a legacy .ridx file in base_dir to ensure it is NOT loaded as a fallback
    let mut writer = IndexWriter::new("legacy_crate", "0.0.1");
    let f = File::create(base_dir.join("legacy.ridx")).unwrap();
    writer.write_to(&mut BufWriter::new(f)).unwrap();

    // WorkspaceIndex::open must return NotFound, not succeed by falling back to legacy.ridx
    let res = WorkspaceIndex::open(&base_dir);
    assert!(
        res.is_err(),
        "should fail to open when active variant directory does not exist"
    );
    assert_eq!(res.err().unwrap().kind(), std::io::ErrorKind::NotFound);
}

#[test]
fn test_active_manifest_with_empty_or_escaping_variant_dir_rejected() {
    let base_dir = PathBuf::from("target/test_invalid_variant_dir_ws");
    let _ = fs::remove_dir_all(&base_dir);
    fs::create_dir_all(&base_dir).unwrap();

    // Put a legacy .ridx in base_dir
    let mut writer = IndexWriter::new("legacy_crate", "0.0.1");
    let f = File::create(base_dir.join("legacy.ridx")).unwrap();
    writer.write_to(&mut BufWriter::new(f)).unwrap();

    // 1. Empty variant_dir
    let manifest_empty = serde_json::json!({
        "version": "2.0",
        "active_variant_id": "var1",
        "variant_dir": "",
        "workspace_root": base_dir.display().to_string(),
        "manifest_path": base_dir.join("Cargo.toml").display().to_string(),
        "profile": "dev",
    });
    fs::write(
        base_dir.join("ridx_manifest.json"),
        serde_json::to_string_pretty(&manifest_empty).unwrap(),
    )
    .unwrap();

    let res = WorkspaceIndex::open(&base_dir);
    assert!(res.is_err(), "should reject empty variant_dir");
    assert_eq!(res.err().unwrap().kind(), std::io::ErrorKind::InvalidData);

    // 2. Dot variant_dir
    let manifest_dot = serde_json::json!({
        "version": "2.0",
        "active_variant_id": "var1",
        "variant_dir": ".",
        "workspace_root": base_dir.display().to_string(),
        "manifest_path": base_dir.join("Cargo.toml").display().to_string(),
        "profile": "dev",
    });
    fs::write(
        base_dir.join("ridx_manifest.json"),
        serde_json::to_string_pretty(&manifest_dot).unwrap(),
    )
    .unwrap();

    let res = WorkspaceIndex::open(&base_dir);
    assert!(res.is_err(), "should reject '.' variant_dir");
    assert_eq!(res.err().unwrap().kind(), std::io::ErrorKind::InvalidData);

    // 3. Escaping variant_dir
    let manifest_escape = serde_json::json!({
        "version": "2.0",
        "active_variant_id": "var1",
        "variant_dir": "../escaping",
        "workspace_root": base_dir.display().to_string(),
        "manifest_path": base_dir.join("Cargo.toml").display().to_string(),
        "profile": "dev",
    });
    fs::write(
        base_dir.join("ridx_manifest.json"),
        serde_json::to_string_pretty(&manifest_escape).unwrap(),
    )
    .unwrap();

    let res = WorkspaceIndex::open(&base_dir);
    assert!(res.is_err(), "should reject escaping variant_dir");
    assert_eq!(res.err().unwrap().kind(), std::io::ErrorKind::InvalidData);

    // 4. Mismatched variant_id and variant_dir
    let manifest_mismatch = serde_json::json!({
        "version": "2.0",
        "active_variant_id": "var1",
        "variant_dir": "variants/var2",
        "workspace_root": base_dir.display().to_string(),
        "manifest_path": base_dir.join("Cargo.toml").display().to_string(),
        "profile": "dev",
    });
    fs::write(
        base_dir.join("ridx_manifest.json"),
        serde_json::to_string_pretty(&manifest_mismatch).unwrap(),
    )
    .unwrap();

    let res = WorkspaceIndex::open(&base_dir);
    assert!(
        res.is_err(),
        "should reject mismatched variant_id and variant_dir"
    );
    assert_eq!(res.err().unwrap().kind(), std::io::ErrorKind::InvalidData);
}

#[test]
fn test_version_mismatch_rejected() {
    let test_dir = std::env::temp_dir().join(format!("ridx_ver_test_{}", std::process::id()));
    let _ = fs::remove_dir_all(&test_dir);
    fs::create_dir_all(&test_dir).unwrap();

    let manifest_future = serde_json::json!({
        "version": "2.0",
        "format_version": 999, // Future format version
        "active_variant_id": "var_future",
        "variant_dir": "variants/var_future",
        "workspace_root": test_dir.display().to_string(),
        "manifest_path": test_dir.join("Cargo.toml").display().to_string(),
        "profile": "dev",
    });

    let var_dir = test_dir.join("variants/var_future");
    fs::create_dir_all(&var_dir).unwrap();

    let mut writer = IndexWriter::new("test_crate", "0.1.0");
    let f = File::create(var_dir.join("test_crate.ridx")).unwrap();
    writer.write_to(&mut BufWriter::new(f)).unwrap();

    fs::write(
        test_dir.join("ridx_manifest.json"),
        serde_json::to_string_pretty(&manifest_future).unwrap(),
    )
    .unwrap();

    let res = WorkspaceIndex::open(&test_dir);
    assert!(res.is_err(), "should reject future format_version");
    let err = res.err().unwrap();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    assert!(ridx_format::is_version_mismatch(&err));

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_dependency_shard_loading() {
    let test_dir = std::env::temp_dir().join(format!("ridx_dep_test_{}", std::process::id()));
    let _ = fs::remove_dir_all(&test_dir);
    let var_dir = test_dir.join("variants/var1");
    fs::create_dir_all(&var_dir).unwrap();

    // 1. Local crate in workspace
    let mut writer_local = IndexWriter::new("local_app", "0.1.0");
    writer_local.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "main".to_string(),
        path: "local_app::main".to_string(),
        signature: Some("fn()".to_string()),
        file: "src/main.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 5,
        end_col: 2,
    });
    let f_local = File::create(var_dir.join("local_app.ridx")).unwrap();
    writer_local.write_to(&mut BufWriter::new(f_local)).unwrap();

    // 2. Dependency crate in separate cache location
    let cache_dir = test_dir.join("cache/objects/ab");
    fs::create_dir_all(&cache_dir).unwrap();
    let dep_obj_path = cache_dir.join("ab1234567890.ridx");

    let mut writer_dep = IndexWriter::new("dep_lib", "1.2.0");
    writer_dep.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "DepConfig".to_string(),
        path: "dep_lib::DepConfig".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 10,
        start_col: 1,
        end_line: 20,
        end_col: 2,
    });
    let f_dep = File::create(&dep_obj_path).unwrap();
    writer_dep.write_to(&mut BufWriter::new(f_dep)).unwrap();

    // 3. Manifest linking dependency shard
    let manifest = serde_json::json!({
        "version": "2.0",
        "format_version": ridx_format::CURRENT_FORMAT_VERSION,
        "extractor_version": ridx_format::EXTRACTOR_VERSION,
        "active_variant_id": "var1",
        "variant_dir": "variants/var1",
        "workspace_root": test_dir.display().to_string(),
        "manifest_path": test_dir.join("Cargo.toml").display().to_string(),
        "profile": "dev",
        "dependency_shards": [
            {
                "name": "dep_lib",
                "version": "1.2.0",
                "unit_key": "ab1234567890",
                "object_path": dep_obj_path.display().to_string(),
                "source_root": "/cargo/registry/dep_lib-1.2.0"
            }
        ]
    });
    fs::write(
        test_dir.join("ridx_manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    // 4. Open workspace and verify both local and dependency crates are accessible
    let ws =
        WorkspaceIndex::open(&test_dir).expect("failed to open workspace with dependency shards");
    assert_eq!(ws.crate_count(), 2);

    let stats = ws.stats();
    assert_eq!(stats.total_crates, 2);
    let dep_summary = stats
        .crate_summaries
        .iter()
        .find(|c| c.name == "dep_lib")
        .unwrap();
    assert!(dep_summary.is_dependency);

    // Cross-crate query reaches dependency definition
    let def = ws
        .find_definition_by_path("dep_lib::DepConfig")
        .expect("should find dep definition");
    assert_eq!(def.name, "DepConfig");

    // Path resolution for dependency crate
    let resolved = ws.resolve_path_for_crate("dep_lib", def.file);
    assert_eq!(
        resolved,
        PathBuf::from("/cargo/registry/dep_lib-1.2.0/src/lib.rs")
    );

    // Find definition at cursor inside dependency file (given absolute path)
    let def_at = ws.find_definition_at("/cargo/registry/dep_lib-1.2.0/src/lib.rs", 12, 5);
    assert!(def_at.is_some());
    assert_eq!(def_at.unwrap().name, "DepConfig");

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_reviewer_cursor_keeps_owning_dependency_unit() {
    let test_dir = std::env::temp_dir().join(format!("ridx_dep_unit_test_{}", std::process::id()));
    let _ = fs::remove_dir_all(&test_dir);
    let var_dir = test_dir.join("variants/var1");
    fs::create_dir_all(&var_dir).unwrap();

    let cache_dir = test_dir.join("cache/objects/ab");
    fs::create_dir_all(&cache_dir).unwrap();
    let dep_a_path = cache_dir.join("unitA.ridx");
    let dep_b_path = cache_dir.join("unitB.ridx");

    // Local workspace crate
    let mut writer_local = IndexWriter::new("local_app", "0.1.0");
    writer_local.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "main".to_string(),
        path: "local_app::main".to_string(),
        signature: Some("fn()".to_string()),
        file: "src/main.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 5,
        end_col: 2,
    });
    writer_local
        .write_to(&mut BufWriter::new(
            File::create(var_dir.join("local_app.ridx")).unwrap(),
        ))
        .unwrap();

    // Both shard A and shard B define `dep::Thing`
    let mut writer_a = IndexWriter::new("dep", "1.0.0");
    writer_a.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "Thing".to_string(),
        path: "dep::Thing".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 10,
        start_col: 1,
        end_line: 15,
        end_col: 2,
    });
    writer_a
        .write_to(&mut BufWriter::new(File::create(&dep_a_path).unwrap()))
        .unwrap();

    let mut writer_b = IndexWriter::new("dep", "2.0.0");
    writer_b.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "Thing".to_string(),
        path: "dep::Thing".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 20,
        start_col: 1,
        end_line: 25,
        end_col: 2,
    });
    writer_b
        .write_to(&mut BufWriter::new(File::create(&dep_b_path).unwrap()))
        .unwrap();

    // Manifest with both dependency shards
    let manifest = serde_json::json!({
        "version": "2.0",
        "format_version": ridx_format::CURRENT_FORMAT_VERSION,
        "extractor_version": ridx_format::EXTRACTOR_VERSION,
        "active_variant_id": "var1",
        "variant_dir": "variants/var1",
        "workspace_root": test_dir.display().to_string(),
        "manifest_path": test_dir.join("Cargo.toml").display().to_string(),
        "profile": "dev",
        "dependency_shards": [
            {
                "name": "dep",
                "version": "1.0.0",
                "unit_key": "unitA",
                "object_path": dep_a_path.display().to_string(),
                "source_root": "/registry/dep-a"
            },
            {
                "name": "dep",
                "version": "2.0.0",
                "unit_key": "unitB",
                "object_path": dep_b_path.display().to_string(),
                "source_root": "/registry/dep-b"
            }
        ]
    });
    fs::write(
        test_dir.join("ridx_manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let ws = WorkspaceIndex::open(&test_dir)
        .expect("failed to open workspace with multi-version dependency shards");

    // Querying cursor inside dep-b (/registry/dep-b/src/lib.rs at line 20) MUST resolve to unitB, NOT unitA!
    let def_b = ws
        .find_definition_at("/registry/dep-b/src/lib.rs", 20, 1)
        .expect("should resolve in unit B");
    assert_eq!(def_b.name, "Thing");
    assert_eq!(def_b.unit_key, Some("unitB"));
    assert_eq!(def_b.start_line, 20);

    // Querying cursor inside dep-a (/registry/dep-a/src/lib.rs at line 10) MUST resolve to unitA!
    let def_a = ws
        .find_definition_at("/registry/dep-a/src/lib.rs", 10, 1)
        .expect("should resolve in unit A");
    assert_eq!(def_a.name, "Thing");
    assert_eq!(def_a.unit_key, Some("unitA"));
    assert_eq!(def_a.start_line, 10);

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_exact_unit_cross_definition_and_references() {
    let test_dir = std::env::temp_dir().join(format!("ridx_exact_unit_{}", std::process::id()));
    let _ = fs::remove_dir_all(&test_dir);
    let var_dir = test_dir.join("variants/var1");
    let deps_dir = var_dir.join(".deps");
    fs::create_dir_all(&deps_dir).unwrap();

    let cache_dir = test_dir.join("cache/objects");
    fs::create_dir_all(&cache_dir).unwrap();
    let dep_a1_path = cache_dir.join("dep_a1.ridx");
    let dep_a2_path = cache_dir.join("dep_a2.ridx");

    // 1. Shard dep v1.0.0 (unitA1)
    let mut writer_a1 = IndexWriter::new("dep", "1.0.0");
    writer_a1.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "foo".to_string(),
        path: "dep::foo".to_string(),
        signature: Some("fn() -> i32".to_string()),
        file: "src/lib.rs".to_string(),
        start_line: 10,
        start_col: 1,
        end_line: 12,
        end_col: 2,
    });
    writer_a1
        .write_to(&mut BufWriter::new(File::create(&dep_a1_path).unwrap()))
        .unwrap();

    // 2. Shard dep v2.0.0 (unitA2)
    let mut writer_a2 = IndexWriter::new("dep", "2.0.0");
    writer_a2.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "foo".to_string(),
        path: "dep::foo".to_string(),
        signature: Some("fn() -> String".to_string()),
        file: "src/lib.rs".to_string(),
        start_line: 20,
        start_col: 1,
        end_line: 22,
        end_col: 2,
    });
    writer_a2
        .write_to(&mut BufWriter::new(File::create(&dep_a2_path).unwrap()))
        .unwrap();

    // 3. Workspace crate b1 (depends on unitA1)
    let mut writer_b1 = IndexWriter::new("b1", "0.1.0");
    writer_b1.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "call_a1".to_string(),
        path: "b1::call_a1".to_string(),
        signature: Some("fn()".to_string()),
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 3,
        end_col: 2,
    });
    writer_b1.add_reference(RawReference {
        file: "src/lib.rs".to_string(),
        start_line: 2,
        start_col: 5,
        end_line: 2,
        end_col: 13,
        target_def_id: None,
        target_path: "dep::foo".to_string(),
        target_unit: None,
        target_file: Some("src/lib.rs".to_string()),
        target_line: 10,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    let b1_ridx = var_dir.join("b1.ridx");
    writer_b1
        .write_to(&mut BufWriter::new(File::create(&b1_ridx).unwrap()))
        .unwrap();

    // Write .deps/b1.json metadata specifying exact unit dependency
    let b1_meta = serde_json::json!({
        "name": "b1",
        "version": "0.1.0",
        "unit_key": "unitB1",
        "object_path": b1_ridx.display().to_string(),
        "dependencies": ["unitA1"],
        "extern_units": {
            "dep": "unitA1"
        }
    });
    fs::write(
        deps_dir.join("b1.json"),
        serde_json::to_string_pretty(&b1_meta).unwrap(),
    )
    .unwrap();

    // 4. Workspace crate b2 (depends on unitA2)
    let mut writer_b2 = IndexWriter::new("b2", "0.1.0");
    writer_b2.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "call_a2".to_string(),
        path: "b2::call_a2".to_string(),
        signature: Some("fn()".to_string()),
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 3,
        end_col: 2,
    });
    writer_b2.add_reference(RawReference {
        file: "src/lib.rs".to_string(),
        start_line: 2,
        start_col: 5,
        end_line: 2,
        end_col: 13,
        target_def_id: None,
        target_path: "dep::foo".to_string(),
        target_unit: None,
        target_file: Some("src/lib.rs".to_string()),
        target_line: 20,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    let b2_ridx = var_dir.join("b2.ridx");
    writer_b2
        .write_to(&mut BufWriter::new(File::create(&b2_ridx).unwrap()))
        .unwrap();

    // Write .deps/b2.json metadata specifying exact unit dependency
    let b2_meta = serde_json::json!({
        "name": "b2",
        "version": "0.1.0",
        "unit_key": "unitB2",
        "object_path": b2_ridx.display().to_string(),
        "dependencies": ["unitA2"],
        "extern_units": {
            "dep": "unitA2"
        }
    });
    fs::write(
        deps_dir.join("b2.json"),
        serde_json::to_string_pretty(&b2_meta).unwrap(),
    )
    .unwrap();

    // 5. Manifest linking both dependency shards
    let manifest = serde_json::json!({
        "version": "2.0",
        "format_version": ridx_format::CURRENT_FORMAT_VERSION,
        "extractor_version": ridx_format::EXTRACTOR_VERSION,
        "active_variant_id": "var1",
        "variant_dir": "variants/var1",
        "workspace_root": test_dir.display().to_string(),
        "manifest_path": test_dir.join("Cargo.toml").display().to_string(),
        "profile": "dev",
        "dependency_shards": [
            {
                "name": "dep",
                "version": "1.0.0",
                "unit_key": "unitA1",
                "object_path": dep_a1_path.display().to_string(),
                "source_root": "/registry/dep-1.0.0"
            },
            {
                "name": "dep",
                "version": "2.0.0",
                "unit_key": "unitA2",
                "object_path": dep_a2_path.display().to_string(),
                "source_root": "/registry/dep-2.0.0"
            }
        ]
    });
    fs::write(
        test_dir.join("ridx_manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let ws = WorkspaceIndex::open(&test_dir)
        .expect("failed to open workspace with multi-version dependency shards");

    // Cross-definition from b1 must resolve strictly to unitA1
    let def_from_b1 = ws
        .find_definition_at("src/lib.rs", 2, 6)
        .expect("should resolve dep::foo from b1");
    assert_eq!(def_from_b1.unit_key, Some("unitA1"));
    assert_eq!(def_from_b1.start_line, 10);
    assert_eq!(def_from_b1.signature, Some("fn() -> i32"));

    // Cross-references for unitA1 must ONLY include b1, excluding b2!
    let refs_a1 = ws.find_references_for_unit("dep::foo", Some("unitA1"));
    assert_eq!(
        refs_a1.len(),
        1,
        "references to unitA1 must only include b1"
    );
    assert_eq!(refs_a1[0].crate_name, "b1");

    // Cross-references for unitA2 must ONLY include b2, excluding b1!
    let refs_a2 = ws.find_references_for_unit("dep::foo", Some("unitA2"));
    assert_eq!(
        refs_a2.len(),
        1,
        "references to unitA2 must only include b2"
    );
    assert_eq!(refs_a2[0].crate_name, "b2");

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_consumer_with_aliased_dependencies() {
    let test_dir = std::env::temp_dir().join(format!("ridx_aliased_dep_{}", std::process::id()));
    let _ = fs::remove_dir_all(&test_dir);
    let var_dir = test_dir.join("variants/var1");
    let deps_dir = var_dir.join(".deps");
    fs::create_dir_all(&deps_dir).unwrap();

    let cache_dir = test_dir.join("cache/objects");
    fs::create_dir_all(&cache_dir).unwrap();
    let dep_a1_path = cache_dir.join("dep_a1.ridx");
    let dep_a2_path = cache_dir.join("dep_a2.ridx");

    // 1. Shard dep v1.0.0 (unitA1)
    let mut writer_a1 = IndexWriter::new("dep", "1.0.0");
    writer_a1.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "foo".to_string(),
        path: "dep::foo".to_string(),
        signature: Some("fn() -> i32".to_string()),
        file: "src/lib.rs".to_string(),
        start_line: 10,
        start_col: 1,
        end_line: 12,
        end_col: 2,
    });
    writer_a1
        .write_to(&mut BufWriter::new(File::create(&dep_a1_path).unwrap()))
        .unwrap();

    // 2. Shard dep v2.0.0 (unitA2)
    let mut writer_a2 = IndexWriter::new("dep", "2.0.0");
    writer_a2.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "foo".to_string(),
        path: "dep::foo".to_string(),
        signature: Some("fn() -> String".to_string()),
        file: "src/lib.rs".to_string(),
        start_line: 20,
        start_col: 1,
        end_line: 22,
        end_col: 2,
    });
    writer_a2
        .write_to(&mut BufWriter::new(File::create(&dep_a2_path).unwrap()))
        .unwrap();

    // 3. Workspace crate app (uses aliases `dep1` -> unitA1 and `dep2` -> unitA2)
    let mut writer_app = IndexWriter::new("app", "0.1.0");
    writer_app.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 5,
        start_col: 5,
        end_line: 5,
        end_col: 14,
        target_def_id: None,
        target_path: "dep1::foo".to_string(),
        target_unit: None,
        target_file: Some("src/lib.rs".to_string()),
        target_line: 10,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer_app.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 8,
        start_col: 5,
        end_line: 8,
        end_col: 14,
        target_def_id: None,
        target_path: "dep2::foo".to_string(),
        target_unit: None,
        target_file: Some("src/lib.rs".to_string()),
        target_line: 20,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    let app_ridx = var_dir.join("app.ridx");
    writer_app
        .write_to(&mut BufWriter::new(File::create(&app_ridx).unwrap()))
        .unwrap();

    // Write .deps/app.json metadata with extern_units mappings for the aliases
    let app_meta = serde_json::json!({
        "name": "app",
        "version": "0.1.0",
        "unit_key": "unitApp",
        "object_path": app_ridx.display().to_string(),
        "dependencies": ["unitA1", "unitA2"],
        "extern_units": {
            "dep1": "unitA1",
            "dep2": "unitA2"
        }
    });
    fs::write(
        deps_dir.join("app.json"),
        serde_json::to_string_pretty(&app_meta).unwrap(),
    )
    .unwrap();

    let manifest = serde_json::json!({
        "version": "2.0",
        "format_version": ridx_format::CURRENT_FORMAT_VERSION,
        "extractor_version": ridx_format::EXTRACTOR_VERSION,
        "active_variant_id": "var1",
        "variant_dir": "variants/var1",
        "workspace_root": test_dir.display().to_string(),
        "manifest_path": test_dir.join("Cargo.toml").display().to_string(),
        "profile": "dev",
        "dependency_shards": [
            {
                "name": "dep",
                "version": "1.0.0",
                "unit_key": "unitA1",
                "object_path": dep_a1_path.display().to_string(),
                "source_root": "/registry/dep-1.0.0"
            },
            {
                "name": "dep",
                "version": "2.0.0",
                "unit_key": "unitA2",
                "object_path": dep_a2_path.display().to_string(),
                "source_root": "/registry/dep-2.0.0"
            }
        ]
    });
    fs::write(
        test_dir.join("ridx_manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let ws = WorkspaceIndex::open(&test_dir)
        .expect("failed to open workspace with aliased dependencies");

    // Definition lookup for aliased target `dep1::foo` must resolve to unitA1
    let def_dep1 = ws
        .find_definition_at("src/main.rs", 5, 6)
        .expect("should resolve dep1::foo");
    assert_eq!(def_dep1.unit_key, Some("unitA1"));
    assert_eq!(def_dep1.start_line, 10);
    assert_eq!(def_dep1.signature, Some("fn() -> i32"));

    // Definition lookup for aliased target `dep2::foo` must resolve to unitA2
    let def_dep2 = ws
        .find_definition_at("src/main.rs", 8, 6)
        .expect("should resolve dep2::foo");
    assert_eq!(def_dep2.unit_key, Some("unitA2"));
    assert_eq!(def_dep2.start_line, 20);
    assert_eq!(def_dep2.signature, Some("fn() -> String"));

    // References for unitA1 must find the aliased reference at line 5
    let refs_a1 = ws.find_references_for_unit("dep::foo", Some("unitA1"));
    assert_eq!(refs_a1.len(), 1);
    assert_eq!(refs_a1[0].span.start_line(), 5);

    // References for unitA2 must find the aliased reference at line 8
    let refs_a2 = ws.find_references_for_unit("dep::foo", Some("unitA2"));
    assert_eq!(refs_a2.len(), 1);
    assert_eq!(refs_a2[0].span.start_line(), 8);

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_consumer_with_dual_version_canonical_references() {
    let test_dir = std::env::temp_dir().join(format!("ridx_canonical_dual_{}", std::process::id()));
    let _ = fs::remove_dir_all(&test_dir);
    let var_dir = test_dir.join("variants/var1");
    let deps_dir = var_dir.join(".deps");
    fs::create_dir_all(&deps_dir).unwrap();

    let cache_dir = test_dir.join("cache/objects");
    fs::create_dir_all(&cache_dir).unwrap();
    let dep_a1_path = cache_dir.join("dep_a1.ridx");
    let dep_a2_path = cache_dir.join("dep_a2.ridx");

    // 1. Dependency shard A1: crate "a" v1.0.0, unitA1, defines a::Thing at line 10
    let mut writer_a1 = IndexWriter::new("a", "1.0.0");
    writer_a1.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "Thing".to_string(),
        path: "a::Thing".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 10,
        start_col: 1,
        end_line: 12,
        end_col: 2,
    });
    writer_a1
        .write_to(&mut BufWriter::new(File::create(&dep_a1_path).unwrap()))
        .unwrap();

    // 2. Dependency shard A2: crate "a" v2.0.0, unitA2, defines a::Thing at line 20
    let mut writer_a2 = IndexWriter::new("a", "2.0.0");
    writer_a2.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "Thing".to_string(),
        path: "a::Thing".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 20,
        start_col: 1,
        end_line: 22,
        end_col: 2,
    });
    writer_a2
        .write_to(&mut BufWriter::new(File::create(&dep_a2_path).unwrap()))
        .unwrap();

    // 3. Consumer crate C: references canonical a::Thing at line 4 (pointing to A1's line 10)
    // and canonical a::Thing at line 8 (pointing to A2's line 20)
    let mut writer_c = IndexWriter::new("c", "0.1.0");
    writer_c.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 4,
        start_col: 5,
        end_line: 4,
        end_col: 13,
        target_def_id: None,
        target_path: "a::Thing".to_string(),
        target_unit: None,
        target_file: Some("src/lib.rs".to_string()),
        target_line: 10,
        target_col: 1,
        kind: DefKind::Struct,
        is_write: false,
        is_macro_expansion: false,
    });
    writer_c.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 8,
        start_col: 5,
        end_line: 8,
        end_col: 13,
        target_def_id: None,
        target_path: "a::Thing".to_string(),
        target_unit: None,
        target_file: Some("src/lib.rs".to_string()),
        target_line: 20,
        target_col: 1,
        kind: DefKind::Struct,
        is_write: false,
        is_macro_expansion: false,
    });
    let c_ridx = var_dir.join("c.ridx");
    writer_c
        .write_to(&mut BufWriter::new(File::create(&c_ridx).unwrap()))
        .unwrap();

    // Sidecar metadata for C with extern_units mapping a1 -> unitA1, a2 -> unitA2
    let c_meta = serde_json::json!({
        "name": "c",
        "version": "0.1.0",
        "unit_key": "unitC",
        "object_path": c_ridx.display().to_string(),
        "dependencies": ["unitA1", "unitA2"],
        "extern_units": {
            "a1": "unitA1",
            "a2": "unitA2"
        }
    });
    fs::write(
        deps_dir.join("c.json"),
        serde_json::to_string_pretty(&c_meta).unwrap(),
    )
    .unwrap();

    let manifest = serde_json::json!({
        "version": "2.0",
        "format_version": ridx_format::CURRENT_FORMAT_VERSION,
        "extractor_version": ridx_format::EXTRACTOR_VERSION,
        "active_variant_id": "var1",
        "variant_dir": "variants/var1",
        "workspace_root": test_dir.display().to_string(),
        "manifest_path": test_dir.join("Cargo.toml").display().to_string(),
        "profile": "dev",
        "dependency_shards": [
            {
                "name": "a",
                "version": "1.0.0",
                "unit_key": "unitA1",
                "object_path": dep_a1_path.display().to_string(),
                "source_root": "/registry/a-1.0.0",
                "is_dependency": true
            },
            {
                "name": "a",
                "version": "2.0.0",
                "unit_key": "unitA2",
                "object_path": dep_a2_path.display().to_string(),
                "source_root": "/registry/a-2.0.0",
                "is_dependency": true
            }
        ]
    });
    fs::write(
        test_dir.join("ridx_manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let ws = WorkspaceIndex::open(&test_dir).expect("failed to open workspace");

    // Definition resolution at line 4 (pointing to A1 coords) must resolve to unitA1
    let def_line4 = ws
        .find_definition_at("src/main.rs", 4, 6)
        .expect("should resolve line 4");
    assert_eq!(def_line4.unit_key, Some("unitA1"));
    assert_eq!(def_line4.start_line, 10);

    // Definition resolution at line 8 (pointing to A2 coords) must resolve to unitA2
    let def_line8 = ws
        .find_definition_at("src/main.rs", 8, 6)
        .expect("should resolve line 8");
    assert_eq!(def_line8.unit_key, Some("unitA2"));
    assert_eq!(def_line8.start_line, 20);

    // Querying references for unitA1 must only return line 4 (not line 8)
    let refs_a1 = ws.find_references_for_unit("a::Thing", Some("unitA1"));
    assert_eq!(refs_a1.len(), 1, "expected only line 4 for unitA1");
    assert_eq!(refs_a1[0].span.start_line(), 4);

    // Querying references for unitA2 must only return line 8 (not line 4)
    let refs_a2 = ws.find_references_for_unit("a::Thing", Some("unitA2"));
    assert_eq!(refs_a2.len(), 1, "expected only line 8 for unitA2");
    assert_eq!(refs_a2[0].span.start_line(), 8);

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_workspace_relative_path_not_rebased_to_member_twice() {
    let test_dir = std::env::temp_dir().join(format!("ridx_rebase_test_{}", std::process::id()));
    let _ = fs::remove_dir_all(&test_dir);
    let member_dir = test_dir.join("crates/app");
    fs::create_dir_all(member_dir.join("src")).unwrap();
    let src_file = member_dir.join("src/lib.rs");
    fs::write(&src_file, "pub fn hello() {}\n").unwrap();

    let var_dir = test_dir.join("variants/var1");
    let deps_dir = var_dir.join(".deps");
    fs::create_dir_all(&deps_dir).unwrap();

    let mut writer = IndexWriter::new("app", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "hello".to_string(),
        path: "app::hello".to_string(),
        signature: Some("fn()".to_string()),
        file: "crates/app/src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 17,
    });
    let app_ridx = var_dir.join("app.ridx");
    writer
        .write_to(&mut BufWriter::new(File::create(&app_ridx).unwrap()))
        .unwrap();

    let app_meta = serde_json::json!({
        "name": "app",
        "version": "0.1.0",
        "unit_key": "unitApp",
        "object_path": app_ridx.display().to_string(),
        "source_root": member_dir.display().to_string(),
        "is_dependency": false
    });
    fs::write(
        deps_dir.join("app.json"),
        serde_json::to_string_pretty(&app_meta).unwrap(),
    )
    .unwrap();

    let ws = WorkspaceIndex::open(&var_dir).expect("failed to open workspace");
    let symbols = ws.search_symbols("hello");
    assert_eq!(symbols.len(), 1);
    let resolved = symbols[0].resolved_path();

    // Must NOT be double rebased to crates/app/crates/app/src/lib.rs
    assert_eq!(resolved, src_file);
    assert!(resolved.exists(), "resolved path {:?} must exist", resolved);

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_local_metadata_not_loaded_as_second_shard() {
    let test_dir = std::env::temp_dir().join(format!("ridx_dedup_test_{}", std::process::id()));
    let _ = fs::remove_dir_all(&test_dir);
    let var_dir = test_dir.join("variants/var1");
    let deps_dir = var_dir.join(".deps");
    fs::create_dir_all(&deps_dir).unwrap();

    let mut writer = IndexWriter::new("app", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "main".to_string(),
        path: "app::main".to_string(),
        signature: Some("fn()".to_string()),
        file: "src/main.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 3,
        end_col: 2,
    });
    let app_ridx = var_dir.join("app.ridx");
    writer
        .write_to(&mut BufWriter::new(File::create(&app_ridx).unwrap()))
        .unwrap();

    let app_meta = serde_json::json!({
        "name": "app",
        "version": "0.1.0",
        "unit_key": "unitApp",
        "object_path": app_ridx.display().to_string(),
        "is_dependency": false
    });
    fs::write(
        deps_dir.join("app.json"),
        serde_json::to_string_pretty(&app_meta).unwrap(),
    )
    .unwrap();

    // Manifest erroneously includes the local shard in dependency_shards
    let manifest = serde_json::json!({
        "version": "2.0",
        "format_version": ridx_format::CURRENT_FORMAT_VERSION,
        "extractor_version": ridx_format::EXTRACTOR_VERSION,
        "active_variant_id": "var1",
        "variant_dir": "variants/var1",
        "workspace_root": test_dir.display().to_string(),
        "manifest_path": test_dir.join("Cargo.toml").display().to_string(),
        "profile": "dev",
        "dependency_shards": [
            {
                "name": "app",
                "version": "0.1.0",
                "unit_key": "unitApp",
                "object_path": app_ridx.display().to_string(),
                "is_dependency": false
            }
        ]
    });
    fs::write(
        test_dir.join("ridx_manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let ws = WorkspaceIndex::open(&test_dir).expect("failed to open workspace");
    let symbols = ws.search_symbols("main");
    // Must be exactly 1 symbol, not duplicated!
    assert_eq!(
        symbols.len(),
        1,
        "expected 1 symbol for main, got {}",
        symbols.len()
    );
    assert_eq!(
        ws.stats().total_crates,
        1,
        "expected 1 total crate, got {}",
        ws.stats().total_crates
    );

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_references_scoped_to_exact_dependency_version_with_same_coordinates() {
    let test_dir =
        std::env::temp_dir().join(format!("ridx_dep_coords_test_{}", std::process::id()));
    let _ = fs::remove_dir_all(&test_dir);
    let var_dir = test_dir.join("variants/var1");
    let deps_dir = var_dir.join(".deps");
    fs::create_dir_all(&deps_dir).unwrap();

    let cache_dir = test_dir.join("cache");
    let dep_a1_path = cache_dir.join("a1.ridx");
    let dep_a2_path = cache_dir.join("a2.ridx");
    fs::create_dir_all(&cache_dir).unwrap();

    // 1. Dependency shard A1: crate "a" v1.0.0, unitA1, defines a::Thing at line 10
    let mut writer_a1 = IndexWriter::new("a", "1.0.0");
    writer_a1.set_unit_key("unitA1");
    writer_a1.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "Thing".to_string(),
        path: "a::Thing".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 10,
        start_col: 1,
        end_line: 12,
        end_col: 2,
    });
    writer_a1
        .write_to(&mut BufWriter::new(File::create(&dep_a1_path).unwrap()))
        .unwrap();

    // 2. Dependency shard A2: crate "a" v2.0.0, unitA2, ALSO defines a::Thing at line 10 in src/lib.rs!
    let mut writer_a2 = IndexWriter::new("a", "2.0.0");
    writer_a2.set_unit_key("unitA2");
    writer_a2.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "Thing".to_string(),
        path: "a::Thing".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 10,
        start_col: 1,
        end_line: 12,
        end_col: 2,
    });
    writer_a2
        .write_to(&mut BufWriter::new(File::create(&dep_a2_path).unwrap()))
        .unwrap();

    // 3. Consumer crate C: references a::Thing at line 4 (target_unit = unitA1)
    // and a::Thing at line 8 (target_unit = unitA2)
    let mut writer_c = IndexWriter::new("c", "0.1.0");
    writer_c.set_unit_key("unitC");
    writer_c.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 4,
        start_col: 5,
        end_line: 4,
        end_col: 13,
        target_def_id: None,
        target_path: "a::Thing".to_string(),
        target_unit: Some("unitA1".to_string()),
        target_file: Some("src/lib.rs".to_string()),
        target_line: 10,
        target_col: 1,
        kind: DefKind::Struct,
        is_write: false,
        is_macro_expansion: false,
    });
    writer_c.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 8,
        start_col: 5,
        end_line: 8,
        end_col: 13,
        target_def_id: None,
        target_path: "a::Thing".to_string(),
        target_unit: Some("unitA2".to_string()),
        target_file: Some("src/lib.rs".to_string()),
        target_line: 10,
        target_col: 1,
        kind: DefKind::Struct,
        is_write: false,
        is_macro_expansion: false,
    });
    let c_ridx = var_dir.join("c.ridx");
    writer_c
        .write_to(&mut BufWriter::new(File::create(&c_ridx).unwrap()))
        .unwrap();

    let c_meta = serde_json::json!({
        "name": "c",
        "version": "0.1.0",
        "unit_key": "unitC",
        "object_path": c_ridx.display().to_string(),
        "dependencies": ["unitA1", "unitA2"],
        "extern_units": {
            "a1": "unitA1",
            "a2": "unitA2"
        }
    });
    fs::write(
        deps_dir.join("c.json"),
        serde_json::to_string_pretty(&c_meta).unwrap(),
    )
    .unwrap();

    let manifest = serde_json::json!({
        "version": "2.0",
        "format_version": ridx_format::CURRENT_FORMAT_VERSION,
        "extractor_version": ridx_format::EXTRACTOR_VERSION,
        "active_variant_id": "var1",
        "variant_dir": "variants/var1",
        "workspace_root": test_dir.display().to_string(),
        "manifest_path": test_dir.join("Cargo.toml").display().to_string(),
        "profile": "dev",
        "dependency_shards": [
            {
                "name": "a",
                "version": "1.0.0",
                "unit_key": "unitA1",
                "object_path": dep_a1_path.display().to_string(),
                "source_root": "/registry/a-1.0.0",
                "is_dependency": true
            },
            {
                "name": "a",
                "version": "2.0.0",
                "unit_key": "unitA2",
                "object_path": dep_a2_path.display().to_string(),
                "source_root": "/registry/a-2.0.0",
                "is_dependency": true
            }
        ]
    });
    fs::write(
        test_dir.join("ridx_manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let ws = WorkspaceIndex::open(&test_dir).expect("failed to open workspace");

    // Querying references for unitA1 must ONLY return line 4 (not line 8)
    let refs_a1 = ws.find_references_for_unit("a::Thing", Some("unitA1"));
    assert_eq!(refs_a1.len(), 1, "expected only line 4 for unitA1");
    assert_eq!(refs_a1[0].span.start_line(), 4);

    // Querying references for unitA2 must ONLY return line 8 (not line 4)
    let refs_a2 = ws.find_references_for_unit("a::Thing", Some("unitA2"));
    assert_eq!(refs_a2.len(), 1, "expected only line 8 for unitA2");
    assert_eq!(refs_a2[0].span.start_line(), 8);

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_source_hash_for_file_does_not_match_unrelated_basename() {
    let test_dir = std::env::temp_dir().join(format!("ridx_basename_test_{}", std::process::id()));
    let _ = fs::remove_dir_all(&test_dir);
    let var_dir = test_dir.join("variants/var1");
    let deps_dir = var_dir.join(".deps");
    fs::create_dir_all(&deps_dir).unwrap();

    let root_foo = test_dir.join("crates/foo");
    let root_bar = test_dir.join("crates/bar");
    fs::create_dir_all(root_foo.join("src")).unwrap();
    fs::create_dir_all(root_bar.join("src")).unwrap();

    let file_foo = root_foo.join("src/lib.rs");
    let file_bar = root_bar.join("src/lib.rs");
    fs::write(&file_foo, "pub fn foo() {}\n").unwrap();
    fs::write(&file_bar, "pub fn bar() {}\n").unwrap();

    let hash_foo = 111111u64;
    let hash_bar = 222222u64;

    let mut writer_foo = IndexWriter::new("foo", "0.1.0");
    writer_foo.set_unit_key("unitFoo");
    writer_foo.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "foo".to_string(),
        path: "foo::foo".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 15,
    });
    let foo_ridx = var_dir.join("foo.ridx");
    writer_foo
        .write_to(&mut BufWriter::new(File::create(&foo_ridx).unwrap()))
        .unwrap();

    let meta_foo = serde_json::json!({
        "name": "foo",
        "version": "0.1.0",
        "unit_key": "unitFoo",
        "object_path": foo_ridx.display().to_string(),
        "source_root": root_foo.display().to_string(),
        "file_hashes": {
            "src/lib.rs": hash_foo
        }
    });
    fs::write(
        deps_dir.join("foo.json"),
        serde_json::to_string_pretty(&meta_foo).unwrap(),
    )
    .unwrap();

    let mut writer_bar = IndexWriter::new("bar", "0.1.0");
    writer_bar.set_unit_key("unitBar");
    writer_bar.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "bar".to_string(),
        path: "bar::bar".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 15,
    });
    let bar_ridx = var_dir.join("bar.ridx");
    writer_bar
        .write_to(&mut BufWriter::new(File::create(&bar_ridx).unwrap()))
        .unwrap();

    let meta_bar = serde_json::json!({
        "name": "bar",
        "version": "0.1.0",
        "unit_key": "unitBar",
        "object_path": bar_ridx.display().to_string(),
        "source_root": root_bar.display().to_string(),
        "file_hashes": {
            "src/lib.rs": hash_bar
        }
    });
    fs::write(
        deps_dir.join("bar.json"),
        serde_json::to_string_pretty(&meta_bar).unwrap(),
    )
    .unwrap();

    let ws = WorkspaceIndex::open(&var_dir).expect("failed to open workspace");

    // foo's lib.rs must match hash_foo
    let res_foo = ws.source_hash_for_file(&file_foo.to_string_lossy());
    assert_eq!(res_foo, Some(hash_foo));

    // bar's lib.rs must match hash_bar
    let res_bar = ws.source_hash_for_file(&file_bar.to_string_lossy());
    assert_eq!(res_bar, Some(hash_bar));

    // An unrelated crate's lib.rs must NOT match either crate!
    let unrelated = test_dir.join("crates/unrelated/src/lib.rs");
    let res_unrelated = ws.source_hash_for_file(&unrelated.to_string_lossy());
    assert_eq!(
        res_unrelated, None,
        "unrelated path must not match via basename"
    );

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_stale_sidecar_unit_key_mismatch_rejected() {
    let test_dir =
        std::env::temp_dir().join(format!("ridx_stale_sidecar_test_{}", std::process::id()));
    let _ = fs::remove_dir_all(&test_dir);
    let deps_dir = test_dir.join(".deps");
    fs::create_dir_all(&deps_dir).unwrap();

    let mut writer = IndexWriter::new("app", "0.1.0");
    writer.set_unit_key("unitNew");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "foo".to_string(),
        path: "app::foo".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 15,
    });
    let ridx_file = test_dir.join("app.ridx");
    writer
        .write_to(&mut BufWriter::new(File::create(&ridx_file).unwrap()))
        .unwrap();

    // Sidecar has old unit_key ("unitOld")
    let meta = serde_json::json!({
        "name": "app",
        "version": "0.1.0",
        "unit_key": "unitOld",
        "object_path": ridx_file.display().to_string(),
        "file_hashes": {
            "src/lib.rs": 999999u64
        }
    });
    fs::write(
        deps_dir.join("app.json"),
        serde_json::to_string_pretty(&meta).unwrap(),
    )
    .unwrap();

    let ws = WorkspaceIndex::open(&ridx_file).expect("failed to open single shard");
    // Since sidecar unit_key ("unitOld") mismatches index unit_key ("unitNew"), sidecar metadata is discarded
    assert_eq!(ws.source_hash_for_file("src/lib.rs"), None);

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_normalized_source_hash_crlf_and_bom() {
    let lf_bytes = b"pub fn hello() {\n    println!(\"hello\");\n}\n";
    let crlf_bytes = b"pub fn hello() {\r\n    println!(\"hello\");\r\n}\r\n";
    let bom_crlf_bytes = b"\xEF\xBB\xBFpub fn hello() {\r\n    println!(\"hello\");\r\n}\r\n";

    let hash_lf = ridx_format::normalized_source_hash(lf_bytes);
    let hash_crlf = ridx_format::normalized_source_hash(crlf_bytes);
    let hash_bom_crlf = ridx_format::normalized_source_hash(bom_crlf_bytes);

    assert_eq!(
        hash_lf, hash_crlf,
        "CRLF and LF must produce identical normalized hash"
    );
    assert_eq!(
        hash_lf, hash_bom_crlf,
        "BOM+CRLF and LF must produce identical normalized hash"
    );
}

#[test]
fn test_single_shard_reload_refreshes_metadata_and_hashes() {
    let test_dir =
        std::env::temp_dir().join(format!("ridx_single_reload_test_{}", std::process::id()));
    let _ = fs::remove_dir_all(&test_dir);
    let deps_dir = test_dir.join(".deps");
    fs::create_dir_all(&deps_dir).unwrap();

    let mut writer = IndexWriter::new("app", "0.1.0");
    writer.set_unit_key("unitApp");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "foo".to_string(),
        path: "app::foo".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 15,
    });
    let ridx_file = test_dir.join("app.ridx");
    writer
        .write_to(&mut BufWriter::new(File::create(&ridx_file).unwrap()))
        .unwrap();

    let meta = serde_json::json!({
        "name": "app",
        "version": "0.1.0",
        "unit_key": "unitApp",
        "object_path": ridx_file.display().to_string(),
        "file_hashes": {
            "src/lib.rs": 100u64
        }
    });
    fs::write(
        deps_dir.join("app.json"),
        serde_json::to_string_pretty(&meta).unwrap(),
    )
    .unwrap();

    let mut ws = WorkspaceIndex::open(&ridx_file).expect("failed to open single shard");
    assert_eq!(ws.source_hash_for_file("src/lib.rs"), Some(100));

    // Update metadata and touch .ridx
    std::thread::sleep(std::time::Duration::from_millis(50));
    let meta_updated = serde_json::json!({
        "name": "app",
        "version": "0.1.0",
        "unit_key": "unitApp",
        "object_path": ridx_file.display().to_string(),
        "file_hashes": {
            "src/lib.rs": 200u64
        }
    });
    fs::write(
        deps_dir.join("app.json"),
        serde_json::to_string_pretty(&meta_updated).unwrap(),
    )
    .unwrap();
    // Re-write ridx_file with later mtime
    let mut writer2 = IndexWriter::new("app", "0.1.0");
    writer2.set_unit_key("unitApp");
    writer2.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "foo".to_string(),
        path: "app::foo".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 15,
    });
    writer2
        .write_to(&mut BufWriter::new(File::create(&ridx_file).unwrap()))
        .unwrap();

    let reloaded = ws.reload_if_modified().expect("reload failed");
    assert!(reloaded, "expected reload_if_modified to return true");
    assert_eq!(ws.source_hash_for_file("src/lib.rs"), Some(200));

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_null_local_source_root_resolves_via_workspace_root() {
    let test_dir =
        std::env::temp_dir().join(format!("ridx_ws_root_res_test_{}", std::process::id()));
    let _ = fs::remove_dir_all(&test_dir);
    let var_dir = test_dir.join("variants/var1");
    let deps_dir = var_dir.join(".deps");
    fs::create_dir_all(&deps_dir).unwrap();

    let mut writer = IndexWriter::new("app", "0.1.0");
    writer.set_unit_key("unitApp");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "foo".to_string(),
        path: "app::foo".to_string(),
        signature: None,
        file: "crates/app/src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 15,
    });
    let app_ridx = var_dir.join("app.ridx");
    writer
        .write_to(&mut BufWriter::new(File::create(&app_ridx).unwrap()))
        .unwrap();

    let manifest = serde_json::json!({
        "version": "2.0",
        "format_version": ridx_format::CURRENT_FORMAT_VERSION,
        "extractor_version": ridx_format::EXTRACTOR_VERSION,
        "active_variant_id": "var1",
        "variant_dir": "variants/var1",
        "workspace_root": test_dir.display().to_string(),
        "manifest_path": test_dir.join("Cargo.toml").display().to_string(),
        "profile": "dev"
    });
    fs::write(
        test_dir.join("ridx_manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let ws = WorkspaceIndex::open(&test_dir).expect("failed to open workspace");
    let def = ws
        .find_definition_by_path("app::foo")
        .expect("should find definition");

    let resolved = def.resolved_path();
    assert_eq!(resolved, test_dir.join("crates/app/src/lib.rs"));

    let _ = fs::remove_dir_all(&test_dir);
}
