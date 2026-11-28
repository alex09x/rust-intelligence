/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Feature/cfg graph resolution (`cargo metadata`), `Cargo.lock` fingerprinting,
//! and DAG-exact partial invalidation across feature switches.

use super::feature_seed::{copy_seeded_group, propagate_invalidation, prune_invalidated_artifacts, sidecar_matches_node, variant_node_feature_state, SourceHashes};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Computes the 16-hex FNV-1a digest of `work_dir/Cargo.lock` if present.
pub fn compute_cargo_lock_hash(work_dir: &Path) -> Option<String> {
    let bytes = fs::read(work_dir.join("Cargo.lock")).ok()?;
    Some(format!("{:016x}", ridx_format::fnv1a_64(&bytes)))
}

/// Normalizes a raw CLI `--features` string into a sorted, deduplicated list of feature tokens.
pub fn normalize_feature_list(features: Option<&str>) -> Vec<String> {
    let Some(raw) = features else {
        return Vec::new();
    };
    let set: BTreeSet<String> = raw.split([',', ' ']).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect();
    set.into_iter().collect()
}

/// Queries the host target triple from `rustc -vV`.
pub fn detect_host_triple() -> Option<String> {
    let out = Command::new("rustc").arg("-vV").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines().find_map(|l| l.strip_prefix("host:").map(|r| r.trim().to_string()))
}

#[derive(Debug, Clone)]
pub struct ResolvedPackageNode {
    pub id: String,
    pub name: String,
    pub crate_name: String,
    pub version: String,
    pub feature_key: String,
    pub manifest_dir: PathBuf,
    pub is_workspace_member: bool,
    pub enabled_features: Vec<String>,
    pub normal_or_build_deps: Vec<String>,
    pub all_deps: Vec<String>,
    pub feature_activated_deps: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ResolvedFeatureGraph {
    pub effective_target: String,
    pub host_triple: Option<String>,
    pub nodes: BTreeMap<String, ResolvedPackageNode>,
    pub active_ids: BTreeSet<String>,
    pub crate_features: BTreeMap<String, Vec<String>>,
}

impl ResolvedFeatureGraph {
    /// Resolves unified dependency and per-crate feature graph using the default resolved `IndexBuilder`.
    pub fn resolve(manifest_path: &Path, target: Option<&str>, package: Option<&str>, features: Option<&str>, no_default_features: bool, all_features: bool) -> Option<Self> {
        Self::resolve_with_builder(super::builder::IndexBuilder::resolve(None), manifest_path, target, package, features, no_default_features, all_features)
    }

    /// Resolves unified dependency and per-crate feature graph via `mitts metadata` (or `cargo metadata`).
    pub fn resolve_with_builder(
        builder: super::builder::IndexBuilder,
        manifest_path: &Path,
        target: Option<&str>,
        package: Option<&str>,
        features: Option<&str>,
        no_default_features: bool,
        all_features: bool,
    ) -> Option<Self> {
        let host_triple = detect_host_triple();
        let effective_target = match target {
            Some(t) if !t.is_empty() && t != "host" => t.to_string(),
            _ => host_triple.clone()?,
        };

        let run_meta = |prog: &str| -> Option<std::process::Output> {
            let mut cmd = Command::new(prog);
            cmd.args(["metadata", "--format-version", "1", "--manifest-path"]);
            cmd.arg(manifest_path);
            cmd.args(["--filter-platform", &effective_target]);
            let norm_feats = normalize_feature_list(features);
            if !norm_feats.is_empty() {
                cmd.arg("--features").arg(norm_feats.join(","));
            }
            if no_default_features {
                cmd.arg("--no-default-features");
            }
            if all_features {
                cmd.arg("--all-features");
            }
            cmd.output().ok().filter(|o| o.status.success())
        };

        let out = run_meta(builder.as_str()).or_else(|| (builder == super::builder::IndexBuilder::Mitts).then(|| run_meta("cargo")).flatten())?;
        let meta: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
        let ws_members: HashSet<String> = meta.get("workspace_members")?.as_array()?.iter().filter_map(|v| v.as_str().map(str::to_string)).collect();

        let mut pkg_meta: HashMap<String, (String, String, PathBuf, HashSet<String>)> = HashMap::new();
        for pkg in meta.get("packages")?.as_array()? {
            let id = pkg.get("id")?.as_str()?.to_string();
            let name = pkg.get("name")?.as_str()?.to_string();
            let version = pkg.get("version")?.as_str().unwrap_or("0.0.0").to_string();
            let mp = PathBuf::from(pkg.get("manifest_path")?.as_str()?);
            let mdir = mp.parent().map(|p| p.canonicalize().unwrap_or_else(|_| p.to_path_buf())).unwrap_or_default();
            let mut alias_map: HashMap<&str, &str> = HashMap::new();
            if let Some(deps) = pkg.get("dependencies").and_then(|v| v.as_array()) {
                for dep in deps {
                    if let (Some(dn), Some(rn)) = (dep.get("name").and_then(|v| v.as_str()), dep.get("rename").and_then(|v| v.as_str())) {
                        alias_map.insert(rn, dn);
                    }
                }
            }
            let mut subfeat_pkgs = HashSet::new();
            if let Some(fmap) = pkg.get("features").and_then(|v| v.as_object()) {
                for item in fmap.values().filter_map(|v| v.as_array()).flat_map(|a| a.iter()).filter_map(|v| v.as_str()) {
                    if let Some((dep_ref, _)) = item.split_once('/') {
                        let clean = dep_ref.strip_suffix('?').unwrap_or(dep_ref);
                        subfeat_pkgs.insert(alias_map.get(clean).copied().unwrap_or(clean).to_string());
                    }
                }
            }
            pkg_meta.insert(id, (name, version, mdir, subfeat_pkgs));
        }

        let resolve_nodes = meta.get("resolve")?.get("nodes")?.as_array()?;
        let mut nodes = BTreeMap::new();
        for n in resolve_nodes {
            let id = n.get("id")?.as_str()?.to_string();
            let Some((name, version, manifest_dir, subfeat_pkgs)) = pkg_meta.get(&id).cloned() else {
                continue;
            };
            let mut feats: Vec<String> =
                n.get("features").and_then(|f| f.as_array()).map(|arr| arr.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
            feats.sort();
            feats.dedup();

            let mut all_deps = Vec::new();
            let mut normal_or_build_deps = Vec::new();
            let mut feature_activated_deps = Vec::new();
            if let Some(deps) = n.get("deps").and_then(|d| d.as_array()) {
                for d in deps {
                    let Some(dep_id) = d.get("pkg").and_then(|p| p.as_str()).map(str::to_string) else {
                        continue;
                    };
                    if !all_deps.contains(&dep_id) {
                        all_deps.push(dep_id.clone());
                    }
                    let is_non_dev = d
                        .get("dep_kinds")
                        .and_then(|dk| dk.as_array())
                        .map(|arr| arr.is_empty() || arr.iter().any(|k| k.get("kind").and_then(|v| v.as_str()) != Some("dev")))
                        .unwrap_or(true);
                    if is_non_dev && !normal_or_build_deps.contains(&dep_id) {
                        normal_or_build_deps.push(dep_id.clone());
                    }
                    if pkg_meta.get(&dep_id).is_some_and(|(dn, _, _, _)| subfeat_pkgs.contains(dn)) && !feature_activated_deps.contains(&dep_id) {
                        feature_activated_deps.push(dep_id);
                    }
                }
            }
            let crate_name = name.replace('-', "_");
            let is_workspace_member = ws_members.contains(&id);
            nodes.insert(
                id.clone(),
                ResolvedPackageNode {
                    id,
                    feature_key: name.clone(),
                    name,
                    crate_name,
                    version,
                    manifest_dir,
                    is_workspace_member,
                    enabled_features: feats,
                    normal_or_build_deps,
                    all_deps,
                    feature_activated_deps,
                },
            );
        }

        let mut active_ids = BTreeSet::new();
        let mut queue = VecDeque::new();
        for (id, node) in &nodes {
            let is_root =
                if let Some(pkg_filter) = package { node.is_workspace_member && (node.name == pkg_filter || node.crate_name == pkg_filter) } else { node.is_workspace_member };
            if is_root && active_ids.insert(id.clone()) {
                queue.push_back((id.clone(), true));
            }
        }
        while let Some((cur_id, include_dev)) = queue.pop_front() {
            let Some(node) = nodes.get(&cur_id) else {
                continue;
            };
            let edges = if include_dev { &node.all_deps } else { &node.normal_or_build_deps };
            for dep_id in edges {
                if nodes.contains_key(dep_id) && active_ids.insert(dep_id.clone()) {
                    queue.push_back((dep_id.clone(), false));
                }
            }
        }

        let mut name_counts: HashMap<String, usize> = HashMap::new();
        for id in &active_ids {
            if let Some(n) = nodes.get(id) {
                *name_counts.entry(n.name.clone()).or_insert(0) += 1;
            }
        }
        let mut crate_features = BTreeMap::new();
        for id in &active_ids {
            if let Some(n) = nodes.get_mut(id) {
                let key = if name_counts.get(&n.name).copied().unwrap_or(0) > 1 { format!("{}@{}", n.name, n.version) } else { n.name.clone() };
                n.feature_key = key.clone();
                crate_features.insert(key, n.enabled_features.clone());
            }
        }

        Some(Self { effective_target, host_triple, nodes, active_ids, crate_features })
    }
}

/// Computes a shared `CARGO_TARGET_DIR` name keyed by `(profile, target, rustflags, toolchain, format_version)`
/// so Cargo incremental fingerprints survive feature toggles while isolating incompatible toolchains/targets.
pub fn compute_shared_cargo_cache_id(profile: &str, effective_target: Option<&str>) -> String {
    let mut buf = format!("{}:{}:v{}:ext{}", profile, effective_target.unwrap_or("host"), ridx_format::CURRENT_FORMAT_VERSION, ridx_format::EXTRACTOR_VERSION,);
    if let Ok(rf) = std::env::var("RUSTFLAGS") {
        buf.push_str(":rf=");
        buf.push_str(&rf);
    }
    if let Ok(erf) = std::env::var("CARGO_ENCODED_RUSTFLAGS") {
        buf.push_str(":erf=");
        buf.push_str(&erf);
    }
    if let Ok(tc) = std::env::var("RUSTUP_TOOLCHAIN") {
        buf.push_str(":tc=");
        buf.push_str(&tc);
    }
    format!("{profile}-{:016x}", ridx_format::fnv1a_64(buf.as_bytes()))
}

#[derive(Debug, Default)]
pub struct SeedSummary {
    pub seeded_pkg_ids: HashSet<String>,
    pub invalidated_pkg_ids: HashSet<String>,
    /// Source files read from disk to check the candidate sidecars.
    pub source_reads: usize,
    /// Candidate sidecars checked against the resolved graph.
    pub sidecar_checks: usize,
    /// Why each invalidated package is checked again.
    pub reasons: BTreeMap<String, String>,
    /// Packages of the graph no earlier variant indexed, neither seeded nor invalidated: Cargo builds them if it needs to.
    pub left_to_cargo: usize,
}

/// Seeds unaffected workspace shards and `.deps` sidecars into `variant_out_dir` from existing
/// variants in `base_out_dir/variants/`, propagating invalidation along reverse dependency edges,
/// and purges Cargo `.fingerprint` entries for invalidated packages so `RUSTC_WRAPPER` runs on them.
pub fn seed_unaffected_and_prune_invalidated(
    graph: &ResolvedFeatureGraph,
    base_out_dir: &Path,
    variant_out_dir: &Path,
    previous_variant: Option<&Path>,
    cargo_target_dir: &Path,
    profile_name: &str,
    cargo_lock_hash: Option<&str>,
) -> SeedSummary {
    let mut cand_dirs: Vec<PathBuf> = Vec::new();
    if variant_out_dir.join(".deps").is_dir() && variant_out_dir.join("variant_manifest.json").is_file() {
        cand_dirs.push(variant_out_dir.to_path_buf());
    }
    if let Some(prev) = previous_variant {
        if prev.is_dir() && !cand_dirs.contains(&prev.to_path_buf()) {
            cand_dirs.push(prev.to_path_buf());
        }
    }
    if let Ok(rd) = fs::read_dir(base_out_dir.join("variants")) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() && p.join("variant_manifest.json").is_file() && !cand_dirs.contains(&p) {
                cand_dirs.push(p);
            }
        }
    }

    // Index candidate sidecars by canonical `source_root`
    #[allow(clippy::type_complexity)]
    let mut sidecars_by_root: HashMap<PathBuf, Vec<(PathBuf, PathBuf, serde_json::Value)>> = HashMap::new();
    let mut vm_by_vdir: HashMap<PathBuf, ridx_format::VariantManifest> = HashMap::new();
    for vdir in &cand_dirs {
        let Ok(vm_bytes) = fs::read(vdir.join("variant_manifest.json")) else { continue };
        let Ok(vm) = serde_json::from_slice::<ridx_format::VariantManifest>(&vm_bytes) else { continue };
        if vm.profile != profile_name || vm.format_version != ridx_format::CURRENT_FORMAT_VERSION {
            continue;
        }
        if vm.target.as_deref().is_some_and(|t| !t.is_empty() && t != "host" && t != graph.effective_target) {
            continue;
        }
        let ddir = vdir.join(".deps");
        let Ok(entries) = fs::read_dir(&ddir) else { continue };
        vm_by_vdir.insert(vdir.clone(), vm);
        for entry in entries.flatten() {
            let sp = entry.path();
            if sp.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let Ok(bytes) = fs::read(&sp) else { continue };
            let Ok(val) = serde_json::from_slice::<serde_json::Value>(&bytes) else { continue };
            let Some(sr) = val.get("source_root").and_then(|v| v.as_str()) else { continue };
            let canon_sr = Path::new(sr).canonicalize().unwrap_or_else(|_| PathBuf::from(sr));
            sidecars_by_root.entry(canon_sr).or_default().push((vdir.clone(), sp, val));
        }
    }

    let mut matched: HashMap<String, Vec<(PathBuf, PathBuf, serde_json::Value)>> = HashMap::new();
    let mut dirty: HashSet<String> = HashSet::new();
    let mut feat_dirty: HashSet<String> = HashSet::new();
    let mut hashes = SourceHashes::default();
    let mut sidecar_checks = 0;
    let mut reasons: BTreeMap<String, String> = BTreeMap::new();
    let mut unindexed: HashSet<String> = HashSet::new();

    for pkg_id in &graph.active_ids {
        let Some(node) = graph.nodes.get(pkg_id) else { continue };
        let Some(cands) = sidecars_by_root.get(&node.manifest_dir) else {
            if !node.is_workspace_member {
                // No earlier variant indexed this dependency: `cargo metadata` resolves packages `cargo check` never
                // compiles (another platform's, one only a target not checked uses), so it has no sidecar in any of them.
                // Invalidating it invalidated everything depending on it, and pruning by name the fingerprints of another
                // version of it, on every new variant (prod-code #2101: 47 of tokio's 160 packages each time Cargo.lock
                // changed). Cargo compiles it if it is new, and the crates depending on it, whose manifest changed too.
                unindexed.insert(pkg_id.clone());
                continue;
            }
            // A workspace member is always checked, so one without a sidecar is new: everything it reaches may change.
            dirty.insert(pkg_id.clone());
            feat_dirty.insert(pkg_id.clone());
            reasons.insert(pkg_id.clone(), "no earlier variant indexed this workspace member".into());
            continue;
        };
        let mut chosen_vdir: Option<PathBuf> = None;
        let mut any_feats_matched = false;
        // Each variant's group once: `cands` holds one entry per sidecar, and a variant whose group did not match was
        // checked again for every sidecar it has (Bevy's root package: 1,391 times, hours of reading).
        let mut tried: HashSet<&PathBuf> = HashSet::new();
        for (vdir, _, _) in cands {
            if !tried.insert(vdir) {
                continue;
            }
            let fstate = variant_node_feature_state(vm_by_vdir.get(vdir), node, &graph.effective_target, graph.host_triple.as_deref());
            if fstate == Some(false) {
                continue;
            }
            let feats_matched = fstate == Some(true);
            any_feats_matched |= feats_matched;
            let group: Vec<&(PathBuf, PathBuf, serde_json::Value)> = cands.iter().filter(|(vd, _, _)| vd == vdir).collect();
            if group.iter().all(|(_, _, val)| {
                sidecar_checks += 1;
                sidecar_matches_node(val, node, &graph.effective_target, feats_matched, &mut hashes)
            }) {
                chosen_vdir = Some(vdir.clone());
                break;
            }
        }
        if let Some(vdir) = chosen_vdir {
            let group: Vec<(PathBuf, PathBuf, serde_json::Value)> = cands.iter().filter(|(vd, _, _)| vd == &vdir).cloned().collect();
            matched.insert(pkg_id.clone(), group);
        } else {
            dirty.insert(pkg_id.clone());
            if !any_feats_matched {
                feat_dirty.insert(pkg_id.clone());
            }
            let why = if any_feats_matched { "its sources or build differ from every earlier index" } else { "its features differ from every earlier index" };
            reasons.insert(pkg_id.clone(), why.into());
        }
    }

    propagate_invalidation(graph, &mut dirty, &mut feat_dirty, &mut reasons);

    let dst_deps = variant_out_dir.join(".deps");
    let _ = fs::create_dir_all(&dst_deps);
    let mut summary = SeedSummary { source_reads: hashes.reads, sidecar_checks, ..SeedSummary::default() };

    summary.left_to_cargo = unindexed.len();
    for pkg_id in &graph.active_ids {
        if unindexed.contains(pkg_id) {
            continue;
        }
        if dirty.contains(pkg_id) {
            summary.invalidated_pkg_ids.insert(pkg_id.clone());
            continue;
        }
        let Some(group) = matched.get(pkg_id) else {
            summary.invalidated_pkg_ids.insert(pkg_id.clone());
            continue;
        };
        if copy_seeded_group(group, variant_out_dir, &dst_deps, cargo_lock_hash) {
            summary.seeded_pkg_ids.insert(pkg_id.clone());
        } else {
            reasons.insert(pkg_id.clone(), "its shard could not be copied".into());
            summary.invalidated_pkg_ids.insert(pkg_id.clone());
        }
    }

    prune_invalidated_artifacts(graph, &summary.invalidated_pkg_ids, variant_out_dir, cargo_target_dir, profile_name);
    reasons.retain(|k, _| summary.invalidated_pkg_ids.contains(k));
    summary.reasons = reasons;
    summary
}
