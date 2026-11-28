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
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// The subdirectory of a variant holding, during a run, the shards of the crates that run rebuilds.
pub(crate) const STALE_DIR: &str = ".stale";

/// Hashes (normalized, raw) of the source files read while seeding, so that a file tracked by many sidecars is read
/// once a run: every example of a package like Bevy's root has its own sidecar, 1,391 of them tracking 741,678 files.
#[derive(Default)]
pub(super) struct SourceHashes {
    known: HashMap<PathBuf, Option<(u64, u64)>>,
    /// Files read from disk.
    pub(super) reads: usize,
}

impl SourceHashes {
    fn of(&mut self, path: &Path) -> Option<(u64, u64)> {
        if let Some(h) = self.known.get(path) {
            return *h;
        }
        self.reads += 1;
        let h = fs::read(path).ok().map(|b| (ridx_format::normalized_source_hash(&b), ridx_format::fnv1a_64(&b)));
        self.known.insert(path.to_path_buf(), h);
        h
    }
}

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

pub(super) fn sidecar_matches_node(val: &serde_json::Value, node: &ResolvedPackageNode, effective_target: &str, variant_feats_matched: bool, hashes: &mut SourceHashes) -> bool {
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
        let Some((norm_h, raw_h)) = hashes.of(&resolved) else {
            return false;
        };
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
        // The invalidated sidecars move aside to `STALE_DIR`, not away: a crate that then fails to build takes them
        // back (`settle_after_check`) instead of dropping out of the index. Their shards stay in place until a rebuilt
        // unit supersedes them, so the manifest a running server reads keeps naming files that exist.
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
                if inv_roots.contains(&canon_sr) && fs::rename(&sp, stale.join(".deps").join(entry.file_name())).is_err() {
                    let _ = fs::remove_file(&sp);
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
