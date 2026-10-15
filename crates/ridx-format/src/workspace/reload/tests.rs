/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

mod coverage;

use super::*;
use crate::format::DefKind;
use crate::writer::{IndexWriter, RawDefinition};
use std::fs::{self, File};

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

pub(super) fn write_test_index(path: &Path, crate_name: &str) {
    let mut writer = IndexWriter::new(crate_name, "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "test_fn".to_string(),
        path: format!("{crate_name}::test_fn"),
        signature: Some("fn()".to_string()),
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 2,
        end_col: 2,
    });
    let mut f = File::create(path).unwrap();
    writer.write_to(&mut f).unwrap();
}

#[test]
fn test_reload_format_and_extractor_version_errors() {
    let test_dir = new_test_dir("ridx_reload_err");
    let idx = test_dir.0.join("sample.ridx");
    write_test_index(&idx, "sample");

    let mut ws = WorkspaceIndex::open(&test_dir.0).unwrap();

    let mf = test_dir.0.join("ridx_manifest.json");
    let manifest_data = serde_json::json!({
        "version": "2.0",
        "format_version": 999,
        "active_variant_id": "var1",
        "variant_dir": "variants/var1",
        "workspace_root": test_dir.0.display().to_string(),
    });
    fs::write(&mf, serde_json::to_string(&manifest_data).unwrap()).unwrap();

    let err = ws.reload_if_modified();
    assert!(err.is_err());
    assert!(err
        .unwrap_err()
        .to_string()
        .contains("Format version mismatch"));

    // Extractor version mismatch
    let manifest_data_ext = serde_json::json!({
        "version": "2.0",
        "format_version": crate::format::CURRENT_FORMAT_VERSION,
        "extractor_version": "non_matching_v99",
        "active_variant_id": "var1",
        "variant_dir": "variants/var1",
        "workspace_root": test_dir.0.display().to_string(),
    });
    fs::write(&mf, serde_json::to_string(&manifest_data_ext).unwrap()).unwrap();
    ws.manifest_mtime = None; // Reset cached mtime to force re-reading manifest

    let err2 = ws.reload_if_modified();
    assert!(err2.is_err());
    assert!(err2
        .unwrap_err()
        .to_string()
        .contains("Extractor version mismatch"));
}

#[test]
fn test_reload_file_modifications_and_deletions() {
    let test_dir = new_test_dir("ridx_reload_mod");
    let idx1 = test_dir.0.join("crate1.ridx");
    write_test_index(&idx1, "crate1");

    let mut ws = WorkspaceIndex::open(&test_dir.0).unwrap();
    assert_eq!(ws.readers.len(), 1);

    // Immediate reload without changes returns false
    assert!(!ws.reload_if_modified().unwrap());

    // Overwrite file
    std::thread::sleep(std::time::Duration::from_millis(50));
    write_test_index(&idx1, "crate1");
    assert!(ws.reload_if_modified().unwrap());
    assert!(ws.is_file_reloaded("src/lib.rs"));

    // Add new file
    let idx2 = test_dir.0.join("crate2.ridx");
    write_test_index(&idx2, "crate2");
    assert!(ws.reload_if_modified().unwrap());
    assert_eq!(ws.readers.len(), 2);

    // Remove file
    fs::remove_file(&idx2).unwrap();
    assert!(ws.reload_if_modified().unwrap());
    assert_eq!(ws.readers.len(), 1);
}

#[test]
fn test_reload_active_variant_switch() {
    let test_dir = new_test_dir("ridx_reload_switch");
    let v1_dir = test_dir.0.join("variants/var1");
    fs::create_dir_all(&v1_dir).unwrap();
    write_test_index(&v1_dir.join("crate1.ridx"), "crate1");

    let v2_dir = test_dir.0.join("variants/var2");
    fs::create_dir_all(&v2_dir).unwrap();
    write_test_index(&v2_dir.join("crate2.ridx"), "crate2");

    let mf = test_dir.0.join("ridx_manifest.json");
    let manifest_data = serde_json::json!({
        "version": "2.0",
        "format_version": crate::format::CURRENT_FORMAT_VERSION,
        "extractor_version": crate::format::EXTRACTOR_VERSION,
        "active_variant_id": "var1",
        "variant_dir": "variants/var1",
        "workspace_root": test_dir.0.display().to_string(),
    });
    fs::write(&mf, serde_json::to_string(&manifest_data).unwrap()).unwrap();

    let mut ws = WorkspaceIndex::open(&v1_dir).unwrap();
    ws.manifest_path = Some(mf.clone());
    ws.active_variant_id = Some("var1".to_string());

    // Switch to var2 with dependency shard
    let dep_file = test_dir.0.join("dep_crate.ridx");
    write_test_index(&dep_file, "dep_crate");

    std::thread::sleep(std::time::Duration::from_millis(50));
    let manifest_data2 = serde_json::json!({
        "version": "2.0",
        "format_version": crate::format::CURRENT_FORMAT_VERSION,
        "extractor_version": crate::format::EXTRACTOR_VERSION,
        "active_variant_id": "var2",
        "variant_dir": "variants/var2",
        "workspace_root": test_dir.0.display().to_string(),
        "dependency_shards": [{
            "name": "dep_crate",
            "version": "0.1.0",
            "unit_key": "dep_crate:0.1.0",
            "generation": "1234567812345678",
            "object_path": dep_file.display().to_string(),
            "is_dependency": true,
            "dependencies": [],
            "extern_units": {},
            "file_hashes": {}
        }]
    });
    fs::write(&mf, serde_json::to_string(&manifest_data2).unwrap()).unwrap();

    let res = ws.reload_if_modified().unwrap();
    assert!(res);
    assert_eq!(ws.active_variant_id.as_deref(), Some("var2"));
    assert_eq!(ws.readers.len(), 2);
}

#[test]
fn test_reload_with_metadata_sidecar() {
    let test_dir = new_test_dir("ridx_reload_sidecar");
    let idx = test_dir.0.join("crate1.ridx");
    write_test_index(&idx, "crate1");

    let deps_dir = test_dir.0.join(".deps");
    fs::create_dir_all(&deps_dir).unwrap();
    let meta_file = deps_dir.join("crate1.json");
    let meta_data = serde_json::json!({
        "name": "crate1",
        "is_dependency": false,
        "source_root": test_dir.0.display().to_string(),
        "object_path": idx.display().to_string(),
        "dependencies": ["dep_a"],
        "extern_units": ["ext_u1"]
    });
    fs::write(&meta_file, serde_json::to_string(&meta_data).unwrap()).unwrap();

    let mut ws = WorkspaceIndex::open(&test_dir.0).unwrap();
    assert_eq!(ws.readers.len(), 1);

    // Update sidecar metadata
    std::thread::sleep(std::time::Duration::from_millis(50));
    let meta_data2 = serde_json::json!({
        "name": "crate1",
        "is_dependency": false,
        "source_root": test_dir.0.display().to_string(),
        "object_path": idx.display().to_string(),
        "dependencies": ["dep_a", "dep_b"],
        "extern_units": ["ext_u1"]
    });
    fs::write(&meta_file, serde_json::to_string(&meta_data2).unwrap()).unwrap();

    let res = ws.reload_if_modified().unwrap();
    assert!(res);
}

#[test]
fn test_reload_new_file_discovery_and_is_reloaded() {
    let test_dir = new_test_dir("ridx_reload_new_file");
    let idx1 = test_dir.0.join("crate1.ridx");
    write_test_index(&idx1, "crate1");

    let mut ws = WorkspaceIndex::open(&test_dir.0).unwrap();
    assert_eq!(ws.readers.len(), 1);

    // Before adding new index:
    assert!(!ws.is_file_reloaded("src/lib.rs"));

    // Add crate2.ridx to directory
    std::thread::sleep(std::time::Duration::from_millis(50));
    let idx2 = test_dir.0.join("crate2.ridx");
    write_test_index(&idx2, "crate2");

    let changed = ws.reload_if_modified().unwrap();
    assert!(changed);
    assert_eq!(ws.readers.len(), 2);
    assert!(ws.is_file_reloaded("src/lib.rs"));
}
