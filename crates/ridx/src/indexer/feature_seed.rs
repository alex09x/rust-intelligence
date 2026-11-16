/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Sidecar invariance checks and selective Cargo `.fingerprint` pruning for feature switches.

use super::feature_graph::{ResolvedFeatureGraph, ResolvedPackageNode};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// The subdirectory of a variant holding, during a run, the shards of the crates that run rebuilds.
pub(crate) const STALE_DIR: &str = ".stale";

pub(super) fn variant_node_feature_state(vm: Option<&ridx_format::VariantManifest>, node: &ResolvedPackageNode, effective_target: &str, host_triple: Option<&str>) -> Option<bool> {
    let vm = vm?;
    if vm.crate_features.is_empty() {
        return None;
    }
    let vm_eff_target = vm.target.as_deref().filter(|s| !s.is_empty() && *s != "host").or(host_triple);
    if vm_eff_target.is_some_and(|vt| vt != effective_target) {
        return Some(false);
    }
    let versioned_key = format!("{}@{}", node.name, node.version);
    let recorded = vm.crate_features.get(&versioned_key).or_else(|| vm.crate_features.get(&node.feature_key)).or_else(|| {
        let prefix = format!("{}@", node.name);
        (!vm.crate_features.keys().any(|k| k.starts_with(&prefix))).then(|| vm.crate_features.get(&node.name)).flatten()
    });
    let Some(recorded_feats) = recorded else {
        return Some(false);
    };
    if vm_eff_target == Some(effective_target) {
        Some(recorded_feats == &node.enabled_features)
    } else if recorded_feats != &node.enabled_features {
        Some(false)
    } else {
        None
    }
}

pub(super) fn sidecar_matches_node(val: &serde_json::Value, node: &ResolvedPackageNode, effective_target: &str, variant_feats_matched: bool) -> bool {
    if val.get("version").and_then(|v| v.as_str()).unwrap_or("") != node.version {
        return false;
    }
    if !variant_feats_matched {
        let Some(tgt) = val.get("target").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) else {
            return false;
        };
        if tgt != effective_target {
            return false;
        }
        let Some(feat_arr) = val.get("enabled_features").and_then(|v| v.as_array()) else {
            return false;
        };
        let mut recorded_feats: Vec<String> = feat_arr.iter().filter_map(|x| x.as_str().map(str::to_string)).collect();
        recorded_feats.sort();
        recorded_feats.dedup();
        if recorded_feats != node.enabled_features {
            return false;
        }
    }

    let Some(obj_str) = val.get("object_path").and_then(|v| v.as_str()) else {
        return false;
    };
    let obj_path = Path::new(obj_str);
    if !obj_path.is_file() {
        return false;
    }

    let is_dep = val.get("is_dependency").and_then(|v| v.as_bool()).unwrap_or(false);
    if is_dep {
        return true;
    }

    let Ok(shard_mtime) = fs::metadata(obj_path).and_then(|m| m.modified()) else {
        return false;
    };
    let build_rs = node.manifest_dir.join("build.rs");
    if build_rs.is_file() && fs::metadata(&build_rs).and_then(|m| m.modified()).is_ok_and(|t| t > shard_mtime) {
        return false;
    }

    let Some(fh_obj) = val.get("file_hashes").and_then(|v| v.as_object()) else {
        return false;
    };
    if fh_obj.is_empty() {
        return false;
    }

    let mut tracked_canon = HashSet::new();
    for (path_key, hash_val) in fh_obj {
        let Some(expected) = hash_val.as_u64() else {
            return false;
        };
        let p = Path::new(path_key);
        let resolved = if p.is_absolute() && p.is_file() {
            p.to_path_buf()
        } else {
            let joined = ridx_format::join_resolving_overlap(&node.manifest_dir, p);
            if joined.is_file() {
                joined
            } else if let Some(ws_cand) = node.manifest_dir.ancestors().skip(1).map(|a| a.join(p)).find(|c| c.is_file()) {
                ws_cand
            } else {
                return false;
            }
        };
        if let Ok(canon) = resolved.canonicalize() {
            tracked_canon.insert(canon);
        }
        let Ok(bytes) = fs::read(&resolved) else {
            return false;
        };
        let norm_h = ridx_format::normalized_source_hash(&bytes);
        let raw_h = ridx_format::fnv1a_64(&bytes);
        if norm_h != expected && raw_h != expected {
            return false;
        }
    }

    !crate::indexer::direct::config::has_untracked_rs_files(&node.manifest_dir, &tracked_canon.iter().map(|p| p.display().to_string()).collect(), shard_mtime)
}

pub(super) fn prune_invalidated_artifacts(
    graph: &ResolvedFeatureGraph,
    invalidated_pkg_ids: &HashSet<String>,
    variant_out_dir: &Path,
    cargo_target_dir: &Path,
    profile_name: &str,
) {
    if invalidated_pkg_ids.is_empty() {
        return;
    }

    let mut inv_roots: HashSet<PathBuf> = HashSet::new();
    let mut inv_prefixes: Vec<String> = Vec::new();
    for pkg_id in invalidated_pkg_ids {
        if let Some(node) = graph.nodes.get(pkg_id) {
            inv_roots.insert(node.manifest_dir.clone());
            inv_prefixes.push(format!("{}-", node.name));
            if node.crate_name != node.name {
                inv_prefixes.push(format!("{}-", node.crate_name));
            }
        }
    }

    if variant_out_dir.join("variant_manifest.json").is_file() {
        // The invalidated shards and sidecars move aside to `STALE_DIR`, not away: a crate that then fails to build
        // takes them back (`keep_last_good_shards`) instead of dropping out of the index.
        let stale = variant_out_dir.join(STALE_DIR);
        let _ = fs::create_dir_all(stale.join(".deps"));
        let ddir = variant_out_dir.join(".deps");
        if let Ok(entries) = fs::read_dir(&ddir) {
            for entry in entries.flatten() {
                let sp = entry.path();
                if sp.extension().and_then(|s| s.to_str()) != Some("json") {
                    continue;
                }
                let Ok(bytes) = fs::read(&sp) else { continue };
                let Ok(val) = serde_json::from_slice::<serde_json::Value>(&bytes) else { continue };
                let Some(sr) = val.get("source_root").and_then(|v| v.as_str()) else { continue };
                let canon_sr = Path::new(sr).canonicalize().unwrap_or_else(|_| PathBuf::from(sr));
                if inv_roots.contains(&canon_sr) {
                    if val.get("is_dependency").and_then(|v| v.as_bool()) == Some(false) {
                        if let Some(obj) = val.get("object_path").and_then(|v| v.as_str()).map(Path::new) {
                            if obj.file_name().is_none_or(|f| fs::rename(obj, stale.join(f)).is_err()) {
                                let _ = fs::remove_file(obj);
                            }
                        }
                    }
                    if fs::rename(&sp, stale.join(".deps").join(entry.file_name())).is_err() {
                        let _ = fs::remove_file(&sp);
                    }
                }
            }
        }
    }

    let cargo_prof = if profile_name == "release" { "release" } else { "debug" };
    let fp_dirs = [cargo_target_dir.join(cargo_prof).join(".fingerprint"), cargo_target_dir.join(&graph.effective_target).join(cargo_prof).join(".fingerprint")];
    for fp_dir in &fp_dirs {
        let Ok(entries) = fs::read_dir(fp_dir) else { continue };
        for entry in entries.flatten() {
            let fname = entry.file_name();
            let s = fname.to_string_lossy();
            if inv_prefixes.iter().any(|pref| s.starts_with(pref)) {
                let p = entry.path();
                if p.is_dir() {
                    let _ = fs::remove_dir_all(&p);
                } else {
                    let _ = fs::remove_file(&p);
                }
            }
        }
    }
}

pub(super) fn propagate_invalidation(graph: &ResolvedFeatureGraph, dirty: &mut HashSet<String>, feat_dirty: &mut HashSet<String>) {
    // 1. Forward propagation for packages whose subfeatures could be toggled across a resolver v2 host/target split
    loop {
        let mut progressed = false;
        let current: Vec<String> = feat_dirty.iter().cloned().collect();
        for pkg_id in current {
            let Some(node) = graph.nodes.get(&pkg_id) else { continue };
            for dep_id in &node.feature_activated_deps {
                if graph.active_ids.contains(dep_id) && feat_dirty.insert(dep_id.clone()) {
                    dirty.insert(dep_id.clone());
                    progressed = true;
                }
            }
        }
        if !progressed {
            break;
        }
    }

    // 2. Backward propagation through reverse dependencies to a fixed point
    loop {
        let mut progressed = false;
        for pkg_id in &graph.active_ids {
            if dirty.contains(pkg_id) {
                continue;
            }
            let Some(node) = graph.nodes.get(pkg_id) else { continue };
            let deps = if node.is_workspace_member { &node.all_deps } else { &node.normal_or_build_deps };
            if deps.iter().any(|d| dirty.contains(d)) {
                dirty.insert(pkg_id.clone());
                progressed = true;
            }
        }
        if !progressed {
            break;
        }
    }
}

pub(super) fn copy_seeded_group(group: &[(PathBuf, PathBuf, serde_json::Value)], variant_out_dir: &Path, dst_deps: &Path, cargo_lock_hash: Option<&str>) -> bool {
    let mut copied_any = false;
    for (src_vdir, src_sidecar, mut val) in group.iter().cloned() {
        let is_dep = val.get("is_dependency").and_then(|v| v.as_bool()).unwrap_or(false);
        let Some(obj_str) = val.get("object_path").and_then(|v| v.as_str()).map(str::to_string) else { continue };
        let src_obj = PathBuf::from(&obj_str);
        if !src_obj.is_file() {
            return false;
        }
        if !is_dep {
            let Some(fname) = src_obj.file_name() else { continue };
            let dst_obj = variant_out_dir.join(fname);
            if src_vdir != variant_out_dir && fs::copy(&src_obj, &dst_obj).is_err() {
                return false;
            }
            val["object_path"] = serde_json::json!(dst_obj.display().to_string());
            if let Some(lh) = cargo_lock_hash {
                val["cargo_lock_hash"] = serde_json::json!(lh);
            }
        }
        if let Some(sfname) = src_sidecar.file_name() {
            let dst_sidecar = dst_deps.join(sfname);
            if let Ok(bytes) = serde_json::to_vec_pretty(&val) {
                if fs::write(&dst_sidecar, bytes).is_ok() {
                    copied_any = true;
                }
            }
        }
    }
    copied_any
}

/// After a check that failed for some crates: the shards it wrote replace the earlier shards of the same crates,
/// and a crate that did not build keeps its last good shards.
pub(super) fn keep_last_good_shards(dir: &Path, started: std::time::SystemTime, previous: Option<&Path>) {
    let shards: Vec<(PathBuf, String, bool)> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "ridx"))
        .filter_map(|p| {
            let stem = p.file_stem()?.to_str()?.to_string();
            let krate = stem.rsplit_once('-').map_or(stem.clone(), |(c, _)| c.to_string());
            let fresh = fs::metadata(&p).and_then(|m| m.modified()).is_ok_and(|t| t >= started);
            Some((p, krate, fresh))
        })
        .collect();
    let rebuilt: std::collections::HashSet<&str> = shards.iter().filter(|s| s.2).map(|s| s.1.as_str()).collect();
    for (path, krate, fresh) in &shards {
        if !fresh && rebuilt.contains(krate.as_str()) {
            let _ = fs::remove_file(path);
        }
    }
    // Crates with no shard here take theirs, with their sidecars, from the variant active before.
    let Some(prev) = previous else { return };
    let present: std::collections::HashSet<String> = shards.iter().map(|s| s.1.clone()).collect();
    let mut adopted = std::collections::HashSet::new();
    for entry in fs::read_dir(prev).into_iter().flatten().flatten() {
        let p = entry.path();
        let Some(name) = p.file_name().and_then(|n| n.to_str()).map(str::to_string) else { continue };
        let Some(stem) = name.strip_suffix(".ridx") else { continue };
        let krate = stem.rsplit_once('-').map_or(stem, |(c, _)| c);
        if !present.contains(krate) && fs::copy(&p, dir.join(&name)).is_ok() {
            adopted.insert(name);
        }
    }
    let (from, to) = (prev.join(".deps"), dir.join(".deps"));
    for entry in fs::read_dir(&from).into_iter().flatten().flatten() {
        let Some(mut j) = fs::read(entry.path()).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()) else { continue };
        let shard = j["object_path"].as_str().and_then(|o| Path::new(o).file_name()).and_then(|f| f.to_str()).map(str::to_string);
        if !shard.as_ref().is_some_and(|s| adopted.contains(s)) || to.join(entry.file_name()).exists() {
            continue;
        }
        j["object_path"] = serde_json::json!(dir.join(shard.unwrap_or_default()).display().to_string());
        if let Ok(b) = serde_json::to_vec(&j) {
            let _ = fs::write(to.join(entry.file_name()), b);
        }
    }
}
