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

#[test]
fn test_provenance_edge_cases_and_sidecar_fallback() {
    // 1. normalize_path with root .. and leading components
    assert_eq!(normalize_path(Path::new("/..")), PathBuf::from("/"));
    assert_eq!(normalize_path(Path::new("a/b/../../c")), PathBuf::from("c"));
    assert_eq!(normalize_path(Path::new("../../a")), PathBuf::from("../../a"));

    // 2. Symlink parent traversal
    let temp_dir = std::env::temp_dir().join(format!("ridx_prov_edge_{}", std::process::id()));
    let _ = fs::create_dir_all(&temp_dir);
    let temp_dir = temp_dir.canonicalize().unwrap_or(temp_dir);

    let real_dir = temp_dir.join("real");
    let _ = fs::create_dir_all(&real_dir);
    let link_dir = temp_dir.join("link");
    #[cfg(unix)]
    let _ = std::os::unix::fs::symlink(&real_dir, &link_dir);

    #[cfg(unix)]
    {
        let p_with_symlink = link_dir.join("../real");
        assert!(path_traverses_symlink_parent(&p_with_symlink));
    }

    // 3. file_entry_matches relative matches
    assert!(file_entry_matches("src/lib.rs", None, Path::new("src/lib.rs"), None));
    assert!(!file_entry_matches("src/lib.rs", None, Path::new("src/other.rs"), None));
    assert!(!file_entry_matches("", None, Path::new("src/lib.rs"), None));

    // 4. Shard mtime and source hash when file not found
    let test_idx = temp_dir.join("test.ridx");
    let mut writer = crate::writer::IndexWriter::new("prov_crate", "0.1.0");
    let mut map = HashMap::new();
    map.insert("src/lib.rs".to_string(), 12345u64);
    writer.set_file_hashes(map);
    let mut f = fs::File::create(&test_idx).unwrap();
    writer.write_to(&mut f).unwrap();
    drop(f);

    let ws = WorkspaceIndex::open(&test_idx).unwrap();
    assert_eq!(ws.shard_mtime_for_file("nonexistent.rs"), None);
    assert_eq!(ws.source_hash_for_file("nonexistent.rs"), None);

    // 5. load_file_metadata directly
    let meta_file = temp_dir.join("test.meta");
    fs::write(&meta_file, b"{ corrupted_json").unwrap();
    let embedded = HashMap::new();
    let meta = WorkspaceIndex::load_file_metadata(&test_idx, &temp_dir, "prov_crate", "uk", "gen", &embedded);
    assert_eq!(meta.source_root, None);
    assert_eq!(meta.unit_key, Some("uk".to_string()));
    assert_eq!(meta.generation, Some("gen".to_string()));
    assert!(meta.dependencies.is_empty());
    assert!(meta.extern_units.is_empty());
    assert!(meta.file_hashes.is_empty());
    assert!(meta.meta_path.is_some());
    assert_eq!(meta.meta_mtime, None);

    let meta_empty = WorkspaceIndex::load_file_metadata(&test_idx, &temp_dir, "prov_crate", "", "", &embedded);
    assert_eq!(meta_empty.unit_key, None);
    assert_eq!(meta_empty.generation, None);

    let _ = fs::remove_dir_all(&temp_dir);

    // 6. Secondary candidate matching in shard_mtime_for_file and source_hash_for_file (lines 267-271, 389-393)
    let temp2 = std::env::temp_dir().join(format!("ridx_prov_sec_{}", std::process::id()));
    let _ = fs::create_dir_all(&temp2);
    let temp2 = temp2.canonicalize().unwrap_or(temp2);
    let test_idx2 = temp2.join("test2.ridx");
    let mut writer2 = crate::writer::IndexWriter::new("prov_crate2", "0.1.0");
    let mut f2 = fs::File::create(&test_idx2).unwrap();
    writer2.write_to(&mut f2).unwrap();
    drop(f2);

    let mut ws_cand = WorkspaceIndex::open(&test_idx2).unwrap();
    ws_cand.workspace_root = Some(temp2.clone());
    ws_cand.source_roots[0] = Some(PathBuf::from("/other_dep_root"));
    let sub_lib = temp2.join("sub/lib.rs");
    ws_cand.reader_file_hashes[0].insert(sub_lib.to_str().unwrap().to_string(), 88888u64);
    assert!(ws_cand.shard_mtime_for_file("sub/lib.rs").is_some());
    assert_eq!(ws_cand.source_hash_for_file("sub/lib.rs"), Some(88888));

    // 7. Non-normalized entry matching (lines 151-158)
    let sub_dir = temp2.join("sub_dir");
    let _ = fs::create_dir_all(&sub_dir);
    let lib_sub = sub_dir.join("lib.rs");
    let _ = fs::write(&lib_sub, b"// content");
    let non_norm_entry = temp2.join("sub_dir/../sub_dir/lib.rs");
    assert!(file_entry_matches(non_norm_entry.to_str().unwrap(), Some(&temp2), Path::new("sub_dir/lib.rs"), None,));

    // 8. CurDir in symlink traversal (line 67) & relative entry match with no reader_origin (line 142)
    assert!(!path_traverses_symlink_parent(Path::new("./sub_dir/lib.rs")));
    assert!(file_entry_matches("sub_dir/lib.rs", None, Path::new("sub_dir/lib.rs"), None));

    // 9. Sidecar with h == 0 or empty key (line 384)
    ws_cand.reader_file_hashes[0].insert("".to_string(), 12345);
    ws_cand.reader_file_hashes[0].insert("dummy.rs".to_string(), 0);
    assert_eq!(ws_cand.source_hash_for_file("dummy.rs"), None);

    // 10. Shard mtime and source hash matching ws_abs in reader (lines 247, 366)
    let mut w_reader_abs = crate::writer::IndexWriter::new("abs_crate", "0.1.0");
    let abs_file_str = temp2.join("sub_dir/lib.rs").to_str().unwrap().to_string();
    let hash = crate::format::normalized_source_hash(b"// content");
    let mut fmap = HashMap::new();
    fmap.insert(abs_file_str.clone(), hash);
    w_reader_abs.set_file_hashes(fmap);
    w_reader_abs.add_definition(crate::writer::RawDefinition {
        def_id: 1,
        kind: crate::DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "abs_fn".to_string(),
        path: "abs_crate::abs_fn".to_string(),
        signature: None,
        file: abs_file_str,
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 10,
    });
    let abs_idx_file = temp2.join("abs.ridx");
    let mut f_abs = fs::File::create(&abs_idx_file).unwrap();
    w_reader_abs.write_to(&mut f_abs).unwrap();
    drop(f_abs);
    let mut ws_abs = WorkspaceIndex::open(&abs_idx_file).unwrap();
    ws_abs.workspace_root = Some(temp2.clone());
    assert!(ws_abs.shard_mtime_for_file("sub_dir/lib.rs").is_some());
    assert_eq!(ws_abs.source_hash_for_file("sub_dir/lib.rs"), Some(crate::format::normalized_source_hash(b"// content")));

    // 11. load_file_metadata with computed digest generation matching and mismatching (lines 440-448)
    let deps_dir = temp2.join(".deps");
    fs::create_dir_all(&deps_dir).unwrap();
    let mut fhashes = HashMap::new();
    fhashes.insert("sub_dir/lib.rs".to_string(), 55555u64);
    let expected_digest = crate::format::compute_source_snapshot_digest(&fhashes);
    let expected_gen_str = format!("{:016x}", expected_digest);

    let dep_info = crate::workspace::DependencyShardInfo {
        name: "computed_crate".to_string(),
        version: "0.1.0".to_string(),
        unit_key: "comp_uk".to_string(),
        generation: None,
        object_path: abs_idx_file.display().to_string(),
        source_root: Some(temp2.display().to_string()),
        target_kind: None,
        is_dependency: true,
        dependencies: vec![],
        extern_units: HashMap::new(),
        file_hashes: fhashes,
        stable_crate_id: None,
        caught_panics: None,
        compiler_args: vec![],
        env: HashMap::new(),
    };
    fs::write(deps_dir.join("computed_crate.json"), serde_json::to_string(&dep_info).unwrap()).unwrap();

    // Mismatching expected generation (lines 445-447)
    let meta_mismatch = WorkspaceIndex::load_file_metadata(&temp2.join("computed_crate.ridx"), &temp2, "computed_crate", "comp_uk", "wrong_gen_12345678", &HashMap::new());
    assert_eq!(meta_mismatch.source_root, None);

    // Matching expected generation (lines 441-444, 463-472)
    let meta_match = WorkspaceIndex::load_file_metadata(&temp2.join("computed_crate.ridx"), &temp2, "computed_crate", "comp_uk", &expected_gen_str, &HashMap::new());
    assert_eq!(meta_match.source_root, Some(temp2.clone()));
    assert_eq!(meta_match.unit_key, Some("comp_uk".to_string()));
    assert_eq!(meta_match.generation, Some(expected_gen_str));

    let _ = fs::remove_dir_all(&temp2);
}
