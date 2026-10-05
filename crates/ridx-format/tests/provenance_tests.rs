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

#[test]
fn test_symlink_parent_path_uses_filesystem_identity() {
    let test_dir = unique_temp_dir("ridx_symlink_parent_test");
    let _ = fs::remove_dir_all(&test_dir);
    let ws_root = test_dir.join("workspace");
    let external_dir = ws_root.join("external");
    let nested_dir = external_dir.join("nested");
    fs::create_dir_all(&nested_dir).unwrap();

    let ws_file = ws_root.join("file.rs");
    fs::write(&ws_file, "pub fn ws() {}\n").unwrap();

    let ext_file = external_dir.join("file.rs");
    fs::write(&ext_file, "pub fn ext() {}\n").unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        symlink(&nested_dir, ws_root.join("shortcut")).unwrap();
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::symlink_dir;
        let _ = symlink_dir(&nested_dir, ws_root.join("shortcut"));
    }

    let var_dir = ws_root.join("target/ridx/variants/var1");
    fs::create_dir_all(&var_dir).unwrap();

    // 1. Shard 00_ws.ridx embeds workspace/file.rs -> 111
    let mut writer_ws = IndexWriter::new("ws_crate", "0.1.0");
    writer_ws.set_unit_key("unitWs");
    let mut ws_hashes = std::collections::HashMap::new();
    ws_hashes.insert(ws_file.display().to_string(), 111u64);
    writer_ws.set_file_hashes(ws_hashes);
    writer_ws.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "ws_fn".to_string(),
        path: "ws_crate::ws_fn".to_string(),
        signature: None,
        file: ws_file.display().to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 10,
    });
    let ws_ridx = var_dir.join("00_ws.ridx");
    writer_ws
        .write_to(&mut BufWriter::new(File::create(&ws_ridx).unwrap()))
        .unwrap();

    // 2. Shard 01_ext.ridx embeds workspace/external/file.rs -> 222
    let mut writer_ext = IndexWriter::new("ext_crate", "0.1.0");
    writer_ext.set_unit_key("unitExt");
    let mut ext_hashes = std::collections::HashMap::new();
    ext_hashes.insert(ext_file.display().to_string(), 222u64);
    writer_ext.set_file_hashes(ext_hashes);
    writer_ext.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "ext_fn".to_string(),
        path: "ext_crate::ext_fn".to_string(),
        signature: None,
        file: ext_file.display().to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 10,
    });
    let ext_ridx = var_dir.join("01_ext.ridx");
    writer_ext
        .write_to(&mut BufWriter::new(File::create(&ext_ridx).unwrap()))
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

    let query_path = ws_root.join("shortcut/../file.rs");
    if let Ok(canon_query) = query_path.canonicalize() {
        assert_eq!(
            canon_query,
            ext_file.canonicalize().unwrap(),
            "query path must canonicalize to external/file.rs"
        );

        // Absolute query through symlink/.. must resolve to ext_file (222), not ws_file (111)
        let hash = ws.source_hash_for_file(&query_path.display().to_string());
        assert_eq!(
            hash,
            Some(222),
            "source_hash_for_file must resolve filesystem identity across symlink/.."
        );

        let mtime = ws.shard_mtime_for_file(&query_path.display().to_string());
        assert!(
            mtime.is_some(),
            "shard_mtime_for_file must resolve filesystem identity across symlink/.."
        );

        // Relative query through symlink/..
        let rel_hash = ws.source_hash_for_file("shortcut/../file.rs");
        assert_eq!(
            rel_hash,
            Some(222),
            "relative query through symlink/.. must match 222"
        );

        let rel_mtime = ws.shard_mtime_for_file("shortcut/../file.rs");
        assert!(
            rel_mtime.is_some(),
            "relative query through symlink/.. must return mtime"
        );
    }

    // Direct queries
    let hash_direct = ws.source_hash_for_file(&ext_file.display().to_string());
    assert_eq!(hash_direct, Some(222));

    let hash_ws = ws.source_hash_for_file(&ws_file.display().to_string());
    assert_eq!(hash_ws, Some(111));

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_relative_query_is_anchored_to_selected_workspace() {
    let test_dir = unique_temp_dir("ridx_rel_anchor_test");
    let _ = fs::remove_dir_all(&test_dir);
    let ws_root = test_dir.join("selected_workspace");
    let dep_cwd = test_dir.join("dep_cwd");
    fs::create_dir_all(ws_root.join("src")).unwrap();
    fs::create_dir_all(dep_cwd.join("src")).unwrap();

    let ws_file = ws_root.join("src/lib.rs");
    fs::write(&ws_file, "pub fn ws() {}\n").unwrap();

    let dep_file = dep_cwd.join("src/lib.rs");
    fs::write(&dep_file, "pub fn dep() {}\n").unwrap();

    let var_dir = ws_root.join("target/ridx/variants/var1");
    fs::create_dir_all(&var_dir).unwrap();

    // 1. Shard 00_dep.ridx embeds <dep_cwd>/src/lib.rs -> 111 (path dependency)
    let mut writer_dep = IndexWriter::new("dep_crate", "0.1.0");
    writer_dep.set_unit_key("unitDep");
    let mut dep_hashes = std::collections::HashMap::new();
    dep_hashes.insert(dep_file.display().to_string(), 111u64);
    writer_dep.set_file_hashes(dep_hashes);
    writer_dep.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "dep_fn".to_string(),
        path: "dep_crate::dep_fn".to_string(),
        signature: None,
        file: dep_file.display().to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 10,
    });
    let dep_ridx = var_dir.join("00_dep.ridx");
    writer_dep
        .write_to(&mut BufWriter::new(File::create(&dep_ridx).unwrap()))
        .unwrap();

    // 2. Shard 01_ws.ridx embeds <ws_root>/src/lib.rs -> 222 (selected workspace)
    let mut writer_ws = IndexWriter::new("ws_crate", "0.1.0");
    writer_ws.set_unit_key("unitWs");
    let mut ws_hashes = std::collections::HashMap::new();
    ws_hashes.insert(ws_file.display().to_string(), 222u64);
    writer_ws.set_file_hashes(ws_hashes);
    writer_ws.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "ws_fn".to_string(),
        path: "ws_crate::ws_fn".to_string(),
        signature: None,
        file: ws_file.display().to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 10,
    });
    let ws_ridx = var_dir.join("01_ws.ridx");
    writer_ws
        .write_to(&mut BufWriter::new(File::create(&ws_ridx).unwrap()))
        .unwrap();

    // 3. Manifest recording workspace_root and dependency shards
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
        "dependency_shards": [
            {
                "name": "dep_crate",
                "unit_key": "unitDep",
                "object_path": dep_ridx.display().to_string(),
                "source_root": dep_cwd.display().to_string(),
                "dependencies": [],
                "file_hashes": {
                    dep_file.display().to_string(): 111u64
                }
            }
        ]
    });
    fs::write(
        ws_root.join("target/ridx/ridx_manifest.json"),
        serde_json::to_string_pretty(&manifest_json).unwrap(),
    )
    .unwrap();

    let ws = WorkspaceIndex::open(&ws_root).expect("failed to open workspace");

    struct CwdGuard(Option<std::path::PathBuf>);
    impl Drop for CwdGuard {
        fn drop(&mut self) {
            if let Some(ref orig) = self.0 {
                let _ = std::env::set_current_dir(orig);
            }
        }
    }

    // Temporarily switch cwd to dep_cwd to simulate launching from a path dependency directory
    let guard = CwdGuard(std::env::current_dir().ok());
    let _ = std::env::set_current_dir(&dep_cwd);

    // Relative query "src/lib.rs" must be anchored to ws_root (222), NOT dep_cwd (111)
    let res_rel = ws.source_hash_for_file("src/lib.rs");
    let res_rel_mtime = ws.shard_mtime_for_file("src/lib.rs");

    drop(guard);

    assert_eq!(
        res_rel,
        Some(222),
        "relative query must anchor to workspace_root (222) and not process cwd (111)"
    );
    assert!(
        res_rel_mtime.is_some(),
        "shard_mtime_for_file must return mtime of selected workspace shard"
    );

    // Absolute query for ws_file must also return 222
    let res_abs = ws.source_hash_for_file(&ws_file.display().to_string());
    assert_eq!(res_abs, Some(222));

    let _ = fs::remove_dir_all(&test_dir);
}
