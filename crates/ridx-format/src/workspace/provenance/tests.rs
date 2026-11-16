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
mod edge_cases;
mod units_agree;

use super::*;
use crate::writer::IndexWriter;
use std::collections::HashMap;
use std::fs::File;

struct TestDir(PathBuf);
impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn new_test_dir(prefix: &str) -> TestDir {
    static CNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let id = CNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!("{}_{}_{}", prefix, std::process::id(), id));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    TestDir(path)
}

#[test]
fn test_normalize_path_variants() {
    assert_eq!(normalize_path(Path::new("a/b/../c")), PathBuf::from("a/c"));
    assert_eq!(normalize_path(Path::new("./a/./b")), PathBuf::from("a/b"));
    assert_eq!(normalize_path(Path::new("/a/b/../../c")), PathBuf::from("/c"));
    assert_eq!(normalize_path(Path::new("")), PathBuf::from(""));
    assert_eq!(normalize_path(Path::new("a/../../b")), PathBuf::from("../b"));
}

#[test]
fn test_path_traverses_symlink_parent() {
    assert!(!path_traverses_symlink_parent(Path::new("/tmp/normal/path")));
    assert!(!path_traverses_symlink_parent(Path::new("relative/path/../foo")));
}

#[test]
fn test_file_entry_matches_empty_and_lexical() {
    assert!(!file_entry_matches("", None, Path::new("/tmp/test"), None));
    assert!(file_entry_matches("/tmp/foo", None, Path::new("/tmp/foo"), None));
    assert!(!file_entry_matches("/tmp/foo", None, Path::new("/tmp/bar"), None));

    let rel_entry = "src/lib.rs";
    let base = Path::new("/workspace");
    assert!(file_entry_matches(rel_entry, Some(base), Path::new("/workspace/src/lib.rs"), None));
}

#[test]
fn test_shard_mtime_and_source_hash_for_file() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_prov_test_{}", std::process::id()));
    let _ = fs::create_dir_all(&temp_dir);
    let src_file = temp_dir.join("src/lib.rs");
    let _ = fs::create_dir_all(src_file.parent().unwrap());
    let _ = fs::write(&src_file, b"pub fn hello() {}\n");

    let mut writer = crate::writer::IndexWriter::new("prov_crate", "0.1.0");
    let hash = crate::format::normalized_source_hash(b"pub fn hello() {}\n");
    let mut map = HashMap::new();
    map.insert(src_file.to_str().unwrap().to_string(), hash);
    writer.set_file_hashes(map);
    writer.add_definition(crate::writer::RawDefinition {
        def_id: 1,
        kind: crate::format::DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "hello".to_string(),
        path: "prov_crate::hello".to_string(),
        signature: None,
        file: src_file.to_str().unwrap().to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 16,
    });
    let ridx_path = temp_dir.join("prov.ridx");
    let mut f = fs::File::create(&ridx_path).unwrap();
    writer.write_to(&mut f).unwrap();
    drop(f);

    let ws = WorkspaceIndex::open(&ridx_path).unwrap();
    assert_eq!(ws.source_hash_for_file(src_file.to_str().unwrap()), Some(hash));
    assert!(ws.shard_mtime_for_file(src_file.to_str().unwrap()).is_some());
    assert_eq!(ws.source_hash_for_file("/nonexistent/file.rs"), None);
    assert_eq!(ws.shard_mtime_for_file("/nonexistent/file.rs"), None);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_shard_mtime_and_source_hash_with_dependency_source_root() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_prov_dep_{}", std::process::id()));
    let _ = fs::create_dir_all(&temp_dir);

    let dep_root = temp_dir.join("dep_src");
    let _ = fs::create_dir_all(&dep_root);
    let dep_file = dep_root.join("dep.rs");
    let _ = fs::write(&dep_file, b"pub fn dep() {}\n");

    let mut dep_writer = crate::writer::IndexWriter::new("dep_lib", "0.1.0");
    dep_writer.set_unit_key("dep_lib:0.1.0");
    dep_writer.set_generation("1111");
    let hash = crate::format::normalized_source_hash(b"pub fn dep() {}\n");
    let mut map = HashMap::new();
    map.insert("dep.rs".to_string(), hash);
    dep_writer.set_file_hashes(map);
    dep_writer.add_definition(crate::writer::RawDefinition {
        def_id: 1,
        kind: crate::format::DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "dep".to_string(),
        path: "dep_lib::dep".to_string(),
        signature: None,
        file: "dep.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 10,
    });

    let cache_dir = temp_dir.join("cache");
    let _ = fs::create_dir_all(&cache_dir);
    let dep_ridx = cache_dir.join("dep.ridx");
    let mut f = fs::File::create(&dep_ridx).unwrap();
    dep_writer.write_to(&mut f).unwrap();
    drop(f);

    let v_manifest = temp_dir.join("variant_manifest.json");
    let manifest_data = serde_json::json!({
        "version": "2.0",
        "format_version": crate::format::CURRENT_FORMAT_VERSION,
        "extractor_version": crate::format::EXTRACTOR_VERSION,
        "active_variant_id": "v1",
        "variant_dir": "variants/v1",
        "workspace_root": temp_dir.display().to_string(),
        "dependency_shards": [{
            "name": "dep_lib",
            "version": "0.1.0",
            "unit_key": "dep_lib:0.1.0",
            "generation": "1111",
            "object_path": dep_ridx.display().to_string(),
            "source_root": dep_root.display().to_string(),
            "is_dependency": true,
            "dependencies": [],
            "extern_units": {},
            "file_hashes": { "dep.rs": hash }
        }]
    });
    fs::write(&v_manifest, serde_json::to_string(&manifest_data).unwrap()).unwrap();

    let ws = WorkspaceIndex::open(&temp_dir).unwrap();

    // Query with absolute path to dep.rs
    let full_dep_path = dep_file.display().to_string();
    assert_eq!(ws.source_hash_for_file(&full_dep_path), Some(hash));
    assert!(ws.shard_mtime_for_file(&full_dep_path).is_some());

    // Query with relative path when ws has workspace_root
    assert_eq!(ws.source_hash_for_file("dep_src/dep.rs"), Some(hash));
    assert!(ws.shard_mtime_for_file("dep_src/dep.rs").is_some());

    let _ = fs::remove_dir_all(&temp_dir);
}
