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
use crate::indexer::driver::*;
use crate::indexer::orchestrate::*;
use crate::indexer::types::*;
use crate::indexer::variant::*;
use std::fs;

#[test]
fn test_indexer_dependency_shards_and_cargo_config() {
    let _lock = lock_indexer_test();
    let temp = new_test_dir("ridx_dep_shards_test");
    let work_dir = temp.0.join("sample_pkg");
    let src_dir = work_dir.join("src");
    fs::create_dir_all(&src_dir).unwrap();

    let manifest_file = work_dir.join("Cargo.toml");
    fs::write(
        &manifest_file,
        "[package]\nname = \"sample_pkg\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(src_dir.join("lib.rs"), "pub fn sample() {}\n").unwrap();

    let host_target = std::process::Command::new("rustc")
        .args(["-vV"])
        .output()
        .ok()
        .and_then(|out| {
            let s = String::from_utf8_lossy(&out.stdout);
            s.lines()
                .find(|l| l.starts_with("host: "))
                .map(|l| l.trim_start_matches("host: ").trim().to_string())
        })
        .unwrap_or_else(|| "x86_64-unknown-linux-gnu".to_string());

    // 1. Cargo config with build target (lines 551-560)
    let dot_cargo = work_dir.join(".cargo");
    fs::create_dir_all(&dot_cargo).unwrap();
    fs::write(
        dot_cargo.join("config.toml"),
        format!("[build]\ntarget = \"{}\"\n", host_target),
    )
    .unwrap();

    let out_dir = temp.0.join("target_ridx");
    let opts = IndexOptions {
        path: work_dir.clone(),
        out_dir_opt: Some(out_dir.clone()),
        package: None,
        release: false,
        features: None,
        no_default_features: false,
        all_features: false,
        target: None,
        cache_dir: None,
        driver_bin: None,
    };

    // Create mock dependency shard
    let dep_file = temp.0.join("dep.ridx");
    let mut w_dep = ridx_format::IndexWriter::new("dep_crate", "0.1.0");
    w_dep.set_unit_key("dep_crate_key");
    w_dep.set_generation("gen_alpha");
    w_dep.add_file_hash("src/lib.rs", 12345);
    let mut f_dep = fs::File::create(&dep_file).unwrap();
    w_dep.write_to(&mut f_dep).unwrap();
    drop(f_dep);

    // Prepopulate .deps directory with DependencyShardInfo (lines 506-548)
    let variant_id = compute_variant_id(
        "dev",
        Some(&host_target),
        None,
        None,
        false,
        false,
        &manifest_file,
    );
    let variant_out_dir = out_dir.join(format!("variants/{}", variant_id));
    let deps_dir = variant_out_dir.join(".deps");
    fs::create_dir_all(&deps_dir).unwrap();

    let dep_info = ridx_format::DependencyShardInfo {
        name: "dep_crate".to_string(),
        version: "0.1.0".to_string(),
        unit_key: "dep_crate_key".to_string(),
        generation: Some("gen_alpha".to_string()),
        object_path: dep_file.display().to_string(),
        source_root: Some(temp.0.display().to_string()),
        target_kind: None,
        is_dependency: true,
        dependencies: vec![],
        extern_units: std::collections::HashMap::new(),
        file_hashes: std::collections::HashMap::new(),
        stable_crate_id: None,
        caught_panics: None,
        ..Default::default()
    };
    fs::write(
        deps_dir.join("dep_crate.json"),
        serde_json::to_string(&dep_info).unwrap(),
    )
    .unwrap();

    // Shard with generation mismatch (lines 526-528)
    let dep_info_mismatch = ridx_format::DependencyShardInfo {
        name: "dep_mismatch".to_string(),
        version: "0.1.0".to_string(),
        unit_key: "dep_crate_key".to_string(),
        generation: Some("mismatched_gen".to_string()),
        object_path: dep_file.display().to_string(),
        source_root: Some(temp.0.display().to_string()),
        target_kind: None,
        is_dependency: true,
        dependencies: vec!["some_dep".to_string()],
        extern_units: std::collections::HashMap::new(),
        file_hashes: std::collections::HashMap::new(),
        stable_crate_id: None,
        caught_panics: None,
        ..Default::default()
    };
    fs::write(
        deps_dir.join("dep_mismatch.json"),
        serde_json::to_string(&dep_info_mismatch).unwrap(),
    )
    .unwrap();

    // Shard with None generation and empty file_hashes (line 523)
    let dep_info_empty_hashes = ridx_format::DependencyShardInfo {
        name: "dep_empty_h".to_string(),
        version: "0.1.0".to_string(),
        unit_key: "dep_crate_key".to_string(),
        generation: None,
        object_path: dep_file.display().to_string(),
        source_root: Some(temp.0.display().to_string()),
        target_kind: None,
        is_dependency: true,
        dependencies: vec![],
        extern_units: std::collections::HashMap::new(),
        file_hashes: std::collections::HashMap::new(),
        stable_crate_id: None,
        caught_panics: None,
        ..Default::default()
    };
    fs::write(
        deps_dir.join("dep_empty_h.json"),
        serde_json::to_string(&dep_info_empty_hashes).unwrap(),
    )
    .unwrap();

    // Shard with generation: None and computed digest matching bin_gen (lines 518-525)
    let mut f_hashes = std::collections::HashMap::new();
    f_hashes.insert("src/lib.rs".to_string(), 12345u64);
    let computed_digest = ridx_format::compute_source_snapshot_digest(&f_hashes);
    let computed_gen = format!("{:016x}", computed_digest);

    let dep_matching_file = temp.0.join("dep_matching.ridx");
    let mut w_matching = ridx_format::IndexWriter::new("dep_matching", "0.1.0");
    w_matching.set_unit_key("dep_matching_key");
    w_matching.set_generation(&computed_gen);
    let mut fm = fs::File::create(&dep_matching_file).unwrap();
    w_matching.write_to(&mut fm).unwrap();
    drop(fm);

    let dep_info_matching = ridx_format::DependencyShardInfo {
        name: "dep_matching".to_string(),
        version: "0.1.0".to_string(),
        unit_key: "dep_matching_key".to_string(),
        generation: None,
        object_path: dep_matching_file.display().to_string(),
        source_root: Some(temp.0.display().to_string()),
        target_kind: None,
        is_dependency: true,
        dependencies: vec![],
        extern_units: std::collections::HashMap::new(),
        file_hashes: f_hashes.clone(),
        stable_crate_id: None,
        caught_panics: None,
        ..Default::default()
    };
    fs::write(
        deps_dir.join("dep_matching.json"),
        serde_json::to_string(&dep_info_matching).unwrap(),
    )
    .unwrap();

    // Shard with generation: None and computed digest NOT matching bin_gen (lines 514-518)
    let dep_mismatch_gen_file = temp.0.join("dep_mismatch_gen.ridx");
    let mut w_mismatch_gen = ridx_format::IndexWriter::new("dep_mismatch_gen", "0.1.0");
    w_mismatch_gen.set_unit_key("dep_mismatch_gen_key");
    w_mismatch_gen.set_generation("some_other_bin_gen");
    let mut f_mg = fs::File::create(&dep_mismatch_gen_file).unwrap();
    w_mismatch_gen.write_to(&mut f_mg).unwrap();
    drop(f_mg);

    let dep_info_mismatch_gen = ridx_format::DependencyShardInfo {
        name: "dep_mismatch_gen".to_string(),
        version: "0.1.0".to_string(),
        unit_key: "dep_mismatch_gen_key".to_string(),
        generation: None,
        object_path: dep_mismatch_gen_file.display().to_string(),
        source_root: Some(temp.0.display().to_string()),
        target_kind: None,
        is_dependency: true,
        dependencies: vec!["some_dep".to_string()],
        extern_units: std::collections::HashMap::new(),
        file_hashes: f_hashes.clone(),
        stable_crate_id: None,
        caught_panics: None,
        ..Default::default()
    };
    fs::write(
        deps_dir.join("dep_mismatch_gen.json"),
        serde_json::to_string(&dep_info_mismatch_gen).unwrap(),
    )
    .unwrap();

    // Shard with is_dependency: false (lines 544-546)
    let dep_info_not_dep = ridx_format::DependencyShardInfo {
        name: "workspace_shard".to_string(),
        version: "0.1.0".to_string(),
        unit_key: "ws_shard_key".to_string(),
        generation: None,
        object_path: dep_file.display().to_string(),
        source_root: Some(temp.0.display().to_string()),
        target_kind: None,
        is_dependency: false,
        dependencies: vec![],
        extern_units: std::collections::HashMap::new(),
        file_hashes: std::collections::HashMap::new(),
        stable_crate_id: None,
        caught_panics: None,
        ..Default::default()
    };
    fs::write(
        deps_dir.join("dep_not_dep.json"),
        serde_json::to_string(&dep_info_not_dep).unwrap(),
    )
    .unwrap();

    // Shard with nonexistent object_path (lines 540-541)
    let dep_info_missing_obj = ridx_format::DependencyShardInfo {
        name: "dep_missing_obj".to_string(),
        version: "0.1.0".to_string(),
        unit_key: "missing_obj_key".to_string(),
        generation: None,
        object_path: "/nonexistent/object.ridx".to_string(),
        source_root: Some(temp.0.display().to_string()),
        target_kind: None,
        is_dependency: true,
        dependencies: vec![],
        extern_units: std::collections::HashMap::new(),
        file_hashes: std::collections::HashMap::new(),
        stable_crate_id: None,
        caught_panics: None,
        ..Default::default()
    };
    fs::write(
        deps_dir.join("dep_missing_obj.json"),
        serde_json::to_string(&dep_info_missing_obj).unwrap(),
    )
    .unwrap();

    // Non-json file in .deps (lines 548-549)
    fs::write(deps_dir.join("ignored.txt"), b"not json").unwrap();
    fs::write(deps_dir.join("invalid.json"), b"invalid json").unwrap();

    let _ld_guard = EnvGuard::set("LD_LIBRARY_PATH", "/usr/lib");
    let _dyld_guard = EnvGuard::set("DYLD_FALLBACK_LIBRARY_PATH", "/usr/local/lib");

    if find_driver_binary().is_some() {
        let res = try_run_indexing(opts.clone());
        assert!(res.is_ok());
        let published = res.unwrap();
        let mf = fs::read_to_string(published.base_out_dir.join("ridx_manifest.json")).unwrap();
        assert!(mf.contains("dep_crate"));
        assert!(mf.contains("dep_mismatch"));
        assert!(mf.contains("dep_matching"));

        // Test atomic manifest write temp file failure (lines 610-616)
        let tmp_manifest_blocker =
            out_dir.join(format!("ridx_manifest.json.tmp.{}", std::process::id()));
        if fs::create_dir_all(&tmp_manifest_blocker).is_ok() {
            let err_tmp = try_run_indexing(opts.clone());
            assert!(err_tmp.is_err());
            assert!(err_tmp
                .unwrap_err()
                .contains("failed to write temporary variant manifest"));
            let _ = fs::remove_dir_all(&tmp_manifest_blocker);
        }

        // Test atomic manifest publish rename error (lines 618-622)
        let manifest_dest = out_dir.join("ridx_manifest.json");
        let _ = fs::remove_file(&manifest_dest);
        if fs::create_dir_all(manifest_dest.join("nested_blocker")).is_ok() {
            let err_res = try_run_indexing(opts);
            assert!(err_res.is_err());
            assert!(err_res
                .unwrap_err()
                .contains("failed to publish active variant manifest"));
            let _ = fs::remove_dir_all(&manifest_dest);
        }
    }
}
