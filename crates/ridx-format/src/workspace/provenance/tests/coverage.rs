/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::super::*;
use super::*;
use crate::workspace::DependencyShardInfo;

#[test]
fn test_provenance_additional_coverage() {
    // Line 142: both entry and req are relative and match
    assert!(file_entry_matches("src/lib.rs", None, Path::new("src/lib.rs"), None));

    // Line 127: path traverses symlink parent
    let temp = new_test_dir("prov_cov_symlink");
    let real_dir = temp.0.join("real");
    fs::create_dir_all(&real_dir).unwrap();
    let symlink_dir = temp.0.join("link");
    #[cfg(unix)]
    {
        let _ = std::os::unix::fs::symlink(&real_dir, &symlink_dir);
        let bad_path = symlink_dir.join("../escape.rs");
        assert!(!file_entry_matches("src/lib.rs", None, &bad_path, None));
    }

    // Test shard_mtime_for_file and source_hash_for_file with absolute path matching dep source_root (lines 209, 327)
    let dep_dir = new_test_dir("prov_cov_dep");
    let dep_src = dep_dir.0.join("src");
    fs::create_dir_all(&dep_src).unwrap();
    let dep_file = dep_src.join("lib.rs");
    fs::write(&dep_file, b"fn dep() {}").unwrap();

    let idx_file = dep_dir.0.join("dep.ridx");
    let mut w = IndexWriter::new("prov_dep", "0.1.0");
    w.set_unit_key("prov_dep_key");
    w.get_or_insert_file("");
    w.get_or_insert_file("src/lib.rs");
    let mut hashes = HashMap::new();
    hashes.insert("src/lib.rs".to_string(), 77777u64);
    w.set_file_hashes(hashes);
    let mut f = File::create(&idx_file).unwrap();
    w.write_to(&mut f).unwrap();
    drop(f);

    let mut ws = WorkspaceIndex::open(&idx_file).unwrap();
    ws.source_roots = vec![Some(dep_dir.0.clone())];
    ws.reader_is_dependency = vec![true];

    // Absolute query matching dep_dir (lines 209, 327)
    let mtime = ws.shard_mtime_for_file(&dep_file.to_string_lossy());
    assert!(mtime.is_some());
    let hash = ws.source_hash_for_file(&dep_file.to_string_lossy());
    assert_eq!(hash, Some(77777u64));

    // Sidecar only file for shard_mtime_for_file (lines 265, 270)
    let mut sidecar_map = HashMap::new();
    sidecar_map.insert("src/sidecar_only.rs".to_string(), 88888u64);
    ws.reader_file_hashes = vec![sidecar_map];
    let sidecar_mtime = ws.shard_mtime_for_file("src/sidecar_only.rs");
    assert!(sidecar_mtime.is_some());

    // Relative path joined with workspace_root matching embedded table (line 366)
    ws.workspace_root = Some(dep_dir.0.clone());
    let ws_abs_hash = ws.source_hash_for_file("src/lib.rs");
    assert_eq!(ws_abs_hash, Some(77777u64));

    // Direct file_entry_matches edge cases (lines 142, 154, 156, 158)
    assert!(file_entry_matches("./src/lib.rs", None, Path::new("src/lib.rs"), None));
    assert!(file_entry_matches(&dep_file.to_string_lossy(), Some(&dep_dir.0), Path::new("src/lib.rs"), None));

    // Relative query with workspace_root set matching norm_root (lines 198-199, 316-317)
    let mtime_ws = ws.shard_mtime_for_file("src/lib.rs");
    assert!(mtime_ws.is_some());
    let hash_ws = ws.source_hash_for_file("src/lib.rs");
    assert_eq!(hash_ws, Some(77777u64));

    // Symlinked entry_str for file_entry_matches (lines 154-156)
    #[cfg(unix)]
    {
        let symlink_dep = dep_dir.0.join("symlink_dep");
        if std::os::unix::fs::symlink(&dep_dir.0, &symlink_dep).is_ok() {
            let symlink_file = symlink_dep.join("src/lib.rs");
            assert!(file_entry_matches(&symlink_file.to_string_lossy(), Some(&dep_dir.0), Path::new("src/lib.rs"), None));
        }
    }

    // load_file_metadata with mismatching computed generation (line 448) and invalid JSON (line 474)
    let meta_test_dir = new_test_dir("meta_test_cov");
    let deps_dir = meta_test_dir.0.join(".deps");
    fs::create_dir_all(&deps_dir).unwrap();

    // Invalid JSON file (line 474)
    fs::write(deps_dir.join("sample.json"), b"invalid json content").unwrap();
    let _ = WorkspaceIndex::load_file_metadata(&meta_test_dir.0.join("sample.ridx"), &meta_test_dir.0, "sample", "", "", &HashMap::new());

    // Valid JSON with mismatching computed generation (line 448)
    let mut fh = HashMap::new();
    fh.insert("src/lib.rs".to_string(), 11111u64);
    let dep_info_mismatch = DependencyShardInfo {
        name: "sample".to_string(),
        version: "0.1.0".to_string(),
        unit_key: "uk_sample".to_string(),
        generation: None,
        object_path: "".to_string(),
        source_root: None,
        target_kind: None,
        is_dependency: true,
        dependencies: vec![],
        extern_units: HashMap::new(),
        file_hashes: fh,
        stable_crate_id: None,
        caught_panics: None,
        compiler_args: vec![],
        env: HashMap::new(),
    };
    fs::write(deps_dir.join("sample.json"), serde_json::to_string(&dep_info_mismatch).unwrap()).unwrap();
    let _ = WorkspaceIndex::load_file_metadata(&meta_test_dir.0.join("sample.ridx"), &meta_test_dir.0, "sample", "uk_sample", "different_gen_than_computed", &HashMap::new());

    // ws_abs matching where direct relative query against reader origin fails (lines 247, 269, 366, 391)
    let ws_parent = new_test_dir("ws_abs_parent");
    let ws_sub = ws_parent.0.join("sub_pkg");
    let _ = fs::create_dir_all(ws_sub.join("src"));
    let abs_lib = ws_sub.join("src/virtual_lib.rs");

    let mut abs_writer = IndexWriter::new("sub_pkg", "0.1.0");
    let abs_lib_str = abs_lib.to_string_lossy().to_string();
    abs_writer.get_or_insert_file(&abs_lib_str);
    abs_writer.add_file_hash(&abs_lib_str, 99999u64);
    let abs_idx_file = ws_sub.join("sub_pkg.ridx");
    let mut f_abs = fs::File::create(&abs_idx_file).unwrap();
    abs_writer.write_to(&mut f_abs).unwrap();
    drop(f_abs);

    let abs_reader = crate::reader::IndexReader::open(&abs_idx_file).unwrap();
    let mut ws_abs_idx = WorkspaceIndex::open(&abs_idx_file).unwrap();
    ws_abs_idx.workspace_root = Some(ws_parent.0.clone());
    ws_abs_idx.source_roots = vec![Some(ws_sub.clone())];
    ws_abs_idx.readers = vec![abs_reader];
    ws_abs_idx.mtimes = vec![Some(std::time::SystemTime::now())];

    // 1. Phase 1 ws_abs match (lines 247, 366)
    let mtime_p1 = ws_abs_idx.shard_mtime_for_file("sub_pkg/src/virtual_lib.rs");
    assert!(mtime_p1.is_some());
    let hash_p1 = ws_abs_idx.source_hash_for_file("sub_pkg/src/virtual_lib.rs");
    assert_eq!(hash_p1, Some(99999u64));

    // 2. Phase 2 ws_abs match via reader_file_hashes (lines 269, 391)
    let mut sidecar_map2 = HashMap::new();
    let abs_sidecar = ws_sub.join("src/virtual_sidecar.rs").to_string_lossy().to_string();
    sidecar_map2.insert(abs_sidecar, 55555u64);
    ws_abs_idx.reader_file_hashes = vec![sidecar_map2];
    let mtime_p2 = ws_abs_idx.shard_mtime_for_file("sub_pkg/src/virtual_sidecar.rs");
    assert!(mtime_p2.is_some());
    let hash_p2 = ws_abs_idx.source_hash_for_file("sub_pkg/src/virtual_sidecar.rs");
    assert_eq!(hash_p2, Some(55555u64));
}

#[test]
fn test_provenance_fast_path_in_memory() {
    let td = new_test_dir("fast_path_test");
    let idx_path = td.0.join("sample.ridx");
    let mut writer = IndexWriter::new("sample", "0.1.0");
    let fid = writer.get_or_insert_file("src/lib.rs");
    assert_eq!(fid, 0);
    writer.add_file_hash("src/lib.rs", 0x1234_5678_u64);
    let mut f = fs::File::create(&idx_path).unwrap();
    writer.write_to(&mut f).unwrap();
    drop(f);

    let mut ws = WorkspaceIndex::open(&idx_path).unwrap();
    let now = std::time::SystemTime::now();
    ws.mtimes = vec![Some(now)];

    // Test fast path on relative path
    assert_eq!(ws.source_hash_for_file("src/lib.rs"), Some(0x1234_5678_u64));
    assert_eq!(ws.shard_mtime_for_file("src/lib.rs"), Some(now));

    // Test fast path on absolute path when workspace root is configured
    ws.workspace_root = Some(td.0.clone());
    let abs_lib = td.0.join("src/lib.rs");
    let abs_str = abs_lib.to_string_lossy();
    assert_eq!(ws.source_hash_for_file(&abs_str), Some(0x1234_5678_u64));
    assert_eq!(ws.shard_mtime_for_file(&abs_str), Some(now));
}
