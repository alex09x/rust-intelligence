/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Regression tests for `1.3` feature/cfg awareness and DAG-scoped partial crate invalidation.

use super::*;
use crate::indexer::driver::find_driver_binary;
use crate::indexer::orchestrate::{keep_last_good_shards, run_indexing};
use crate::indexer::variant::compute_variant_id;
use crate::indexer::{IndexBuilder, IndexOptions};
use ridx_format::{IndexReader, VariantManifest};
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

#[test]
fn test_failed_crates_keep_last_good_shards() {
    let root = std::env::temp_dir().join(format!("ridx_last_good_{}", std::process::id()));
    let (prev, cur) = (root.join("prev"), root.join("cur"));
    for d in [&prev.join(".deps"), &cur.join(".deps")] {
        fs::create_dir_all(d).unwrap();
    }
    fs::write(prev.join("test_suite-aa.ridx"), b"old").unwrap();
    fs::write(prev.join("serde-aa.ridx"), b"old").unwrap();
    fs::write(prev.join(".deps/test_suite-aa.json"), serde_json::json!({ "object_path": prev.join("test_suite-aa.ridx").display().to_string() }).to_string()).unwrap();
    fs::write(cur.join("serde-old.ridx"), b"stale").unwrap();
    let started = SystemTime::now();
    std::thread::sleep(Duration::from_millis(20));
    fs::write(cur.join("serde-new.ridx"), b"fresh").unwrap();
    keep_last_good_shards(&cur, started, Some(&prev));
    let mut names: Vec<String> = fs::read_dir(&cur).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".ridx")).collect();
    names.sort();
    assert_eq!(names, ["serde-new.ridx", "test_suite-aa.ridx"]);
    let side: serde_json::Value = serde_json::from_slice(&fs::read(cur.join(".deps/test_suite-aa.json")).unwrap()).unwrap();
    assert_eq!(side["object_path"], cur.join("test_suite-aa.ridx").display().to_string());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_feature_order_normalization_in_variant_id() {
    let _lock = lock_indexer_test();
    let test_dir = new_test_dir("ridx_feat_norm");
    let cargo_toml = test_dir.0.join("Cargo.toml");
    fs::write(&cargo_toml, "[package]\nname = \"norm_pkg\"\nversion = \"0.1.0\"\n").unwrap();

    let v_ab = compute_variant_id("dev", None, None, Some("alpha,beta"), false, false, &cargo_toml);
    let v_ba = compute_variant_id("dev", None, None, Some("beta alpha"), false, false, &cargo_toml);
    let v_dup = compute_variant_id("dev", None, None, Some("beta, alpha, beta"), false, false, &cargo_toml);
    assert_eq!(v_ab, v_ba);
    assert_eq!(v_ab, v_dup);
}

fn find_lib_shard_for_crate(variant_dir: &Path, crate_name: &str) -> std::path::PathBuf {
    let deps_dir = variant_dir.join(".deps");
    for entry in fs::read_dir(&deps_dir).unwrap().flatten() {
        let p = entry.path();
        if p.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let Ok(val) = serde_json::from_slice::<serde_json::Value>(&fs::read(&p).unwrap()) else { continue };
        if val.get("name").and_then(|v| v.as_str()) == Some(crate_name) && val.get("target_kind").and_then(|v| v.as_str()) == Some("lib") {
            if let Some(obj) = val.get("object_path").and_then(|v| v.as_str()) {
                return std::path::PathBuf::from(obj);
            }
        }
    }
    panic!("lib shard not found for {crate_name} in {variant_dir:?}");
}

fn max_fingerprint_mtime(cargo_ridx_dir: &Path, prefix: &str) -> SystemTime {
    let mut latest: Option<SystemTime> = None;
    let mut stack = vec![cargo_ridx_dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for entry in rd.flatten() {
            let p = entry.path();
            if d.file_name().and_then(|n| n.to_str()) == Some(".fingerprint") {
                if entry.file_name().to_string_lossy().starts_with(prefix) {
                    let mut fp_stack = vec![p];
                    while let Some(fp) = fp_stack.pop() {
                        if let Ok(m) = fs::metadata(&fp).and_then(|x| x.modified()) {
                            latest = Some(latest.map_or(m, |cur| cur.max(m)));
                        }
                        if fp.is_dir() {
                            if let Ok(sub) = fs::read_dir(&fp) {
                                for se in sub.flatten() {
                                    fp_stack.push(se.path());
                                }
                            }
                        }
                    }
                }
            } else if p.is_dir() {
                stack.push(p);
            }
        }
    }
    latest.expect("expected Cargo .fingerprint entry")
}

#[test]
fn test_feature_switch_invalidates_only_affected_crates_and_records_cfgs() {
    let _lock = lock_indexer_test();
    let Some(driver) = find_driver_binary() else {
        return;
    };

    let test_dir = new_test_dir("ridx_feat_partial_inv");
    let ws = test_dir.0.join("ws");
    let cache_dir = test_dir.0.join("cas");
    fs::create_dir_all(&ws).unwrap();
    fs::create_dir_all(&cache_dir).unwrap();

    fs::write(ws.join("Cargo.toml"), "[workspace]\nmembers = [\"ws_unaffected\", \"ws_feat\", \"ws_downstream\"]\nresolver = \"2\"\n").unwrap();

    // 1. ws_unaffected: completely independent of ws_feat
    fs::create_dir_all(ws.join("ws_unaffected/src")).unwrap();
    fs::write(ws.join("ws_unaffected/Cargo.toml"), "[package]\nname = \"ws_unaffected\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
    fs::write(ws.join("ws_unaffected/src/lib.rs"), "pub fn stable_helper() -> usize { 7 }\n").unwrap();

    // 2. ws_feat: has optional feature `extra`
    fs::create_dir_all(ws.join("ws_feat/src")).unwrap();
    fs::write(ws.join("ws_feat/Cargo.toml"), "[package]\nname = \"ws_feat\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[features]\ndefault = []\nextra = []\n").unwrap();
    fs::write(ws.join("ws_feat/src/lib.rs"), "pub fn base_api() -> u32 { 10 }\n#[cfg(feature = \"extra\")]\npub fn gated_extra_api() -> u32 { 99 }\n").unwrap();

    // 3. ws_downstream: depends on ws_feat (transitively affected when ws_feat features change)
    fs::create_dir_all(ws.join("ws_downstream/src")).unwrap();
    fs::write(
        ws.join("ws_downstream/Cargo.toml"),
        "[package]\nname = \"ws_downstream\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nws_feat = { path = \"../ws_feat\" }\n",
    )
    .unwrap();
    fs::write(ws.join("ws_downstream/src/lib.rs"), "pub fn call_base() -> u32 { ws_feat::base_api() }\n").unwrap();

    let mk_opts = |features: Option<&str>| IndexOptions {
        path: ws.clone(),
        out_dir_opt: None,
        package: None,
        release: false,
        features: features.map(str::to_string),
        no_default_features: false,
        all_features: false,
        target: None,
        cache_dir: Some(cache_dir.clone()),
        driver_bin: Some(driver.clone()),
        capture_child_output: true,
        builder: Some(IndexBuilder::Cargo),
    };

    // Run 1: default features (`extra` disabled)
    let pv1 = run_indexing(mk_opts(None));
    let mf1: VariantManifest = serde_json::from_slice(&fs::read(pv1.base_out_dir.join("ridx_manifest.json")).unwrap()).unwrap();
    assert!(mf1.cargo_lock_hash.as_deref().is_some_and(|h| h.len() == 16));
    assert_eq!(mf1.crate_features.get("ws_feat").cloned().unwrap_or_default(), vec!["default".to_string()]);

    let feat_r1 = IndexReader::open(find_lib_shard_for_crate(&pv1.variant_out_dir, "ws_feat")).unwrap();
    assert_eq!(feat_r1.enabled_features().collect::<Vec<_>>(), vec!["default"]);
    let r1_names: Vec<&str> = feat_r1.definitions().iter().map(|d| feat_r1.get_str(d.name_offset())).collect();
    assert!(r1_names.contains(&"base_api") && !r1_names.contains(&"gated_extra_api"));
    drop(feat_r1);

    let unaff_r1 = IndexReader::open(find_lib_shard_for_crate(&pv1.variant_out_dir, "ws_unaffected")).unwrap();
    let unaff_uk1 = unaff_r1.unit_key().to_string();
    drop(unaff_r1);
    let down_r1 = IndexReader::open(find_lib_shard_for_crate(&pv1.variant_out_dir, "ws_downstream")).unwrap();
    let down_uk1 = down_r1.unit_key().to_string();
    drop(down_r1);

    let cargo_ridx = ws.join("target/ridx_cargo");
    let unaff_fp_t1 = max_fingerprint_mtime(&cargo_ridx, "ws_unaffected-");

    std::thread::sleep(Duration::from_millis(60));
    let before_run2 = SystemTime::now();
    std::thread::sleep(Duration::from_millis(20));

    // Run 2: enable `ws_feat/extra`
    let pv2 = run_indexing(mk_opts(Some("ws_feat/extra")));
    assert_ne!(pv1.variant_id, pv2.variant_id);

    let mf2: VariantManifest = serde_json::from_slice(&fs::read(pv2.base_out_dir.join("ridx_manifest.json")).unwrap()).unwrap();
    assert_eq!(mf2.crate_features.get("ws_feat").cloned().unwrap_or_default(), vec!["default".to_string(), "extra".to_string()]);

    // Affected crate (`ws_feat`) re-indexed with `extra` enabled
    let feat_r2 = IndexReader::open(find_lib_shard_for_crate(&pv2.variant_out_dir, "ws_feat")).unwrap();
    let feats2: Vec<&str> = feat_r2.enabled_features().collect();
    assert_eq!(feats2, vec!["default", "extra"]);
    assert!(feat_r2.has_cfg_key_value("feature", "extra"));
    let r2_names: Vec<&str> = feat_r2.definitions().iter().map(|d| feat_r2.get_str(d.name_offset())).collect();
    assert!(r2_names.contains(&"base_api") && r2_names.contains(&"gated_extra_api"));
    drop(feat_r2);

    // Downstream dependent (`ws_downstream`) transitively re-indexed with updated upstream .rmeta digest
    let down_shard2 = find_lib_shard_for_crate(&pv2.variant_out_dir, "ws_downstream");
    assert!(fs::metadata(&down_shard2).unwrap().modified().unwrap() >= before_run2);
    let down_r2 = IndexReader::open(&down_shard2).unwrap();
    assert_ne!(down_r2.unit_key(), down_uk1);
    drop(down_r2);

    // Unaffected crate (`ws_unaffected`) was NOT recompiled by Cargo and preserved exact unit_key
    let unaff_fp_t2 = max_fingerprint_mtime(&cargo_ridx, "ws_unaffected-");
    assert_eq!(unaff_fp_t1, unaff_fp_t2, "unaffected crate Cargo fingerprint must stay untouched across feature switch");
    let unaff_r2 = IndexReader::open(find_lib_shard_for_crate(&pv2.variant_out_dir, "ws_unaffected")).unwrap();
    assert_eq!(unaff_r2.unit_key(), unaff_uk1);
    drop(unaff_r2);
}

#[test]
fn test_dual_host_target_sidecars_seeded_unless_subfeature_toggled() {
    use crate::indexer::feature_graph::{seed_unaffected_and_prune_invalidated, ResolvedFeatureGraph, ResolvedPackageNode};
    use std::collections::{BTreeMap, BTreeSet};

    let _lock = lock_indexer_test();
    let test_dir = new_test_dir("ridx_dual_sidecars");
    let root = &test_dir.0;
    let dep_dir = root.join("dep_dual");
    let leaf_dir = root.join("ws_leaf");
    let cas_dir = root.join("cas");
    let base_out = root.join("out");
    let prev_vdir = base_out.join("variants/v_prev");
    let cur1_vdir = base_out.join("variants/v_cur1");
    let cur2_vdir = base_out.join("variants/v_cur2");
    let cargo_tgt = root.join("cargo_tgt");
    for d in [&dep_dir, &leaf_dir, &cas_dir, &prev_vdir.join(".deps"), &cur1_vdir, &cur2_vdir, &cargo_tgt] {
        fs::create_dir_all(d).unwrap();
    }
    let dep_canon = dep_dir.canonicalize().unwrap();
    let leaf_canon = leaf_dir.canonicalize().unwrap();
    let leaf_rs = leaf_canon.join("lib.rs");
    fs::write(&leaf_rs, "pub fn leaf() {}\n").unwrap();

    let shard_host = cas_dir.join("dep_dual-host.ridx");
    let shard_tgt = cas_dir.join("dep_dual-tgt.ridx");
    let shard_leaf = prev_vdir.join("ws_leaf-old.ridx");
    for f in [&shard_host, &shard_tgt, &shard_leaf] {
        fs::write(f, b"ridx-stub").unwrap();
    }

    let tgt = "x86_64-unknown-linux-gnu";
    let vm = serde_json::json!({
        "version": "2.0",
        "format_version": ridx_format::CURRENT_FORMAT_VERSION,
        "generation": 1,
        "active_variant_id": "v_prev",
        "variant_dir": "variants/v_prev",
        "profile": "dev",
        "target": serde_json::Value::Null,
        "crate_features": {
            "dep_dual": ["pm_feat"],
            "ws_leaf": []
        }
    });
    fs::write(prev_vdir.join("variant_manifest.json"), serde_json::to_vec(&vm).unwrap()).unwrap();
    for (idx, (obj, feats)) in [(&shard_host, serde_json::json!(["pm_feat"])), (&shard_tgt, serde_json::json!([]))].into_iter().enumerate() {
        let sc = serde_json::json!({
            "name": "dep_dual", "version": "0.1.0", "source_root": dep_canon.display().to_string(),
            "object_path": obj.display().to_string(), "is_dependency": true, "target": tgt, "enabled_features": feats,
        });
        fs::write(prev_vdir.join(format!(".deps/dep_dual-{idx}.json")), serde_json::to_vec(&sc).unwrap()).unwrap();
    }

    let mk_graph = |activated: Vec<String>| {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            "dep_dual".into(),
            ResolvedPackageNode {
                id: "dep_dual".into(),
                name: "dep_dual".into(),
                crate_name: "dep_dual".into(),
                version: "0.1.0".into(),
                feature_key: "dep_dual".into(),
                manifest_dir: dep_canon.clone(),
                is_workspace_member: false,
                enabled_features: vec!["pm_feat".into()],
                normal_or_build_deps: vec![],
                all_deps: vec![],
                feature_activated_deps: vec![],
            },
        );
        nodes.insert(
            "ws_leaf".into(),
            ResolvedPackageNode {
                id: "ws_leaf".into(),
                name: "ws_leaf".into(),
                crate_name: "ws_leaf".into(),
                version: "0.1.0".into(),
                feature_key: "ws_leaf".into(),
                manifest_dir: leaf_canon.clone(),
                is_workspace_member: true,
                enabled_features: vec!["leaf_flag".into()],
                normal_or_build_deps: vec!["dep_dual".into()],
                all_deps: vec!["dep_dual".into()],
                feature_activated_deps: activated,
            },
        );
        ResolvedFeatureGraph {
            effective_target: tgt.into(),
            host_triple: Some(tgt.into()),
            nodes,
            active_ids: BTreeSet::from(["dep_dual".to_string(), "ws_leaf".to_string()]),
            crate_features: BTreeMap::from([("dep_dual".into(), vec!["pm_feat".into()]), ("ws_leaf".into(), vec!["leaf_flag".into()])]),
        }
    };

    // Case 1: unrelated leaf feature -> dual-compiled `dep_dual` stays seeded with both sidecars intact
    let s1 = seed_unaffected_and_prune_invalidated(&mk_graph(vec![]), &base_out, &cur1_vdir, Some(&prev_vdir), &cargo_tgt, "dev", None);
    assert!(s1.seeded_pkg_ids.contains("dep_dual"));
    assert!(s1.invalidated_pkg_ids.contains("ws_leaf"));
    assert_eq!(fs::read_dir(cur1_vdir.join(".deps")).unwrap().count(), 2);

    // Case 2: `ws_leaf` activates a subfeature on `dep_dual` (`dep_dual/...`) -> `dep_dual` is invalidated
    let s2 = seed_unaffected_and_prune_invalidated(&mk_graph(vec!["dep_dual".into()]), &base_out, &cur2_vdir, Some(&prev_vdir), &cargo_tgt, "dev", None);
    assert!(s2.invalidated_pkg_ids.contains("dep_dual"));
    assert!(s2.invalidated_pkg_ids.contains("ws_leaf"));
}
