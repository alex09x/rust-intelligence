/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Seeding a new variant when the resolved graph holds packages `cargo check` never compiled, and pruning Cargo's
//! fingerprints of the packages checked again (prod-code #2101: on every new variant, as when an agent edits
//! `Cargo.toml`, 47 of tokio's 160 packages were checked again and the reindex took 45-60 s on a large workspace).

use super::*;
use crate::indexer::feature_graph::{seed_unaffected_and_prune_invalidated, ResolvedFeatureGraph, ResolvedPackageNode};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const TGT: &str = "x86_64-unknown-linux-gnu";

fn node(name: &str, version: &str, dir: &Path, deps: &[&str]) -> ResolvedPackageNode {
    ResolvedPackageNode {
        id: name.into(),
        name: name.into(),
        crate_name: name.replace('-', "_"),
        version: version.into(),
        feature_key: name.into(),
        manifest_dir: dir.to_path_buf(),
        is_workspace_member: false,
        enabled_features: vec![],
        normal_or_build_deps: deps.iter().map(|d| d.to_string()).collect(),
        all_deps: deps.iter().map(|d| d.to_string()).collect(),
        feature_activated_deps: vec![],
    }
}

struct Ws {
    _dir: TestDir,
    base_out: PathBuf,
    prev: PathBuf,
    cur: PathBuf,
    cargo_tgt: PathBuf,
}

/// An earlier variant `v_prev` holding a dependency sidecar for each `(name, version, sidecar_version)` given.
fn ws(name: &str, indexed: &[(&str, &str)]) -> Ws {
    let dir = new_test_dir(name);
    let root = dir.0.canonicalize().unwrap();
    let base_out = root.join("out");
    let prev = base_out.join("variants/v_prev");
    let cur = base_out.join("variants/v_cur");
    let cargo_tgt = root.join("cargo_tgt");
    for d in [prev.join(".deps"), cur.clone(), cargo_tgt.join("debug/.fingerprint")] {
        fs::create_dir_all(d).unwrap();
    }
    let vm = serde_json::json!({
        "version": "2.0", "format_version": ridx_format::CURRENT_FORMAT_VERSION, "generation": 1,
        "active_variant_id": "v_prev", "variant_dir": "variants/v_prev", "profile": "dev",
        "target": serde_json::Value::Null, "crate_features": {}
    });
    fs::write(prev.join("variant_manifest.json"), serde_json::to_vec(&vm).unwrap()).unwrap();
    for (pkg, version) in indexed {
        let src = root.join("registry").join(format!("{pkg}-{version}"));
        fs::create_dir_all(&src).unwrap();
        let shard = root.join(format!("{pkg}.ridx"));
        fs::write(&shard, b"ridx-stub").unwrap();
        let sc = serde_json::json!({
            "name": pkg, "version": version, "source_root": src.display().to_string(), "object_path": shard.display().to_string(),
            "is_dependency": true, "target": TGT, "enabled_features": [],
        });
        fs::write(prev.join(format!(".deps/{pkg}.json")), serde_json::to_vec(&sc).unwrap()).unwrap();
    }
    Ws { _dir: dir, base_out, prev, cur, cargo_tgt }
}

fn graph(nodes: Vec<ResolvedPackageNode>) -> ResolvedFeatureGraph {
    ResolvedFeatureGraph {
        effective_target: TGT.into(),
        host_triple: Some(TGT.into()),
        active_ids: nodes.iter().map(|n| n.id.clone()).collect::<BTreeSet<_>>(),
        crate_features: BTreeMap::new(),
        nodes: nodes.into_iter().map(|n| (n.id.clone(), n)).collect(),
    }
}

fn src(w: &Ws, pkg: &str, version: &str) -> PathBuf {
    w.base_out.parent().unwrap().join("registry").join(format!("{pkg}-{version}"))
}

#[test]
fn test_a_dependency_never_compiled_leaves_the_packages_depending_on_it_seeded() {
    let _lock = lock_indexer_test();
    let w = ws("ridx_seed_unindexed", &[("uses-it", "1.0.0")]);
    // `never-built` is in the resolved graph (another platform's dependency, say) but has no sidecar anywhere.
    let never = w.base_out.parent().unwrap().join("registry/never-built-0.2.0");
    fs::create_dir_all(&never).unwrap();
    let g = graph(vec![node("uses-it", "1.0.0", &src(&w, "uses-it", "1.0.0"), &["never-built"]), node("never-built", "0.2.0", &never, &[])]);
    // Had Cargo built it after all (its sidecar lost with a removed variant), its own fingerprint goes so the driver runs.
    let fp = w.cargo_tgt.join("debug/.fingerprint");
    for d in ["never-built-0123456789abcdef", "uses-it-fedcba9876543210"] {
        fs::create_dir_all(fp.join(d)).unwrap();
    }
    let s = seed_unaffected_and_prune_invalidated(&g, &w.base_out, &w.cur, Some(&w.prev), &w.cargo_tgt, "dev", None);
    assert!(s.seeded_pkg_ids.contains("uses-it"), "{s:?}");
    assert!(s.invalidated_pkg_ids.is_empty(), "{s:?}");
    assert_eq!(s.left_to_cargo, 1);
    assert!(!fp.join("never-built-0123456789abcdef").exists(), "its own fingerprint is pruned");
    assert!(fp.join("uses-it-fedcba9876543210").exists(), "the seeded package's is kept");
}

#[test]
fn test_a_dependency_never_compiled_keeps_the_fingerprint_of_another_version() {
    let _lock = lock_indexer_test();
    let w = ws("ridx_seed_unindexed_dup", &[("hashbrown", "0.15.0")]);
    let never = w.base_out.parent().unwrap().join("registry/hashbrown-0.17.0");
    fs::create_dir_all(&never).unwrap();
    let mut newer = node("hashbrown", "0.17.0", &never, &[]);
    newer.id = "hashbrown@0.17.0".into();
    let g = graph(vec![node("hashbrown", "0.15.0", &src(&w, "hashbrown", "0.15.0"), &[]), newer]);
    let fp = w.cargo_tgt.join("debug/.fingerprint/hashbrown-0123456789abcdef");
    fs::create_dir_all(&fp).unwrap();
    let s = seed_unaffected_and_prune_invalidated(&g, &w.base_out, &w.cur, Some(&w.prev), &w.cargo_tgt, "dev", None);
    assert!(s.seeded_pkg_ids.contains("hashbrown"), "{s:?}");
    assert!(fp.exists(), "the compiled hashbrown keeps its fingerprint: Cargo's entries do not say which version they are");
}

#[test]
fn test_without_an_earlier_variant_a_dependency_without_a_sidecar_is_checked_again() {
    let _lock = lock_indexer_test();
    // A new output directory: no variant to read, while the shared Cargo target directory may hold the crate fresh.
    let w = ws("ridx_seed_no_history", &[]);
    fs::remove_dir_all(&w.prev).unwrap();
    let dir = w.base_out.parent().unwrap().join("registry/built-elsewhere-1.0.0");
    fs::create_dir_all(&dir).unwrap();
    let g = graph(vec![node("built-elsewhere", "1.0.0", &dir, &[])]);
    let fp = w.cargo_tgt.join("debug/.fingerprint/built-elsewhere-0123456789abcdef");
    fs::create_dir_all(&fp).unwrap();
    let s = seed_unaffected_and_prune_invalidated(&g, &w.base_out, &w.cur, None, &w.cargo_tgt, "dev", None);
    assert!(s.invalidated_pkg_ids.contains("built-elsewhere"), "{s:?}");
    assert_eq!(s.left_to_cargo, 0);
    assert!(!fp.exists(), "its fingerprint goes, so the driver runs on it");
}

#[test]
fn test_a_package_checked_again_prunes_only_its_own_fingerprints() {
    let _lock = lock_indexer_test();
    let w = ws("ridx_seed_prune_exact", &[("regex", "1.0.0"), ("regex-syntax", "0.8.0")]);
    // `regex` resolves to another version than the one indexed, so it is checked again; `regex-syntax` is unchanged.
    let g = graph(vec![node("regex", "1.1.0", &src(&w, "regex", "1.0.0"), &[]), node("regex-syntax", "0.8.0", &src(&w, "regex-syntax", "0.8.0"), &[])]);
    let fp = w.cargo_tgt.join("debug/.fingerprint");
    for d in ["regex-0123456789abcdef", "regex-syntax-fedcba9876543210"] {
        fs::create_dir_all(fp.join(d)).unwrap();
    }
    let s = seed_unaffected_and_prune_invalidated(&g, &w.base_out, &w.cur, Some(&w.prev), &w.cargo_tgt, "dev", None);
    assert!(s.invalidated_pkg_ids.contains("regex"), "{s:?}");
    assert!(s.seeded_pkg_ids.contains("regex-syntax"), "{s:?}");
    assert!(!fp.join("regex-0123456789abcdef").exists(), "the checked package's fingerprint is pruned");
    assert!(fp.join("regex-syntax-fedcba9876543210").exists(), "another package whose name starts the same keeps its fingerprint");
}

#[test]
fn test_fingerprint_entries_match_the_package_name_exactly() {
    use crate::indexer::feature_seed::is_fingerprint_of;
    assert!(is_fingerprint_of("tokio-0123456789abcdef", "tokio-"));
    assert!(!is_fingerprint_of("tokio-macros-0123456789abcdef", "tokio-"));
    assert!(!is_fingerprint_of("futures-core-0123456789abcdef", "futures-"));
    assert!(!is_fingerprint_of("tokio-0123", "tokio-"));
}
