/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use crate::common::unique_temp_dir;
use ridx_format::{DefKind, IndexWriter, RawDefinition, WorkspaceIndex};
use std::fs::{self, File};
use std::io::BufWriter;

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
    writer_ws.write_to(&mut BufWriter::new(File::create(&ws_ridx).unwrap())).unwrap();

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
    writer_ext.write_to(&mut BufWriter::new(File::create(&ext_ridx).unwrap())).unwrap();

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
    fs::write(ws_root.join("target/ridx/ridx_manifest.json"), serde_json::to_string_pretty(&manifest_json).unwrap()).unwrap();

    let ws = WorkspaceIndex::open(&ws_root).expect("failed to open workspace");

    let query_path = ws_root.join("shortcut/../file.rs");
    if let Ok(canon_query) = query_path.canonicalize() {
        assert_eq!(canon_query, ext_file.canonicalize().unwrap(), "query path must canonicalize to external/file.rs");

        // Absolute query through symlink/.. must resolve to ext_file (222), not ws_file (111)
        let hash = ws.source_hash_for_file(&query_path.display().to_string());
        assert_eq!(hash, Some(222), "source_hash_for_file must resolve filesystem identity across symlink/..");

        let mtime = ws.shard_mtime_for_file(&query_path.display().to_string());
        assert!(mtime.is_some(), "shard_mtime_for_file must resolve filesystem identity across symlink/..");

        // Relative query through symlink/..
        let rel_hash = ws.source_hash_for_file("shortcut/../file.rs");
        assert_eq!(rel_hash, Some(222), "relative query through symlink/.. must match 222");

        let rel_mtime = ws.shard_mtime_for_file("shortcut/../file.rs");
        assert!(rel_mtime.is_some(), "relative query through symlink/.. must return mtime");
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
    writer_dep.write_to(&mut BufWriter::new(File::create(&dep_ridx).unwrap())).unwrap();

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
    writer_ws.write_to(&mut BufWriter::new(File::create(&ws_ridx).unwrap())).unwrap();

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
    fs::write(ws_root.join("target/ridx/ridx_manifest.json"), serde_json::to_string_pretty(&manifest_json).unwrap()).unwrap();

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

    assert_eq!(res_rel, Some(222), "relative query must anchor to workspace_root (222) and not process cwd (111)");
    assert!(res_rel_mtime.is_some(), "shard_mtime_for_file must return mtime of selected workspace shard");

    // Absolute query for ws_file must also return 222
    let res_abs = ws.source_hash_for_file(&ws_file.display().to_string());
    assert_eq!(res_abs, Some(222));

    let _ = fs::remove_dir_all(&test_dir);
}
