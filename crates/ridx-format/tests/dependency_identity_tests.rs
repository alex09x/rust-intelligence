mod common;

use common::unique_temp_dir;
use ridx_format::{DefKind, IndexWriter, RawDefinition, RawReference, WorkspaceIndex};
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::PathBuf;

#[test]
fn test_dependency_shard_loading() {
    let test_dir = unique_temp_dir("ridx_dep_test");
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
    let test_dir = unique_temp_dir("ridx_dep_unit_test");
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
    let test_dir = unique_temp_dir("ridx_exact_unit");
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
    let test_dir = unique_temp_dir("ridx_aliased_dep");
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

    // 3. Workspace consumer crate with aliased extern names:
    // dep_old -> unitA1, dep_new -> unitA2
    let mut writer_consumer = IndexWriter::new("consumer", "0.1.0");
    writer_consumer.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 3,
        start_col: 5,
        end_line: 3,
        end_col: 17,
        target_def_id: None,
        target_path: "dep_old::foo".to_string(),
        target_unit: None,
        target_file: Some("src/lib.rs".to_string()),
        target_line: 10,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    writer_consumer.add_reference(RawReference {
        file: "src/main.rs".to_string(),
        start_line: 7,
        start_col: 5,
        end_line: 7,
        end_col: 17,
        target_def_id: None,
        target_path: "dep_new::foo".to_string(),
        target_unit: None,
        target_file: Some("src/lib.rs".to_string()),
        target_line: 20,
        target_col: 1,
        kind: DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    let consumer_ridx = var_dir.join("consumer.ridx");
    writer_consumer
        .write_to(&mut BufWriter::new(File::create(&consumer_ridx).unwrap()))
        .unwrap();

    let consumer_meta = serde_json::json!({
        "name": "consumer",
        "version": "0.1.0",
        "unit_key": "unitConsumer",
        "object_path": consumer_ridx.display().to_string(),
        "dependencies": ["unitA1", "unitA2"],
        "extern_units": {
            "dep_old": "unitA1",
            "dep_new": "unitA2"
        }
    });
    fs::write(
        deps_dir.join("consumer.json"),
        serde_json::to_string_pretty(&consumer_meta).unwrap(),
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

    // Definition via dep_old alias must resolve to unitA1 (v1.0.0 at line 10)
    let def_old = ws
        .find_definition_at("src/main.rs", 3, 6)
        .expect("should resolve dep_old::foo");
    assert_eq!(def_old.unit_key, Some("unitA1"));
    assert_eq!(def_old.start_line, 10);
    assert_eq!(def_old.signature, Some("fn() -> i32"));

    // Definition via dep_new alias must resolve to unitA2 (v2.0.0 at line 20)
    let def_new = ws
        .find_definition_at("src/main.rs", 7, 6)
        .expect("should resolve dep_new::foo");
    assert_eq!(def_new.unit_key, Some("unitA2"));
    assert_eq!(def_new.start_line, 20);
    assert_eq!(def_new.signature, Some("fn() -> String"));

    // References for unitA1 must find the aliased reference at line 3
    let refs_a1 = ws.find_references_for_unit("dep::foo", Some("unitA1"));
    assert_eq!(refs_a1.len(), 1);
    assert_eq!(refs_a1[0].span.start_line(), 3);

    // References for unitA2 must find the aliased reference at line 7
    let refs_a2 = ws.find_references_for_unit("dep::foo", Some("unitA2"));
    assert_eq!(refs_a2.len(), 1);
    assert_eq!(refs_a2[0].span.start_line(), 7);

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_consumer_with_dual_version_canonical_references() {
    let test_dir = unique_temp_dir("ridx_canonical_dual");
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
fn test_references_scoped_to_exact_dependency_version_with_same_coordinates() {
    let test_dir = unique_temp_dir("ridx_dep_coords_test");
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
fn test_same_named_consumer_definition_and_references() {
    let test_dir = unique_temp_dir("ridx_same_name");

    // Crate A1: unit_key = "A1", crate_name = "a"
    let mut writer_a1 = IndexWriter::new("a", "1.0.0");
    writer_a1.set_unit_key("A1");
    writer_a1.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        name: "Item".to_string(),
        path: "a::Item".to_string(),
        signature: None,
        file: "crates/a1/src/lib.rs".to_string(),
        start_line: 10,
        start_col: 1,
        end_line: 12,
        end_col: 2,
    });
    writer_a1
        .write_to(&mut BufWriter::new(
            File::create(test_dir.join("a1.ridx")).unwrap(),
        ))
        .unwrap();

    // Crate A2: unit_key = "A2", crate_name = "a", depends on A1!
    let mut writer_a2 = IndexWriter::new("a", "2.0.0");
    writer_a2.set_unit_key("A2");
    writer_a2.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "use_item".to_string(),
        path: "a::use_item".to_string(),
        signature: None,
        file: "crates/a2/src/lib.rs".to_string(),
        start_line: 5,
        start_col: 1,
        end_line: 8,
        end_col: 2,
    });
    writer_a2.add_reference(RawReference {
        target_def_id: None,
        target_path: "a::Item".to_string(),
        kind: DefKind::Struct,
        is_write: false,
        is_macro_expansion: false,
        file: "crates/a2/src/lib.rs".to_string(),
        start_line: 6,
        start_col: 10,
        end_line: 6,
        end_col: 14,
        target_unit: Some("A1".to_string()),
        target_file: Some("crates/a1/src/lib.rs".to_string()),
        target_line: 10,
        target_col: 0,
    });
    writer_a2
        .write_to(&mut BufWriter::new(
            File::create(test_dir.join("a2.ridx")).unwrap(),
        ))
        .unwrap();

    // Publish metadata for both
    let deps_dir = test_dir.join(".deps");
    fs::create_dir_all(&deps_dir).unwrap();
    let meta_a1 = serde_json::json!({
        "name": "a",
        "version": "1.0.0",
        "unit_key": "A1",
        "object_path": test_dir.join("a1.ridx").display().to_string(),
        "dependencies": [],
        "extern_units": {},
        "file_hashes": {},
    });
    fs::write(
        deps_dir.join("a1.json"),
        serde_json::to_string_pretty(&meta_a1).unwrap(),
    )
    .unwrap();

    let meta_a2 = serde_json::json!({
        "name": "a",
        "version": "2.0.0",
        "unit_key": "A2",
        "object_path": test_dir.join("a2.ridx").display().to_string(),
        "dependencies": ["A1"],
        "extern_units": {},
        "file_hashes": {},
    });
    fs::write(
        deps_dir.join("a2.json"),
        serde_json::to_string_pretty(&meta_a2).unwrap(),
    )
    .unwrap();

    let ws = WorkspaceIndex::open(&test_dir).expect("should open workspace");
    assert_eq!(ws.crate_count(), 2);

    // 1. Definition lookup at reference in A2 should resolve to A1, NOT be treated as local to A2
    let def = ws
        .find_definition_at("crates/a2/src/lib.rs", 6, 11)
        .expect("should resolve definition");
    assert_eq!(def.name, "Item");
    assert_eq!(def.unit_key, Some("A1"));
    assert_eq!(def.file, "crates/a1/src/lib.rs");

    // 2. References lookup for a::Item in A1 should find reference in consumer A2
    let refs = ws.find_references_for_unit("a::Item", Some("A1"));
    assert_eq!(
        refs.len(),
        1,
        "A2 should not be skipped as a same-named consumer"
    );
    assert_eq!(refs[0].file, "crates/a2/src/lib.rs");
    assert_eq!(refs[0].unit_key, Some("A2"));

    let _ = fs::remove_dir_all(&test_dir);
}
