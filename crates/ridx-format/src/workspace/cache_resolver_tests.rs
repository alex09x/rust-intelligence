/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Unit tests for manifest-relative target classification and resilient dependency cache lookup.

use super::cache_resolver::CacheObjectResolver;
use super::manifest::DependencyShardInfo;
use super::target_kind::{classify_target_with_manifest, TargetKind};
use crate::writer::IndexWriter;
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::Path;

#[test]
fn test_classification_manifest_relative_fallback() {
    let manifest_dir = Path::new("/workspace/crates/my_crate");

    // 1. Bench relative to manifest
    let args = vec!["rustc".to_string(), "benches/main.rs".to_string()];
    assert_eq!(
        classify_target_with_manifest(None, &args, Some(manifest_dir)),
        TargetKind::Bench
    );

    // 2. Example relative to manifest
    let args = vec!["rustc".to_string(), "examples/demo.rs".to_string()];
    assert_eq!(
        classify_target_with_manifest(None, &args, Some(manifest_dir)),
        TargetKind::Example
    );

    // 3. Test relative to manifest
    let args = vec!["rustc".to_string(), "tests/integration.rs".to_string()];
    assert_eq!(
        classify_target_with_manifest(None, &args, Some(manifest_dir)),
        TargetKind::Test
    );

    // 4. Build script
    let args = vec!["rustc".to_string(), "build.rs".to_string()];
    assert_eq!(
        classify_target_with_manifest(None, &args, Some(manifest_dir)),
        TargetKind::CustomBuild
    );

    // 5. Absolute bench path stripped relative to manifest_dir
    let abs_bench = "/workspace/crates/my_crate/benches/bench_ops.rs".to_string();
    let args = vec!["rustc".to_string(), abs_bench];
    assert_eq!(
        classify_target_with_manifest(None, &args, Some(manifest_dir)),
        TargetKind::Bench
    );

    // 6. Primary library unit
    let args = vec!["rustc".to_string(), "src/lib.rs".to_string()];
    assert_eq!(
        classify_target_with_manifest(None, &args, Some(manifest_dir)),
        TargetKind::Lib
    );
}

#[test]
fn test_resilient_cache_resolver_finds_shard_by_generation_and_version() {
    let tmp = tempfile::tempdir().unwrap();
    let cache_dir = tmp.path().join("cache");
    let objects_dir = cache_dir.join("objects").join("ab");
    fs::create_dir_all(&objects_dir).unwrap();

    let shard_file = objects_dir.join("ab1234567890cdef.ridx");
    {
        let mut writer = IndexWriter::new("my_dep", "1.2.3");
        writer.set_unit_key("ab1234567890cdef");
        writer.set_generation("gen_alpha_42");
        let f = File::create(&shard_file).unwrap();
        writer.write_to(&mut BufWriter::new(f)).unwrap();
    }

    let mut resolver = CacheObjectResolver::new(cache_dir);

    // Case 1: Stale object_path and stale unit_key from older driver run
    let dep = DependencyShardInfo {
        name: "my_dep".to_string(),
        version: "1.2.3".to_string(),
        unit_key: "nonexistent_stale_key".to_string(),
        object_path: "/stale/path/to/missing.ridx".to_string(),
        generation: Some("gen_alpha_42".to_string()),
        source_root: None,
        target_kind: Some(TargetKind::Lib),
        is_dependency: true,
        dependencies: Vec::new(),
        extern_units: std::collections::HashMap::new(),
        file_hashes: std::collections::HashMap::new(),
        stable_crate_id: None,
        caught_panics: None,
        compiler_args: Vec::new(),
        env: std::collections::HashMap::new(),
    };

    let resolved = resolver.resolve(&dep);
    assert_eq!(resolved, Some(shard_file.clone()));

    // Case 2: Reject when generation is omitted (strict generation match required)
    let dep_no_gen = DependencyShardInfo {
        generation: None,
        ..dep.clone()
    };
    assert_eq!(resolver.resolve(&dep_no_gen), None);

    // Case 3: Reject when generation is mismatched
    let dep_mismatched_gen = DependencyShardInfo {
        generation: Some("different_gen_digest".to_string()),
        ..dep.clone()
    };
    assert_eq!(resolver.resolve(&dep_mismatched_gen), None);

    // Case 4: Completely unknown crate name -> None
    let dep_unknown = DependencyShardInfo {
        name: "unknown_crate".to_string(),
        ..dep
    };
    assert_eq!(resolver.resolve(&dep_unknown), None);
}

#[test]
fn test_cache_resolver_rejects_mismatched_version() {
    let tmp = tempfile::tempdir().unwrap();
    let cache_dir = tmp.path().join("cache");
    let objects_dir = cache_dir.join("objects").join("ab");
    fs::create_dir_all(&objects_dir).unwrap();

    let shard_file = objects_dir.join("ab1234567890cdef.ridx");
    {
        // Cache contains version 0.3.0
        let mut writer = IndexWriter::new("my_dep", "0.3.0");
        writer.set_unit_key("ab1234567890cdef");
        writer.set_generation("gen_alpha_42");
        let f = File::create(&shard_file).unwrap();
        writer.write_to(&mut BufWriter::new(f)).unwrap();
    }

    let mut resolver = CacheObjectResolver::new(cache_dir);

    // Request dependency specifying version 0.4.0
    let dep_diff_version = DependencyShardInfo {
        name: "my_dep".to_string(),
        version: "0.4.0".to_string(),
        unit_key: "nonexistent_stale_key".to_string(),
        object_path: "/stale/path/to/missing.ridx".to_string(),
        generation: None,
        source_root: None,
        target_kind: Some(TargetKind::Lib),
        is_dependency: true,
        dependencies: Vec::new(),
        extern_units: std::collections::HashMap::new(),
        file_hashes: std::collections::HashMap::new(),
        stable_crate_id: None,
        caught_panics: None,
        compiler_args: Vec::new(),
        env: std::collections::HashMap::new(),
    };

    // Resolver must reject mismatched version and return None, never substituting 0.3.0 for 0.4.0
    assert_eq!(resolver.resolve(&dep_diff_version), None);
}

#[test]
fn test_cache_resolver_allows_resilient_shard_when_unit_key_differs_but_generation_matches() {
    let tmp = tempfile::tempdir().unwrap();
    let cache_dir = tmp.path().join("cache");
    let objects_dir = cache_dir.join("objects").join("ab");
    fs::create_dir_all(&objects_dir).unwrap();

    let shard_file = objects_dir.join("ab1234567890cdef.ridx");
    {
        let mut writer = IndexWriter::new("my_dep", "1.2.3");
        writer.set_unit_key("ab1234567890cdef");
        writer.set_generation("gen_alpha_42");
        let f = File::create(&shard_file).unwrap();
        writer.write_to(&mut BufWriter::new(f)).unwrap();
    }

    let mut resolver = CacheObjectResolver::new(cache_dir);

    // Stale object_path and stale unit_key from older driver run, but generation strictly matches
    let dep = DependencyShardInfo {
        name: "my_dep".to_string(),
        version: "1.2.3".to_string(),
        unit_key: "different_unit_key_9999".to_string(),
        object_path: "/stale/path/to/missing.ridx".to_string(),
        generation: Some("gen_alpha_42".to_string()),
        source_root: None,
        target_kind: Some(TargetKind::Lib),
        is_dependency: true,
        dependencies: Vec::new(),
        extern_units: std::collections::HashMap::new(),
        file_hashes: std::collections::HashMap::new(),
        stable_crate_id: None,
        caught_panics: None,
        compiler_args: Vec::new(),
        env: std::collections::HashMap::new(),
    };

    let resolved = resolver.resolve(&dep);
    assert_eq!(resolved, Some(shard_file.clone()));

    // Open resolved shard with IndexReader to verify reader properties
    let reader = crate::reader::IndexReader::open(&shard_file).unwrap();
    assert_eq!(reader.crate_name(), "my_dep");
    assert_eq!(reader.crate_version(), "1.2.3");
    assert_ne!(reader.unit_key(), dep.unit_key);
    assert_eq!(reader.generation(), dep.generation.as_deref().unwrap());
}
