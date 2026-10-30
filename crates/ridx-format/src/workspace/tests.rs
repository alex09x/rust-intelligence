/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

mod open_edge_cases;

use super::*;
use crate::format::DefKind;
use crate::writer::{IndexWriter, RawDefinition};
use std::fs::File;
use std::io;

pub(super) struct TestDir(pub(super) PathBuf);
impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(super) fn new_test_dir(prefix: &str) -> TestDir {
    static CNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let id = CNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!("{}_{}_{}", prefix, std::process::id(), id));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    TestDir(path)
}

#[test]
fn test_collect_ridx_files_depth_bounding_and_exclusions() {
    let test_dir = new_test_dir("collect_depth_test");
    let root = &test_dir.0;
    let d1 = root.join("lvl1");
    let d2 = d1.join("lvl2");
    let d3 = d2.join("lvl3");
    let hidden = root.join(".hidden");
    let target = root.join("target");
    let node_modules = root.join("node_modules");

    fs::create_dir_all(&d3).unwrap();
    fs::create_dir_all(&hidden).unwrap();
    fs::create_dir_all(&target).unwrap();
    fs::create_dir_all(&node_modules).unwrap();

    fs::write(root.join("root.ridx"), b"").unwrap();
    fs::write(d1.join("lvl1.ridx"), b"").unwrap();
    fs::write(d2.join("lvl2.ridx"), b"").unwrap();
    fs::write(d3.join("lvl3.ridx"), b"").unwrap();
    fs::write(hidden.join("hidden.ridx"), b"").unwrap();
    fs::write(target.join("target.ridx"), b"").unwrap();
    fs::write(node_modules.join("nm.ridx"), b"").unwrap();

    let mut collected = Vec::new();
    collect_ridx_files(root, &mut collected).unwrap();

    let names: Vec<String> = collected.iter().map(|p| p.file_name().unwrap().to_string_lossy().to_string()).collect();

    assert!(names.contains(&"root.ridx".to_string()));
    assert!(names.contains(&"lvl1.ridx".to_string()));
    assert!(names.contains(&"lvl2.ridx".to_string()));
    assert!(!names.contains(&"lvl3.ridx".to_string()));
    assert!(!names.contains(&"hidden.ridx".to_string()));
    assert!(!names.contains(&"target.ridx".to_string()));
    assert!(!names.contains(&"nm.ridx".to_string()));
}

#[test]
fn test_workspace_index_accessors_and_stats() {
    let _lock = ENV_LOCK.lock().unwrap();
    let test_dir = new_test_dir("ridx_ws_mod_test");
    let idx = test_dir.0.join("sample.ridx");

    let mut writer = IndexWriter::new("sample", "0.1.0");
    writer.set_generation("gen123");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "test_fn".to_string(),
        path: "sample::test_fn".to_string(),
        signature: Some("fn()".to_string()),
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 2,
        end_col: 2,
    });
    let mut f = File::create(&idx).unwrap();
    writer.write_to(&mut f).unwrap();

    // 1. Open single file
    let ws_file = WorkspaceIndex::open(&idx).unwrap();
    assert_eq!(ws_file.crate_count(), 1);
    assert_eq!(ws_file.readers().len(), 1);
    assert_eq!(ws_file.paths().len(), 1);
    let _ = ws_file.meta_path_at(0);
    let _ = ws_file.meta_mtime_at(0);
    assert!(ws_file.active_variant_id().is_none());
    assert!(ws_file.workspace_root().is_none());
    let mut ws_mut = ws_file;
    ws_mut.set_workspace_root(Some(PathBuf::from("/test_root")));
    assert_eq!(ws_mut.workspace_root(), Some(Path::new("/test_root")));
    let ws_file = ws_mut;

    let stats = ws_file.stats();
    assert_eq!(stats.total_crates, 1);
    assert_eq!(stats.total_defs, 1);
    assert_eq!(stats.crate_summaries.len(), 1);
    assert_eq!(stats.crate_summaries[0].name, "sample");

    // 2. Open directory
    let ws_dir = WorkspaceIndex::open(&test_dir.0).unwrap();
    assert_eq!(ws_dir.crate_count(), 1);

    // 3. Test collect_ridx_files ignores variants dir
    let variants_dir = test_dir.0.join("variants");
    fs::create_dir_all(&variants_dir).unwrap();
    let var_idx = variants_dir.join("ignored.ridx");
    let mut f2 = File::create(&var_idx).unwrap();
    writer.write_to(&mut f2).unwrap();

    let mut collected = Vec::new();
    collect_ridx_files(&test_dir.0, &mut collected).unwrap();
    assert_eq!(collected.len(), 1);
    assert_eq!(collected[0], idx);

    // 4. Auto-discover with RIDX_INDEX_DIR
    std::env::set_var("RIDX_INDEX_DIR", test_dir.0.to_str().unwrap());
    let ws_auto = WorkspaceIndex::auto_discover().unwrap();
    assert_eq!(ws_auto.crate_count(), 1);
    std::env::remove_var("RIDX_INDEX_DIR");
}

#[test]
fn test_open_with_variant_manifest_and_dependency_shards() {
    let _lock = ENV_LOCK.lock().unwrap();
    let test_dir = new_test_dir("ridx_open_vmanifest");
    let idx = test_dir.0.join("sample.ridx");
    let mut writer = IndexWriter::new("sample", "0.1.0");
    writer.set_unit_key("sample_uk");
    writer.set_generation("sample_gen");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "sample_fn".to_string(),
        path: "sample::sample_fn".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 16,
    });
    let mut f = File::create(&idx).unwrap();
    writer.write_to(&mut f).unwrap();
    drop(f);

    let ext_cache = test_dir.0.join("external_cache");
    fs::create_dir_all(&ext_cache).unwrap();
    let dep_idx = ext_cache.join("dep_crate.ridx");
    let mut dep_writer = IndexWriter::new("dep_crate", "0.1.0");
    dep_writer.set_unit_key("dep_uk");
    dep_writer.set_generation("dep_gen");
    dep_writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "dep_fn".to_string(),
        path: "dep_crate::dep_fn".to_string(),
        signature: None,
        file: "src/dep.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 16,
    });
    let mut f2 = File::create(&dep_idx).unwrap();
    dep_writer.write_to(&mut f2).unwrap();
    drop(f2);

    let mismatch_idx = ext_cache.join("mismatch_crate.ridx");
    let mut mismatch_writer = IndexWriter::new("mismatch_crate", "0.1.0");
    mismatch_writer.set_unit_key("mismatch_uk");
    mismatch_writer.set_generation("dep_gen");
    let mut f3 = File::create(&mismatch_idx).unwrap();
    mismatch_writer.write_to(&mut f3).unwrap();
    drop(f3);

    let v_manifest = test_dir.0.join("variant_manifest.json");
    let manifest_data = serde_json::json!({
        "version": "2.0",
        "format_version": crate::format::CURRENT_FORMAT_VERSION,
        "extractor_version": crate::format::EXTRACTOR_VERSION,
        "active_variant_id": "v1",
        "variant_dir": "variants/v1",
        "workspace_root": test_dir.0.display().to_string(),
        "dependency_shards": [
            {
                "name": "dep_crate",
                "version": "0.1.0",
                "unit_key": "dep_uk",
                "generation": "dep_gen",
                "object_path": dep_idx.display().to_string(),
                "is_dependency": true,
                "dependencies": ["some_dep"],
                "extern_units": {"some_dep": "some_uk"},
                "file_hashes": {"src/dep.rs": 1234}
            },
            {
                "name": "mismatch_crate",
                "version": "0.1.0",
                "unit_key": "mismatch_uk",
                "generation": "wrong_gen",
                "object_path": mismatch_idx.display().to_string(),
                "is_dependency": true,
                "dependencies": ["other_dep"],
                "extern_units": {"other_dep": "other_uk"},
                "file_hashes": {"src/dep.rs": 5678}
            }
        ]
    });
    fs::write(&v_manifest, serde_json::to_string(&manifest_data).unwrap()).unwrap();

    let ws = WorkspaceIndex::open(&test_dir.0).unwrap();
    assert_eq!(ws.crate_count(), 3);
}

#[test]
fn test_workspace_open_validation_errors() {
    let _lock = ENV_LOCK.lock().unwrap();
    let test_dir = new_test_dir("ridx_ws_errs");

    // 1. Invalid json in ridx_manifest.json
    let m_path = test_dir.0.join("ridx_manifest.json");
    fs::write(&m_path, "{ invalid_json }").unwrap();
    assert!(WorkspaceIndex::open(&test_dir.0).is_err());

    // 2. Extractor version mismatch in ridx_manifest.json
    let bad_ext = serde_json::json!({
        "version": "2.0",
        "format_version": crate::format::CURRENT_FORMAT_VERSION,
        "extractor_version": "9999",
        "active_variant_id": "v1",
        "variant_dir": "variants/v1",
    });
    let v1_dir = test_dir.0.join("variants/v1");
    fs::create_dir_all(&v1_dir).unwrap();
    fs::write(&m_path, serde_json::to_string(&bad_ext).unwrap()).unwrap();
    assert!(WorkspaceIndex::open(&test_dir.0).is_err());

    // 3. Extractor version missing in ridx_manifest.json
    let missing_ext = serde_json::json!({
        "version": "2.0",
        "format_version": crate::format::CURRENT_FORMAT_VERSION,
        "active_variant_id": "v1",
        "variant_dir": "variants/v1",
    });
    fs::write(&m_path, serde_json::to_string(&missing_ext).unwrap()).unwrap();
    assert!(WorkspaceIndex::open(&test_dir.0).is_err());

    // Clean up ridx_manifest.json
    fs::remove_file(&m_path).unwrap();

    // 4. variant_manifest.json errors
    let vm_path = test_dir.0.join("variant_manifest.json");
    fs::write(&vm_path, "{ invalid_json }").unwrap();
    assert!(WorkspaceIndex::open(&test_dir.0).is_err());

    // 5. format version mismatch in variant_manifest.json
    let bad_fmt = serde_json::json!({
        "version": "2.0",
        "format_version": 9999,
        "extractor_version": crate::format::EXTRACTOR_VERSION,
    });
    fs::write(&vm_path, serde_json::to_string(&bad_fmt).unwrap()).unwrap();
    assert!(WorkspaceIndex::open(&test_dir.0).is_err());

    // 6. extractor version mismatch in variant_manifest.json
    let bad_vm_ext = serde_json::json!({
        "version": "2.0",
        "format_version": crate::format::CURRENT_FORMAT_VERSION,
        "extractor_version": "9999",
    });
    fs::write(&vm_path, serde_json::to_string(&bad_vm_ext).unwrap()).unwrap();
    assert!(WorkspaceIndex::open(&test_dir.0).is_err());

    // 7. extractor version missing in variant_manifest.json
    let missing_vm_ext = serde_json::json!({
        "version": "2.0",
        "format_version": crate::format::CURRENT_FORMAT_VERSION,
    });
    fs::write(&vm_path, serde_json::to_string(&missing_vm_ext).unwrap()).unwrap();
    assert!(WorkspaceIndex::open(&test_dir.0).is_err());
}
