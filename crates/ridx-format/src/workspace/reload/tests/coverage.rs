/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::{new_test_dir, write_test_index, WorkspaceIndex};
use crate::format::DefKind;
use crate::writer::{IndexWriter, RawDefinition};
use std::collections::HashMap;
use std::fs::{self, File};

#[test]
fn test_reload_variant_dependency_dedup_and_generation_mismatch() {
    let test_dir = new_test_dir("ridx_reload_dedup");
    let v1_dir = test_dir.0.join("variants/v1");
    fs::create_dir_all(&v1_dir).unwrap();
    let idx1 = v1_dir.join("main.ridx");
    write_test_index(&idx1, "main");

    let dep_file = test_dir.0.join("dep.ridx");
    let mut w_dep = IndexWriter::new("dep", "0.1.0");
    w_dep.set_unit_key("dep_key_shared");
    w_dep.set_generation("gen_alpha");
    let mut f_dep = File::create(&dep_file).unwrap();
    w_dep.write_to(&mut f_dep).unwrap();
    drop(f_dep);

    let mf = test_dir.0.join("ridx_manifest.json");
    let manifest_v1 = serde_json::json!({
        "version": "2.0",
        "format_version": crate::CURRENT_FORMAT_VERSION,
        "extractor_version": crate::EXTRACTOR_VERSION,
        "active_variant_id": "v1",
        "variant_dir": "variants/v1",
        "workspace_root": test_dir.0.display().to_string(),
        "shards": [{ "name": "main", "version": "0.1.0", "file": "main.ridx", "is_dependency": false }]
    });
    fs::write(&mf, serde_json::to_string(&manifest_v1).unwrap()).unwrap();

    let mut ws = WorkspaceIndex::open(&test_dir.0).unwrap();
    assert_eq!(ws.readers.len(), 1);

    let v2_dir = test_dir.0.join("variants/v2");
    fs::create_dir_all(&v2_dir).unwrap();
    let idx2 = v2_dir.join("main.ridx");
    write_test_index(&idx2, "main");

    let dep_file2 = test_dir.0.join("dep_mismatch.ridx");
    let mut w_dep2 = IndexWriter::new("dep2", "0.1.0");
    w_dep2.set_unit_key("dep2_key");
    w_dep2.set_generation("gen_actual");
    let mut f_dep2 = File::create(&dep_file2).unwrap();
    w_dep2.write_to(&mut f_dep2).unwrap();
    drop(f_dep2);

    std::thread::sleep(std::time::Duration::from_millis(50));
    let manifest_v2 = serde_json::json!({
        "version": "2.0",
        "format_version": crate::CURRENT_FORMAT_VERSION,
        "extractor_version": crate::EXTRACTOR_VERSION,
        "active_variant_id": "v2",
        "variant_dir": "variants/v2",
        "workspace_root": test_dir.0.display().to_string(),
        "shards": [{ "name": "main", "version": "0.2.0", "file": "main.ridx", "is_dependency": false }],
        "dependency_shards": [
            {
                "name": "dep",
                "version": "0.1.0",
                "unit_key": "dep_key_shared",
                "object_path": dep_file.display().to_string(),
                "source_root": test_dir.0.display().to_string(),
                "is_dependency": true,
                "generation": "gen_alpha",
                "dependencies": ["extra_dep"],
                "extern_units": { "core": "core_unit_key" },
                "file_hashes": { "src/lib.rs": 1234 }
            },
            {
                "name": "dep2",
                "version": "0.1.0",
                "unit_key": "dep2_key",
                "object_path": dep_file2.display().to_string(),
                "source_root": test_dir.0.display().to_string(),
                "is_dependency": true,
                "generation": null,
                "dependencies": ["unreached"],
                "extern_units": {},
                "file_hashes": { "src/different.rs": 9999 }
            },
            {
                "name": "dep_nonexistent",
                "version": "0.1.0",
                "unit_key": "dep_nonexistent_key",
                "object_path": "nonexistent_obj_path.ridx",
                "source_root": null,
                "is_dependency": true,
                "generation": null,
                "dependencies": [],
                "extern_units": {},
                "file_hashes": {}
            }
        ]
    });
    fs::write(&mf, serde_json::to_string(&manifest_v2).unwrap()).unwrap();

    let reloaded = ws.reload_if_modified().unwrap();
    assert!(reloaded);
    assert_eq!(ws.active_variant_id.as_deref(), Some("v2"));

    let canon_file = test_dir.0.join("src/lib.rs");
    let _ = fs::create_dir_all(canon_file.parent().unwrap());
    let _ = fs::write(&canon_file, b"content");
    // Test non-canonical path that resolves to canonical in last_reloaded_files (line 522)
    let non_canon = format!("{}/./src/lib.rs", test_dir.0.display());
    assert!(ws.is_file_reloaded(&non_canon));
    assert!(ws.is_file_reloaded(canon_file.to_str().unwrap()));
    assert!(ws.is_file_reloaded("src/lib.rs"));
    assert!(ws.is_file_reloaded("some_prefix/src/lib.rs"));
    assert!(!ws.is_file_reloaded("other_unrelated_file.rs"));

    // Test shard deletion reconciliation (lines 306-330)
    let prev_count = ws.crate_count();
    let _ = fs::remove_file(&dep_file2);
    let reloaded_after_delete = ws.reload_if_modified().unwrap();
    assert!(reloaded_after_delete);
    assert_eq!(ws.crate_count(), prev_count - 1);

    // Test empty new variant directory fallback (lines 297-302)
    let empty_v3_dir = test_dir.0.join("variants/v3");
    let _ = fs::create_dir_all(&empty_v3_dir);
    let manifest_v3 = serde_json::json!({
        "version": "2.0",
        "format_version": 2,
        "extractor_version": 1,
        "active_variant_id": "v3",
        "variant_dir": "variants/v3",
        "base_out_dir": test_dir.0.display().to_string(),
        "workspace_root": test_dir.0.display().to_string(),
        "manifest_path": "",
        "profile": "dev",
        "timestamp_epoch_secs": 0,
        "total_crates": 0,
        "total_defs": 0,
        "total_refs": 0,
        "total_types": 0,
        "dependency_shards": [],
        "crates": []
    });
    fs::write(&mf, serde_json::to_string(&manifest_v3).unwrap()).unwrap();
    // Since v3 has no readers, it falls through lines 297-302
    let _ = ws.reload_if_modified();

    // Test meta_changed branch (lines 401-443): sidecar changed without .ridx file mtime changing
    let sidecar_p = test_dir.0.join("main.json");
    let sidecar_info = serde_json::json!({
        "name": "main",
        "version": "0.2.0",
        "unit_key": "main_unit",
        "generation": "gen_alpha",
        "object_path": idx2.display().to_string(),
        "source_root": test_dir.0.display().to_string(),
        "is_dependency": false,
        "dependencies": ["dep_updated"],
        "extern_units": {},
        "file_hashes": { "src/lib.rs": 12345 }
    });
    fs::write(&sidecar_p, serde_json::to_string(&sidecar_info).unwrap()).unwrap();
    // Set meta_paths and past meta_mtime so meta_changed evaluates to true
    if let Some(mp) = ws.meta_paths.get_mut(0) {
        *mp = Some(sidecar_p.clone());
    }
    if let Some(mtime) = ws.meta_mtimes.get_mut(0) {
        *mtime = Some(std::time::SystemTime::UNIX_EPOCH);
    }
    let reloaded_meta = ws.reload_if_modified().unwrap();
    assert!(reloaded_meta);
}

#[test]
fn test_reload_additional_coverage() {
    let _lock = crate::workspace::ENV_LOCK.lock().unwrap();
    let temp = new_test_dir("reload_cov");
    let idx_path = temp.0.join("test.ridx");
    let mut w = IndexWriter::new("reload_cov_pkg", "0.1.0");
    w.set_unit_key("cov_uk");
    w.set_generation("gen_cov");
    w.get_or_insert_file("src/lib.rs");
    let mut f = File::create(&idx_path).unwrap();
    w.write_to(&mut f).unwrap();
    drop(f);

    let mut ws = WorkspaceIndex::open(&idx_path).unwrap();

    // 1. is_file_reloaded with canonical path matching (line 522)
    // and record_reloaded_reader_files with source_root (line 538)
    ws.source_roots = vec![Some(temp.0.clone())];
    ws.record_reloaded_reader_files(0);
    let test_src = temp.0.join("src/lib.rs");
    let _ = fs::create_dir_all(test_src.parent().unwrap());
    let _ = fs::write(&test_src, b"fn foo() {}");
    assert!(ws.is_file_reloaded(&test_src.to_string_lossy()));

    // 2. Discover new file whose unit_key is already present (line 485)
    let dup_path = temp.0.join("dup.ridx");
    let mut w_dup = IndexWriter::new("dup_pkg", "0.1.0");
    w_dup.set_unit_key("cov_uk"); // same unit key as idx_path
    let mut f_dup = File::create(&dup_path).unwrap();
    w_dup.write_to(&mut f_dup).unwrap();
    drop(f_dup);

    ws.root_dir = Some(temp.0.clone());
    let changed = ws.reload_if_modified().unwrap();
    // Since dup_pkg has unit_key == "cov_uk" which is already in ws.unit_keys, it is skipped (line 485)
    assert!(!changed);

    // 3. Variant reload with cache fallback (line 141), empty extern_units replaced (lines 180-182),
    // unit_key mismatch (line 205), and computed digest mismatch (line 229)
    let cache_dir = temp.0.join("cache");
    let _ = fs::create_dir_all(&cache_dir);
    std::env::set_var("RIDX_CACHE_DIR", cache_dir.display().to_string());
    let cached_obj = crate::format::cache_object_path(&cache_dir, "cache_reload_uk");
    let _ = fs::create_dir_all(cached_obj.parent().unwrap());
    let mut w_cached = IndexWriter::new("cached_reload_dep", "0.1.0");
    w_cached.set_unit_key("cache_reload_uk");
    w_cached.set_generation("gen_cached_target");
    let mut fc = File::create(&cached_obj).unwrap();
    w_cached.write_to(&mut fc).unwrap();
    drop(fc);

    let cached_mismatch_obj = crate::format::cache_object_path(&cache_dir, "cache_mismatch_uk");
    let _ = fs::create_dir_all(cached_mismatch_obj.parent().unwrap());
    let mut w_mismatch = IndexWriter::new("cached_mismatch_dep", "0.1.0");
    w_mismatch.set_unit_key("cache_mismatch_uk");
    w_mismatch.set_generation("gen_reader_actual");
    let mut fm = File::create(&cached_mismatch_obj).unwrap();
    w_mismatch.write_to(&mut fm).unwrap();
    drop(fm);

    let no_gen_obj = crate::format::cache_object_path(&cache_dir, "no_gen_uk");
    let _ = fs::create_dir_all(no_gen_obj.parent().unwrap());
    let mut w_nogen = IndexWriter::new("no_gen_dep", "0.1.0");
    w_nogen.set_unit_key("no_gen_uk");
    let mut fng = File::create(&no_gen_obj).unwrap();
    w_nogen.write_to(&mut fng).unwrap();
    drop(fng);

    let v_new_dir = temp.0.join("variants/v_cov");
    let _ = fs::create_dir_all(&v_new_dir);
    let v_new_idx = v_new_dir.join("cov.ridx");
    let mut w_new = IndexWriter::new("v_cov_pkg", "0.1.0");
    w_new.set_unit_key("cov_uk");
    w_new.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "test_fn".to_string(),
        path: "v_cov_pkg::test_fn".to_string(),
        signature: Some("fn()".to_string()),
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 2,
        end_col: 2,
    });
    let mut fn_idx = File::create(&v_new_idx).unwrap();
    w_new.write_to(&mut fn_idx).unwrap();
    drop(fn_idx);

    let mut ext_u = HashMap::new();
    ext_u.insert("dep_alias".to_string(), "dep_unit".to_string());

    let mf_cov = serde_json::json!({
        "version": "2.0",
        "format_version": crate::format::CURRENT_FORMAT_VERSION,
        "extractor_version": crate::format::EXTRACTOR_VERSION,
        "active_variant_id": "v_cov",
        "variant_dir": "variants/v_cov",
        "base_out_dir": temp.0.display().to_string(),
        "workspace_root": temp.0.display().to_string(),
        "manifest_path": "",
        "profile": "dev",
        "timestamp_epoch_secs": 0,
        "total_crates": 1,
        "total_defs": 0,
        "total_refs": 0,
        "total_types": 0,
        "dependency_shards": [
            {
                "name": "v_cov_pkg",
                "version": "0.1.0",
                "unit_key": "cov_uk",
                "generation": null,
                "object_path": v_new_idx.display().to_string(),
                "source_root": temp.0.display().to_string(),
                "is_dependency": false,
                "dependencies": ["dep_cov_1"],
                "extern_units": ext_u, // hits lines 180-182
                "file_hashes": { "src/lib.rs": 12345 }
            },
            {
                "name": "cached_reload_dep",
                "version": "0.1.0",
                "unit_key": "cache_reload_uk",
                "generation": null,
                "object_path": "/nonexistent/object.ridx", // hits line 141 (fallback to cache)
                "source_root": null,
                "is_dependency": true,
                "dependencies": [],
                "extern_units": {},
                "file_hashes": { "src/lib.rs": 99999 } // hits line 229 (computed != gen_cached_target)
            },
            {
                "name": "cached_mismatch_dep",
                "version": "0.1.0",
                "unit_key": "cache_mismatch_uk",
                "generation": "gen_different_from_reader", // hits line 228
                "object_path": "/nonexistent/object.ridx",
                "source_root": null,
                "is_dependency": true,
                "dependencies": [],
                "extern_units": {},
                "file_hashes": {}
            },
            {
                "name": "no_gen_dep",
                "version": "0.1.0",
                "unit_key": "no_gen_uk",
                "generation": "dep_gen_fallback", // hits line 244
                "object_path": "/nonexistent/object.ridx",
                "source_root": null,
                "is_dependency": true,
                "dependencies": [],
                "extern_units": {},
                "file_hashes": {}
            },
            {
                "name": "mismatch_key_dep",
                "version": "0.1.0",
                "unit_key": "expected_key_wrong", // hits line 205 (unit_key != r.unit_key)
                "generation": null,
                "object_path": idx_path.display().to_string(),
                "source_root": null,
                "is_dependency": true,
                "dependencies": [],
                "extern_units": {},
                "file_hashes": {}
            }
        ],
        "crates": []
    });

    let mf_path = temp.0.join("ridx_manifest.json");
    fs::write(&mf_path, serde_json::to_string(&mf_cov).unwrap()).unwrap();
    ws.manifest_path = Some(mf_path);
    ws.manifest_mtime = Some(std::time::SystemTime::UNIX_EPOCH);
    let reloaded_v = ws.reload_if_modified().unwrap();
    assert!(reloaded_v);

    // Test is_file_reloaded (lines 535-538)
    assert!(ws.is_file_reloaded("src/lib.rs"));
    assert!(ws.is_file_reloaded("lib.rs"));
    assert!(!ws.is_file_reloaded("nonexistent/path.rs"));

    let _ = fs::remove_file(&cached_obj);
    let _ = fs::remove_file(&cached_mismatch_obj);
    let _ = fs::remove_file(&no_gen_obj);
    std::env::remove_var("RIDX_CACHE_DIR");
}
