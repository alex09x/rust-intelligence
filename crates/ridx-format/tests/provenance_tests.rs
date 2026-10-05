mod common;

use common::unique_temp_dir;
use ridx_format::{DefKind, IndexWriter, RawDefinition, WorkspaceIndex};
use std::fs::{self, File};
use std::io::BufWriter;

#[test]
fn test_source_hash_for_file_does_not_match_unrelated_basename() {
    let test_dir = unique_temp_dir("ridx_basename_test");
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
    let test_dir = unique_temp_dir("ridx_stale_sidecar_test");
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
fn test_sidecar_generation_mismatch_rejected() {
    let test_dir = unique_temp_dir("ridx_gen_mismatch");
    let _ = fs::remove_dir_all(&test_dir);
    fs::create_dir_all(&test_dir).unwrap();

    let mut writer = IndexWriter::new("mycrate", "0.1.0");
    writer.set_unit_key("mycrate_uk");
    writer.set_generation("0000000000001111");
    let mut hashes = std::collections::HashMap::new();
    hashes.insert("src/lib.rs".to_string(), 0x1111u64);
    writer.set_file_hashes(hashes);
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "foo".to_string(),
        path: "mycrate::foo".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 10,
    });

    let index_path = test_dir.join("mycrate.ridx");
    writer
        .write_to(&mut BufWriter::new(File::create(&index_path).unwrap()))
        .unwrap();

    // Sidecar has mismatched generation "0000000000002222"
    let deps_dir = test_dir.join(".deps");
    fs::create_dir_all(&deps_dir).unwrap();
    let sidecar_info = serde_json::json!({
        "name": "mycrate",
        "version": "0.1.0",
        "unit_key": "mycrate_uk",
        "generation": "0000000000002222",
        "is_dependency": false,
        "dependencies": ["some_dep"],
        "extern_units": {},
        "file_hashes": {"src/lib.rs": 0x2222u64},
    });
    fs::write(
        deps_dir.join("mycrate.json"),
        serde_json::to_string_pretty(&sidecar_info).unwrap(),
    )
    .unwrap();

    let ws = WorkspaceIndex::open(&test_dir).expect("should open index");
    // Embedded hashes from index binary are retained!
    assert_eq!(ws.source_hash_for_file("src/lib.rs"), Some(0x1111));
    assert_eq!(ws.generation_at(0), Some("0000000000001111"));

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_dependency_manifest_cannot_override_embedded_generation() {
    let test_dir = unique_temp_dir("ridx_test_dep_manifest_gen");
    let _ = fs::remove_dir_all(&test_dir);

    let var_dir = test_dir.join("variants/var1");
    fs::create_dir_all(&var_dir).unwrap();

    // 1. Write workspace crate
    let mut writer_app = IndexWriter::new("app", "0.1.0");
    writer_app.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "main".to_string(),
        path: "app::main".to_string(),
        signature: None,
        file: "src/main.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 10,
    });
    writer_app
        .write_to(&mut BufWriter::new(
            File::create(var_dir.join("app.ridx")).unwrap(),
        ))
        .unwrap();

    // 2. Write dependency crate A1 with generation "new-generation" and embedded hash 111
    let dep_dir = test_dir.join("registry/a1");
    fs::create_dir_all(&dep_dir).unwrap();
    let mut writer_a1 = IndexWriter::new("a1", "0.1.0");
    writer_a1.set_unit_key("unitA1");
    writer_a1.set_generation("new-generation");
    let mut file_hashes = std::collections::HashMap::new();
    file_hashes.insert("src/lib.rs".to_string(), 111u64);
    writer_a1.set_file_hashes(file_hashes);
    writer_a1.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "dep_fn".to_string(),
        path: "a1::dep_fn".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 10,
    });
    let a1_ridx = dep_dir.join("a1.ridx");
    writer_a1
        .write_to(&mut BufWriter::new(File::create(&a1_ridx).unwrap()))
        .unwrap();

    // Verify reader reads 111
    let reader = ridx_format::IndexReader::open(&a1_ridx).unwrap();
    assert_eq!(reader.generation(), "new-generation");
    assert_eq!(reader.file_source_hash(0), 111);

    // 3. Write ridx_manifest.json with stale dependency shard (generation: "old-generation", hash: 222)
    let manifest_json = serde_json::json!({
        "version": "2.0",
        "format_version": ridx_format::CURRENT_FORMAT_VERSION,
        "extractor_version": ridx_format::EXTRACTOR_VERSION,
        "active_variant_id": "var1",
        "variant_dir": "variants/var1",
        "workspace_root": test_dir.display().to_string(),
        "manifest_path": test_dir.join("Cargo.toml").display().to_string(),
        "profile": "dev",
        "timestamp_epoch_secs": 1000,
        "total_crates": 1,
        "total_defs": 1,
        "total_refs": 0,
        "total_types": 0,
        "dependency_shards": [{
            "name": "a1",
            "version": "0.1.0",
            "unit_key": "unitA1",
            "generation": "old-generation",
            "object_path": a1_ridx.display().to_string(),
            "source_root": dep_dir.display().to_string(),
            "is_dependency": true,
            "dependencies": [],
            "extern_units": {},
            "file_hashes": {
                "src/lib.rs": 222
            }
        }]
    });
    fs::write(
        test_dir.join("ridx_manifest.json"),
        serde_json::to_string_pretty(&manifest_json).unwrap(),
    )
    .unwrap();

    let ws = WorkspaceIndex::open(&test_dir).expect("failed to open workspace");

    // The embedded hash 111 must be preserved and authoritative! The stale manifest hash 222 must NOT override it!
    let a1_file = dep_dir.join("src/lib.rs");
    let res = ws.source_hash_for_file(&a1_file.to_string_lossy());
    assert_eq!(
        res,
        Some(111),
        "embedded hash 111 must be authoritative over stale manifest hash 222"
    );

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_absolute_source_alias_cannot_shadow_authoritative_embedded_hash() {
    let test_dir = unique_temp_dir("ridx_alias_shadow_test");
    let _ = fs::remove_dir_all(&test_dir);

    let var_dir = test_dir.join("variants/var1");
    fs::create_dir_all(&var_dir).unwrap();

    // 1. Write workspace crate
    let mut writer_app = IndexWriter::new("app", "0.1.0");
    writer_app.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "main".to_string(),
        path: "app::main".to_string(),
        signature: None,
        file: "src/main.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 10,
    });
    writer_app
        .write_to(&mut BufWriter::new(
            File::create(var_dir.join("app.ridx")).unwrap(),
        ))
        .unwrap();

    // 2. Write dependency crate A1 with generation computed from embedded hash 111
    let dep_dir = test_dir.join("registry/a1");
    fs::create_dir_all(&dep_dir).unwrap();
    let mut writer_a1 = IndexWriter::new("a1", "0.1.0");
    writer_a1.set_unit_key("unitA1");
    let mut embedded_hashes = std::collections::HashMap::new();
    embedded_hashes.insert("src/lib.rs".to_string(), 111u64);
    let digest = ridx_format::compute_source_snapshot_digest(&embedded_hashes);
    let bin_gen = format!("{:016x}", digest);
    writer_a1.set_generation(&bin_gen);
    writer_a1.set_file_hashes(embedded_hashes);
    writer_a1.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "dep_fn".to_string(),
        path: "a1::dep_fn".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 10,
    });
    let a1_ridx = dep_dir.join("a1.ridx");
    writer_a1
        .write_to(&mut BufWriter::new(File::create(&a1_ridx).unwrap()))
        .unwrap();

    // 3. Write ridx_manifest.json with matching binary generation, but containing a stale absolute alias 222
    let abs_lib_rs = dep_dir.join("src/lib.rs").display().to_string();
    let manifest_json = serde_json::json!({
        "version": "2.0",
        "format_version": ridx_format::CURRENT_FORMAT_VERSION,
        "extractor_version": ridx_format::EXTRACTOR_VERSION,
        "active_variant_id": "var1",
        "variant_dir": "variants/var1",
        "workspace_root": test_dir.display().to_string(),
        "manifest_path": test_dir.join("Cargo.toml").display().to_string(),
        "profile": "dev",
        "timestamp_epoch_secs": 1000,
        "total_crates": 1,
        "total_defs": 1,
        "total_refs": 0,
        "total_types": 0,
        "dependency_shards": [{
            "name": "a1",
            "version": "0.1.0",
            "unit_key": "unitA1",
            "generation": bin_gen,
            "object_path": a1_ridx.display().to_string(),
            "source_root": dep_dir.display().to_string(),
            "is_dependency": true,
            "dependencies": [],
            "extern_units": {},
            "file_hashes": {
                "src/lib.rs": 111,
                abs_lib_rs: 222
            }
        }]
    });
    fs::write(
        test_dir.join("ridx_manifest.json"),
        serde_json::to_string_pretty(&manifest_json).unwrap(),
    )
    .unwrap();

    let ws = WorkspaceIndex::open(&test_dir).expect("failed to open workspace");

    // The embedded hash 111 MUST NOT be shadowed by the stale absolute alias 222!
    let a1_file = dep_dir.join("src/lib.rs");
    let res = ws.source_hash_for_file(&a1_file.to_string_lossy());
    assert_eq!(
        res,
        Some(111),
        "authoritative embedded hash 111 must take precedence over stale absolute alias 222"
    );

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_current_workspace_exact_source_beats_suffix_match() {
    let test_dir = unique_temp_dir("ridx_exact_source_test");
    let _ = fs::remove_dir_all(&test_dir);
    let ws_root = test_dir.join("workspace");
    let var_dir = ws_root.join("target/ridx/variants/var1");
    fs::create_dir_all(&var_dir).unwrap();

    // 1. Shard 00_member.ridx embeds /workspace/crates/member/src/lib.rs -> 111
    let member_file = ws_root.join("crates/member/src/lib.rs");
    let mut writer_member = IndexWriter::new("member", "0.1.0");
    writer_member.set_unit_key("unitMember");
    let mut member_hashes = std::collections::HashMap::new();
    member_hashes.insert(member_file.display().to_string(), 111u64);
    writer_member.set_file_hashes(member_hashes);
    writer_member.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "member_fn".to_string(),
        path: "member::member_fn".to_string(),
        signature: None,
        file: member_file.display().to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 10,
    });
    let member_ridx = var_dir.join("00_member.ridx");
    writer_member
        .write_to(&mut BufWriter::new(File::create(&member_ridx).unwrap()))
        .unwrap();

    // 2. Shard app.ridx embeds /workspace/src/lib.rs -> 222
    let root_file = ws_root.join("src/lib.rs");
    let mut writer_app = IndexWriter::new("app", "0.1.0");
    writer_app.set_unit_key("unitApp");
    let mut app_hashes = std::collections::HashMap::new();
    app_hashes.insert(root_file.display().to_string(), 222u64);
    writer_app.set_file_hashes(app_hashes);
    writer_app.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "app_fn".to_string(),
        path: "app::app_fn".to_string(),
        signature: None,
        file: root_file.display().to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 10,
    });
    let app_ridx = var_dir.join("app.ridx");
    writer_app
        .write_to(&mut BufWriter::new(File::create(&app_ridx).unwrap()))
        .unwrap();

    // 3. Manifest recording workspace_root
    let manifest_json = serde_json::json!({
        "version": "2.0",
        "format_version": ridx_format::CURRENT_FORMAT_VERSION,
        "extractor_version": ridx_format::EXTRACTOR_VERSION,
        "active_variant_id": "var1",
        "variant_dir": "variants/var1",
        "workspace_root": ws_root.display().to_string(),
        "manifest_path": ws_root.join("Cargo.toml").display().to_string(),
        "profile": "dev",
        "timestamp_epoch_secs": 1000,
        "total_crates": 2,
        "total_defs": 2,
        "total_refs": 0,
        "total_types": 0,
        "dependency_shards": []
    });
    fs::write(
        ws_root.join("target/ridx/ridx_manifest.json"),
        serde_json::to_string_pretty(&manifest_json).unwrap(),
    )
    .unwrap();

    let ws = WorkspaceIndex::open(&ws_root).expect("failed to open workspace");

    // Root file must return 222 (exact match), not 111 (suffix match on member's lib.rs)
    let res_root = ws.source_hash_for_file(&root_file.display().to_string());
    assert_eq!(
        res_root,
        Some(222),
        "exact root file query must not match member's lib.rs via suffix"
    );

    // Member file must return 111
    let res_member = ws.source_hash_for_file(&member_file.display().to_string());
    assert_eq!(res_member, Some(111));

    // Relative root file query must also match root file 222
    let res_rel_root = ws.source_hash_for_file("src/lib.rs");
    assert_eq!(res_rel_root, Some(222));

    // Relative member file query must match member file 111
    let res_rel_member = ws.source_hash_for_file("crates/member/src/lib.rs");
    assert_eq!(res_rel_member, Some(111));

    let _ = fs::remove_dir_all(&test_dir);
}
