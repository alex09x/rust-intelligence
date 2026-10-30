/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::*;

#[test]
fn test_workspace_open_target_ridx_and_dep_computed_gen() {
    let _lock = ENV_LOCK.lock().unwrap();
    let test_dir = new_test_dir("ridx_target_open");
    // 1. Empty directory open errors (lines 479-482)
    let empty_dir = test_dir.0.join("empty");
    fs::create_dir_all(&empty_dir).unwrap();
    let err = WorkspaceIndex::open(&empty_dir);
    assert_eq!(err.unwrap_err().kind(), io::ErrorKind::NotFound);

    // 2. Open single file inside target/ridx (lines 93-96)
    let target_ridx = test_dir.0.join("target/ridx");
    fs::create_dir_all(&target_ridx).unwrap();
    let single_file = target_ridx.join("single.ridx");
    let mut w_single = IndexWriter::new("single", "0.1.0");
    let mut f_single = File::create(&single_file).unwrap();
    w_single.write_to(&mut f_single).unwrap();
    drop(f_single);

    let ws_single = WorkspaceIndex::open(&single_file).unwrap();
    assert_eq!(ws_single.workspace_root, Some(test_dir.0.clone()));

    // 3. Open directory ending in target/ridx (lines 492-495)
    let ws_dir = WorkspaceIndex::open(&target_ridx).unwrap();
    assert_eq!(ws_dir.workspace_root, Some(test_dir.0.clone()));

    // 4. Dependency shard with computed generation from file_hashes (lines 423-430)
    let dep_file = test_dir.0.join("dep_comp.ridx");
    let mut w_dep = IndexWriter::new("dep_comp", "0.1.0");
    w_dep.set_unit_key("dep_comp_key");
    let mut file_hashes = HashMap::new();
    file_hashes.insert("src/lib.rs".to_string(), 12345u64);
    let computed_digest = crate::format::compute_source_snapshot_digest(&file_hashes);
    w_dep.set_generation(format!("{:016x}", computed_digest));
    let mut f_dep = File::create(&dep_file).unwrap();
    w_dep.write_to(&mut f_dep).unwrap();
    drop(f_dep);

    let dep_file_mismatch = test_dir.0.join("dep_mismatch.ridx");
    let mut w_dep_m = IndexWriter::new("dep_mismatch", "0.1.0");
    w_dep_m.set_unit_key("dep_mismatch_key");
    w_dep_m.set_generation("mismatched_generation_val");
    let mut f_dep_m = File::create(&dep_file_mismatch).unwrap();
    w_dep_m.write_to(&mut f_dep_m).unwrap();
    drop(f_dep_m);

    let v_comp = target_ridx.join("variants/v_comp");
    fs::create_dir_all(&v_comp).unwrap();
    let mut f_s2 = File::create(v_comp.join("single.ridx")).unwrap();
    w_single.write_to(&mut f_s2).unwrap();
    drop(f_s2);

    let mf = target_ridx.join("ridx_manifest.json");
    let manifest_data = serde_json::json!({
        "version": "2.0",
        "format_version": crate::format::CURRENT_FORMAT_VERSION,
        "extractor_version": crate::format::EXTRACTOR_VERSION,
        "active_variant_id": "v_comp",
        "variant_dir": "variants/v_comp",
        "workspace_root": test_dir.0.display().to_string(),
        "shards": [{ "name": "single", "version": "0.1.0", "file": "single.ridx", "is_dependency": false }],
        "dependency_shards": [
            {
                "name": "dep_comp",
                "version": "0.1.0",
                "unit_key": "dep_comp_key",
                "object_path": dep_file.display().to_string(),
                "source_root": null,
                "is_dependency": true,
                "generation": null,
                "file_hashes": { "src/lib.rs": 12345 },
                "dependencies": [],
                "extern_units": {}
            },
            {
                "name": "dep_mismatch",
                "version": "0.1.0",
                "unit_key": "dep_mismatch_key",
                "object_path": dep_file_mismatch.display().to_string(),
                "source_root": null,
                "is_dependency": true,
                "generation": null,
                "file_hashes": { "src/other.rs": 99999 },
                "dependencies": [],
                "extern_units": {}
            }
        ]
    });
    fs::write(&mf, serde_json::to_string(&manifest_data).unwrap()).unwrap();

    let ws_comp = WorkspaceIndex::open(&target_ridx).unwrap();
    assert_eq!(ws_comp.readers.len(), 3);
}

#[test]
fn test_corrupted_index_skipping_and_auto_discover() {
    let _lock = ENV_LOCK.lock().unwrap();
    let test_dir = new_test_dir("ws_corrupt_test");
    let valid_file = test_dir.0.join("valid.ridx");
    let mut w = IndexWriter::new("valid_crate", "0.1.0");
    let mut f = File::create(&valid_file).unwrap();
    w.write_to(&mut f).unwrap();
    drop(f);

    // Put a corrupted .ridx file in the same directory (lines 342-344)
    let corrupt_file = test_dir.0.join("corrupt.ridx");
    fs::write(&corrupt_file, b"NOT_A_VALID_RIDX_FILE_HEADER").unwrap();

    // WorkspaceIndex::open should skip the corrupt file and successfully load valid.ridx
    let ws = WorkspaceIndex::open(&test_dir.0).unwrap();
    assert_eq!(ws.readers.len(), 1);
    assert_eq!(ws.readers[0].crate_name(), "valid_crate");

    // Test auto_discover fallback (lines 549-552)
    let deep_empty = new_test_dir("ws_empty_discover").0.join("deep/sub/dir");
    fs::create_dir_all(&deep_empty).unwrap();
    let orig_dir = std::env::current_dir().unwrap();
    std::env::remove_var("RIDX_INDEX_DIR");
    std::env::set_current_dir(&deep_empty).unwrap();
    let res = WorkspaceIndex::auto_discover();
    assert!(res.is_err());
    std::env::set_current_dir(orig_dir).unwrap();
}

#[test]
fn test_workspace_open_edge_cases() {
    let _lock = ENV_LOCK.lock().unwrap();
    // 1. Directory with no .ridx files (lines 291-296)
    let empty_dir = new_test_dir("ws_no_ridx");
    let res = WorkspaceIndex::open(&empty_dir.0);
    assert!(res.unwrap_err().to_string().contains("No .ridx index files found"));

    // 1b. Directory with only invalid .ridx files (lines 479-482)
    let corrupt_dir = new_test_dir("ws_all_corrupt");
    fs::write(corrupt_dir.0.join("bad.ridx"), b"bad").unwrap();
    let res_corrupt = WorkspaceIndex::open(&corrupt_dir.0);
    assert!(res_corrupt.unwrap_err().to_string().contains("Failed to open any valid"));

    // 3. Manifest with empty workspace_root (line 213) and empty active_variant_id (line 219)
    let ws_dir = new_test_dir("ws_legacy_manifest");
    let valid_file = ws_dir.0.join("pkg.ridx");
    let mut w = IndexWriter::new("legacy_pkg", "0.1.0");
    w.set_unit_key("unit_legacy");
    w.set_generation("gen_legacy");
    let mut f = File::create(&valid_file).unwrap();
    w.write_to(&mut f).unwrap();
    drop(f);

    // Variant manifest with empty workspace_root and active_variant_id
    let manifest_json = serde_json::json!({
        "version": "2.0",
        "format_version": crate::format::CURRENT_FORMAT_VERSION,
        "extractor_version": crate::format::EXTRACTOR_VERSION,
        "active_variant_id": "", // empty active_variant_id (line 219)
        "variant_dir": "",
        "base_out_dir": ws_dir.0.display().to_string(),
        "workspace_root": "", // empty workspace_root (line 213)
        "manifest_path": "",
        "profile": "dev",
        "timestamp_epoch_secs": 0,
        "total_crates": 1,
        "total_defs": 0,
        "total_refs": 0,
        "total_types": 0,
        "dependency_shards": [
            {
                "name": "legacy_pkg",
                "version": "0.1.0",
                "unit_key": "different_unit_key", // unit mismatch (line 412)
                "generation": null,
                "object_path": valid_file.display().to_string(),
                "source_root": null,
                "is_dependency": true,
                "dependencies": [],
                "extern_units": {},
                "file_hashes": { "src/lib.rs": 999999 } // digest mismatch with gen_legacy (lines 428-430)
            }
        ],
        "crates": []
    });
    fs::write(ws_dir.0.join("ridx_manifest.json"), serde_json::to_string(&manifest_json).unwrap()).unwrap();

    let ws = WorkspaceIndex::open(&ws_dir.0).unwrap();
    assert_eq!(ws.readers.len(), 1);
    assert_eq!(ws.readers[0].crate_name(), "legacy_pkg");

    // 4. Open search_dir with variant_manifest.json and empty workspace_root (lines 221-281, 278)
    let v_dir = new_test_dir("ws_v_manifest");
    let v_file = v_dir.0.join("v_pkg.ridx");
    let mut wv = IndexWriter::new("v_pkg", "0.1.0");
    wv.set_unit_key("unit_v");
    let mut fv = File::create(&v_file).unwrap();
    wv.write_to(&mut fv).unwrap();
    drop(fv);
    let mut v_manifest = manifest_json.clone();
    v_manifest["extractor_version"] = serde_json::Value::String(crate::format::EXTRACTOR_VERSION.to_string());
    v_manifest["workspace_root"] = serde_json::Value::String("".to_string());
    fs::write(v_dir.0.join("variant_manifest.json"), serde_json::to_string(&v_manifest).unwrap()).unwrap();
    let ws_v = WorkspaceIndex::open(&v_dir.0).unwrap();
    assert_eq!(ws_v.readers.len(), 1);
    assert_eq!(ws_v.readers[0].crate_name(), "v_pkg");
    assert_eq!(ws_v.workspace_root, Some(v_dir.0.clone()));

    // 5. Test auto_discover with RIDX_INDEX_DIR set (line 531)
    std::env::set_var("RIDX_INDEX_DIR", v_dir.0.display().to_string());
    let ws_auto = WorkspaceIndex::auto_discover().unwrap();
    assert_eq!(ws_auto.readers.len(), 1);
    std::env::remove_var("RIDX_INDEX_DIR");

    // Test Debug formatting for WorkspaceIndex (lines 52-57)
    let dbg_str = format!("{:?}", ws_auto);
    assert!(dbg_str.contains("WorkspaceIndex"));

    // 6. Manifest with active_variant_id and empty workspace_root (line 210)
    let ws_m_dir = new_test_dir("ws_m_empty_root");
    let m_file = ws_m_dir.0.join("pkg.ridx");
    let mut wm = IndexWriter::new("pkg", "0.1.0");
    wm.set_generation("gen_target");
    let mut fm = File::create(&m_file).unwrap();
    wm.write_to(&mut fm).unwrap();
    drop(fm);

    let v_empty_dir = ws_m_dir.0.join("variants/var_empty");
    let _ = fs::create_dir_all(&v_empty_dir);
    let _ = fs::copy(&m_file, v_empty_dir.join("pkg.ridx"));
    let mut m_empty_ws = manifest_json.clone();
    m_empty_ws["extractor_version"] = serde_json::Value::String(crate::format::EXTRACTOR_VERSION.to_string());
    m_empty_ws["active_variant_id"] = serde_json::Value::String("var_empty".to_string());
    m_empty_ws["variant_dir"] = serde_json::Value::String("variants/var_empty".to_string());
    m_empty_ws["workspace_root"] = serde_json::Value::String("".to_string());
    fs::write(ws_m_dir.0.join("ridx_manifest.json"), serde_json::to_string(&m_empty_ws).unwrap()).unwrap();
    let ws_m = WorkspaceIndex::open(&ws_m_dir.0).unwrap();
    assert_eq!(ws_m.workspace_root, Some(ws_m_dir.0.clone()));

    // 7. Dep shards: fallback to cache_dir (lines 353-355), missing dep (line 357),
    // digest mismatch with null generation (lines 425, 427), and corrupt reader (lines 468-469)
    let cache_dir = ws_m_dir.0.join("cache");
    let _ = fs::create_dir_all(&cache_dir);
    std::env::set_var("RIDX_CACHE_DIR", cache_dir.display().to_string());
    let cached_obj = crate::format::cache_object_path(&cache_dir, "cached_unit");
    let _ = fs::create_dir_all(cached_obj.parent().unwrap());
    let mut w_cached = IndexWriter::new("cached_dep", "0.1.0");
    w_cached.set_unit_key("cached_unit");
    w_cached.set_generation("gen_cached");
    let mut fc = File::create(&cached_obj).unwrap();
    w_cached.write_to(&mut fc).unwrap();
    drop(fc);

    let corrupt_file = ws_m_dir.0.join("corrupt.ridx");
    fs::write(&corrupt_file, b"NOT_A_VALID_RIDX").unwrap();

    let mut m_cache = manifest_json.clone();
    m_cache["extractor_version"] = serde_json::Value::String(crate::format::EXTRACTOR_VERSION.to_string());
    m_cache["dependency_shards"] = serde_json::json!([
        {
            "name": "cached_dep",
            "version": "0.1.0",
            "unit_key": "cached_unit",
            "generation": null,
            "object_path": "/nonexistent/object.ridx",
            "source_root": null,
            "is_dependency": true,
            "dependencies": [],
            "extern_units": {},
            "file_hashes": { "src/lib.rs": 12345 } // hits line 425: computed_gen != "gen_cached"
        },
        {
            "name": "missing_dep",
            "version": "0.1.0",
            "unit_key": "missing_unit",
            "generation": null,
            "object_path": "/nonexistent/missing.ridx",
            "source_root": null,
            "is_dependency": true,
            "dependencies": [],
            "extern_units": {},
            "file_hashes": {}
        },
        {
            "name": "corrupt_dep",
            "version": "0.1.0",
            "unit_key": "corrupt_unit",
            "generation": null,
            "object_path": corrupt_file.display().to_string(),
            "source_root": null,
            "is_dependency": true,
            "dependencies": [],
            "extern_units": {},
            "file_hashes": {}
        }
    ]);
    let _ = fs::remove_file(ws_m_dir.0.join("ridx_manifest.json"));
    fs::write(ws_m_dir.0.join("variant_manifest.json"), serde_json::to_string(&m_cache).unwrap()).unwrap();
    let ws_cache = WorkspaceIndex::open(&ws_m_dir.0).unwrap();
    assert!(ws_cache.readers.iter().any(|r| r.crate_name() == "cached_dep"));
    let _ = fs::remove_file(&cached_obj);
    std::env::remove_var("RIDX_CACHE_DIR");

    // 8. Opening single file with no manifest yields None workspace_root (line 494)
    let single_dir = new_test_dir("single_file_ws");
    let single_file = single_dir.0.join("standalone.ridx");
    let mut ws_standalone = IndexWriter::new("standalone", "0.1.0");
    let mut fs_s = File::create(&single_file).unwrap();
    ws_standalone.write_to(&mut fs_s).unwrap();
    drop(fs_s);
    let ws_single = WorkspaceIndex::open(&single_file).unwrap();
    assert_eq!(ws_single.workspace_root, None);

    // 9. auto_discover with nonexistent RIDX_INDEX_DIR (line 526)
    std::env::set_var("RIDX_INDEX_DIR", "/nonexistent/index_dir_9999");
    let _ = WorkspaceIndex::auto_discover();
    std::env::remove_var("RIDX_INDEX_DIR");

    // 10. collect_ridx_files with non-ridx file and variants dir (lines 634, 640)
    let coll_dir = new_test_dir("collect_test");
    fs::write(coll_dir.0.join("ignore.txt"), b"not ridx").unwrap();
    let var_sub = coll_dir.0.join("variants");
    let _ = fs::create_dir_all(&var_sub);
    fs::write(var_sub.join("nested.ridx"), b"").unwrap();
    let normal_sub = coll_dir.0.join("normal_sub");
    let _ = fs::create_dir_all(&normal_sub);
    fs::write(normal_sub.join("found.ridx"), b"").unwrap();
    let mut collected = Vec::new();
    collect_ridx_files(&coll_dir.0, &mut collected).unwrap();
    assert_eq!(collected.len(), 1);
    assert_eq!(collected[0], normal_sub.join("found.ridx"));
}
