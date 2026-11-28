/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Seeding a new variant from an earlier one when a package has many targets (Bevy's root: 1,391 sidecars).
//! The earlier variant's group was checked once per sidecar it holds, each check reading every tracked file: hours
//! of reading before `cargo check` started on the second index of a workspace without a committed `Cargo.lock`.

use super::*;
use crate::indexer::feature_graph::{seed_unaffected_and_prune_invalidated, ResolvedFeatureGraph, ResolvedPackageNode};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const TARGETS: usize = 300;
const TGT: &str = "x86_64-unknown-linux-gnu";

/// A workspace package with `TARGETS` examples, each with a sidecar in variant `v_prev` tracking the same three files.
/// `stale` gives one sidecar a hash the files no longer have.
fn many_targets(name: &str, stale: bool) -> (TestDir, PathBuf, PathBuf, PathBuf, ResolvedFeatureGraph) {
    let test_dir = new_test_dir(name);
    let root = test_dir.0.clone();
    let pkg = root.join("pkg");
    let base_out = root.join("out");
    let prev = base_out.join("variants/v_prev");
    let cur = base_out.join("variants/v_cur");
    let cargo_tgt = root.join("cargo_tgt");
    for d in [pkg.join("src"), pkg.join("examples"), prev.join(".deps"), cur.clone(), cargo_tgt.clone()] {
        fs::create_dir_all(d).unwrap();
    }
    let pkg = pkg.canonicalize().unwrap();
    let files = ["src/lib.rs", "src/a.rs", "src/b.rs"];
    for f in files {
        fs::write(pkg.join(f), format!("// {f}\npub fn f() {{}}\n")).unwrap();
    }
    // Every source file before any shard: a file newer than a shard counts as one added since that build.
    for i in 0..TARGETS {
        fs::write(pkg.join(format!("examples/ex{i}.rs")), format!("fn main() {{ let _ = {i}; }}\n")).unwrap();
    }
    let hash = |f: &str| ridx_format::normalized_source_hash(&fs::read(pkg.join(f)).unwrap());
    let vm = serde_json::json!({
        "version": "2.0", "format_version": ridx_format::CURRENT_FORMAT_VERSION, "generation": 1,
        "active_variant_id": "v_prev", "variant_dir": "variants/v_prev", "profile": "dev",
        "target": serde_json::Value::Null, "crate_features": { "pkg": [] }
    });
    fs::write(prev.join("variant_manifest.json"), serde_json::to_vec(&vm).unwrap()).unwrap();
    for i in 0..TARGETS {
        let ex = format!("examples/ex{i}.rs");
        let shard = prev.join(format!("ex{i}.ridx"));
        fs::write(&shard, b"ridx-stub").unwrap();
        let mut fh = serde_json::Map::new();
        for f in files {
            let h = if stale && i == TARGETS / 2 && f == "src/b.rs" { hash(f) ^ 1 } else { hash(f) };
            fh.insert(pkg.join(f).display().to_string(), h.into());
        }
        fh.insert(pkg.join(&ex).display().to_string(), hash(&ex).into());
        let sc = serde_json::json!({
            "name": "pkg", "version": "0.1.0", "source_root": pkg.display().to_string(), "object_path": shard.display().to_string(),
            "is_dependency": false, "target": TGT, "enabled_features": [], "file_hashes": fh,
        });
        fs::write(prev.join(format!(".deps/ex{i}.json")), serde_json::to_vec(&sc).unwrap()).unwrap();
    }
    let node = ResolvedPackageNode {
        id: "pkg".into(),
        name: "pkg".into(),
        crate_name: "pkg".into(),
        version: "0.1.0".into(),
        feature_key: "pkg".into(),
        manifest_dir: pkg.clone(),
        is_workspace_member: true,
        enabled_features: vec![],
        normal_or_build_deps: vec![],
        all_deps: vec![],
        feature_activated_deps: vec![],
    };
    let graph = ResolvedFeatureGraph {
        effective_target: TGT.into(),
        host_triple: Some(TGT.into()),
        nodes: BTreeMap::from([("pkg".to_string(), node)]),
        active_ids: BTreeSet::from(["pkg".to_string()]),
        crate_features: BTreeMap::from([("pkg".to_string(), vec![])]),
    };
    (test_dir, base_out, prev, cur, graph)
}

fn seed(base_out: &Path, prev: &Path, cur: &Path, graph: &ResolvedFeatureGraph) -> crate::indexer::feature_graph::SeedSummary {
    seed_unaffected_and_prune_invalidated(graph, base_out, cur, Some(prev), &base_out.join("cargo_tgt"), "dev", None)
}

#[test]
fn test_a_package_with_many_targets_is_seeded_reading_each_file_once() {
    let _lock = lock_indexer_test();
    let (_dir, base_out, prev, cur, graph) = many_targets("ridx_seed_many_targets", false);
    let s = seed(&base_out, &prev, &cur, &graph);
    assert!(s.seeded_pkg_ids.contains("pkg"), "{s:?}");
    assert_eq!(fs::read_dir(cur.join(".deps")).unwrap().count(), TARGETS);
    assert_eq!(s.source_reads, 3 + TARGETS, "the three shared files and each example once");
    assert_eq!(s.sidecar_checks, TARGETS, "each sidecar once");
}

#[test]
fn test_a_stale_sidecar_among_many_targets_invalidates_the_package_after_one_pass() {
    let _lock = lock_indexer_test();
    let (_dir, base_out, prev, cur, graph) = many_targets("ridx_seed_many_targets_stale", true);
    let s = seed(&base_out, &prev, &cur, &graph);
    assert!(s.invalidated_pkg_ids.contains("pkg"), "{s:?}");
    // Before the fix the variant's group was checked again for each of its sidecars: up to TARGETS² checks.
    assert!(s.sidecar_checks <= TARGETS, "{} sidecar checks", s.sidecar_checks);
    assert!(s.source_reads <= 3 + TARGETS, "read {} files", s.source_reads);
}
