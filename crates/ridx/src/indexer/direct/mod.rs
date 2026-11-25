/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Fast-path direct crate invocation bypassing Cargo orchestration on incremental edits.

pub mod args;
pub mod config;
pub mod graph;
pub(crate) mod units;

use super::manifest::{publish_manifest, ManifestParams};
use super::types::{IndexOptions, PublishedVariant};
use args::sanitize_compiler_args_for_direct;
use colored::*;
use config::{has_untracked_rs_files, verify_build_config_unchanged};
use graph::{build_workspace_dependency_graphs, topo_sort_crates, UnitOutcome};
use ridx_format::WorkspaceIndex;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

pub type WorkspaceCrateEntry = (String, String, Vec<String>, HashMap<String, String>, HashMap<String, u64>, PathBuf);

#[allow(clippy::too_many_arguments)]
pub fn try_direct_crate_indexing(
    deps_dir: &Path,
    variant_out_dir: &Path,
    base_out_dir: &Path,
    work_dir: &Path,
    manifest_path: &Path,
    driver_bin: &Path,
    cache_dir: &Path,
    ld_path: Option<&str>,
    variant_id: &str,
    variant_rel_dir: &str,
    profile_name: &str,
    opts: &IndexOptions,
    start: Instant,
) -> Option<PublishedVariant> {
    if !deps_dir.is_dir() {
        eprintln!("[ridx-direct] fallback to Cargo: deps directory missing (cold index)");
        return None;
    }

    let mut dep_shards = Vec::new();
    let mut sidecars: Vec<(WorkspaceCrateEntry, Option<String>)> = Vec::new();

    let entries = fs::read_dir(deps_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }

        let content = fs::read(&path).ok()?;
        let val: serde_json::Value = serde_json::from_slice(&content).ok()?;

        let is_dep = val.get("is_dependency").and_then(|v| v.as_bool()).unwrap_or(false);
        if is_dep {
            if let Ok(info) = serde_json::from_value::<ridx_format::DependencyShardInfo>(val) {
                dep_shards.push(info);
            }
        } else {
            let crate_name = val.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let args: Vec<String> =
                val.get("compiler_args").and_then(|v| v.as_array()).map(|arr| arr.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();

            if args.is_empty() {
                eprintln!("[ridx-direct] fallback to Cargo: compiler args missing for crate '{crate_name}'");
                return None;
            }

            let file_hashes: HashMap<String, u64> =
                val.get("file_hashes").and_then(|v| v.as_object()).map(|obj| obj.iter().filter_map(|(k, v)| v.as_u64().map(|h| (k.clone(), h))).collect()).unwrap_or_default();
            let file_hashes = units::own_file_hashes(file_hashes, val.get("own_sources"), work_dir);

            let env_vars: HashMap<String, String> =
                val.get("env").and_then(|v| v.as_object()).map(|obj| obj.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect()).unwrap_or_default();

            let object = val.get("object_path").and_then(|v| v.as_str()).map(str::to_string);
            let target_id = path.file_stem().and_then(|s| s.to_str()).unwrap_or(&crate_name).to_string();
            sidecars.push(((target_id, crate_name, args, env_vars, file_hashes, path), object));
        }
    }
    // On BTCR an edit of one file compiled 53 sidecars, each of the ~20 units two or three times over.
    let (workspace_crates, aliases) = units::group_by_unit(sidecars);

    if workspace_crates.is_empty() {
        eprintln!("[ridx-direct] fallback to Cargo: no workspace crates found in .deps");
        return None;
    }

    // Verify build configuration (Cargo.toml, Cargo.lock, build.rs, options) is unchanged
    let manifest_mtime = verify_build_config_unchanged(work_dir, base_out_dir, opts)?;
    // A crate that failed in the last check has no sidecar here: neither rebuilt nor seen as changed, it stayed out of
    // the index for good (review of 3690a98d). Only Cargo builds it.
    let (active_variant, complete) = published_variant(base_out_dir)?;
    if !complete {
        eprintln!("[ridx-direct] fallback to Cargo: the last check did not build every crate");
        return None;
    }

    // Each unit against the hashes it recorded itself: two units reading one file may have read two texts of it (a
    // unit left out behind a dependency with errors keeps the hashes of the text before).
    let mut modified_files = HashSet::new();
    let mut changed_units = HashSet::new();
    let mut all_checked_files = HashSet::new();
    let mut current: HashMap<String, Option<(u64, u64)>> = HashMap::new();
    for (target_id, _, _, _, file_hashes, _) in &workspace_crates {
        for (file_str, &recorded_hash) in file_hashes {
            all_checked_files.insert(file_str.clone());
            let now = *current.entry(file_str.clone()).or_insert_with(|| fs::read(work_dir.join(file_str)).ok().map(|b| units::source_hashes(&b)));
            if !now.is_some_and(|now| units::hash_matches(now, recorded_hash)) {
                modified_files.insert(file_str.clone());
                changed_units.insert(target_id.clone());
            }
        }
    }

    // Also include tracked workspace configuration files (Cargo.toml, build.rs, Cargo.lock, etc.) in all_checked_files
    let config_hashes = config::compute_workspace_config_hashes(work_dir);
    for rel_path in config_hashes.keys() {
        all_checked_files.insert(rel_path.clone());
        let full = work_dir.join(rel_path);
        all_checked_files.insert(full.display().to_string());
        if let Ok(canon) = full.canonicalize() {
            all_checked_files.insert(canon.display().to_string());
        }
    }

    // Check for untracked .rs files in crate target directories (src, tests, benches, examples, build.rs)
    for (_, _, _, env_vars, _, _) in &workspace_crates {
        if let Some(mdir_str) = env_vars.get("CARGO_MANIFEST_DIR") {
            let crate_dir = Path::new(mdir_str);
            if has_untracked_rs_files(crate_dir, &all_checked_files, manifest_mtime) {
                return None;
            }
        }
    }

    // Nothing to do when no source, no build configuration and no option changed since the published index: Zed reports
    // the whole tree changed after a rescan, and on BTCR each such report cost a full `cargo check` (~70 s, rustc and
    // ridx-driver at 1.5–3 GB) that rebuilt the same index.
    if modified_files.is_empty() && active_variant == variant_rel_dir {
        eprintln!("  {} index is up to date in {:.1} ms: no source or build configuration changed", "RIDX Success:".green().bold(), start.elapsed().as_secs_f64() * 1000.0);
        return Some(PublishedVariant { base_out_dir: base_out_dir.to_path_buf(), variant_out_dir: variant_out_dir.to_path_buf(), variant_id: variant_id.to_string() });
    }

    // Build dependency and reverse-dependency graphs among workspace targets
    let (dependencies_of, rdeps_of) = build_workspace_dependency_graphs(&workspace_crates);

    // Map target_ids back to workspace crate data
    let crate_map: HashMap<String, _> =
        workspace_crates.into_iter().map(|(tid, name, args, env_vars, file_hashes, path)| (tid, (name, args, env_vars, file_hashes, path))).collect();

    let initial_changed = changed_units;
    if initial_changed.is_empty() {
        eprintln!("[ridx-direct] fallback to Cargo: no workspace crate files modified");
        return None;
    }

    // Expand with reverse dependencies
    let mut affected_target_ids = initial_changed.clone();
    let mut queue: VecDeque<String> = initial_changed.into_iter().collect();
    while let Some(tid) = queue.pop_front() {
        if let Some(rdeps) = rdeps_of.get(&tid) {
            for rdep in rdeps {
                if affected_target_ids.insert(rdep.clone()) {
                    queue.push_back(rdep.clone());
                }
            }
        }
    }

    // Collect distinct affected package names
    let affected_packages: HashSet<String> =
        affected_target_ids.iter().filter_map(|tid| crate_map.get(tid).map(|(name, _, env, ..)| env.get("CARGO_PKG_NAME").cloned().unwrap_or_else(|| name.clone()))).collect();

    // Proc-macro crates produce dynamic libraries needed at compile time by downstream crates.
    // Direct indexing compiles with --emit=metadata which only produces .rmeta, causing downstream
    // compilation to fail if a proc-macro was modified or recompiled.
    for tid in &affected_target_ids {
        if let Some((crate_name, args, ..)) = crate_map.get(tid) {
            let is_proc_macro = args.windows(2).any(|w| w[0] == "--crate-type" && w[1] == "proc-macro") || args.iter().any(|a| a == "--crate-type=proc-macro");
            if is_proc_macro {
                eprintln!("[ridx-direct] fallback to Cargo: affected crate '{crate_name}' is a proc-macro");
                return None;
            }
        }
    }

    // If more than 10 distinct workspace packages changed, let Cargo orchestrate
    if affected_packages.len() > 10 {
        eprintln!("[ridx-direct] fallback to Cargo: too many affected packages ({} > 10: {:?})", affected_packages.len(), affected_packages);
        return None;
    }

    // Topological sort of affected targets (dependencies before dependents)
    let sorted_target_ids = match topo_sort_crates(&affected_target_ids, &dependencies_of) {
        Some(s) => s,
        None => {
            eprintln!("[ridx-direct] fallback to Cargo: cyclic or unresolvable dependencies among affected targets");
            return None;
        }
    };

    let mut recompiled_crates = HashSet::new();
    for tid in &affected_target_ids {
        if let Some((name, args, env, ..)) = crate_map.get(tid) {
            let is_lib = !args.contains(&"--test".to_string());
            if is_lib {
                recompiled_crates.insert(tid.clone());
                recompiled_crates.insert(name.clone());
                let norm = name.replace('-', "_");
                recompiled_crates.insert(norm);
                if let Some(pkg) = env.get("CARGO_PKG_NAME") {
                    recompiled_crates.insert(pkg.clone());
                    recompiled_crates.insert(pkg.replace('-', "_"));
                }
            }
        }
    }

    // Prepare private isolated directories for direct build artifacts and incremental cache
    let direct_dir = work_dir.join("target/ridx-direct");
    let direct_deps = direct_dir.join("deps");
    let direct_incr = direct_dir.join("incremental");
    // Kept from run to run: a unit rebuilt here before is named by the sidecars of its dependents (`--extern` into
    // this directory), and a run after one that rebuilt a dependency failed on its missing `.rmeta` and fell back to
    // Cargo (prod-code, issue #7). A unit rebuilt now overwrites its own.
    fs::create_dir_all(&direct_deps).ok()?;
    fs::create_dir_all(&direct_incr).ok()?;

    let direct_lock_hash = super::feature_graph::compute_cargo_lock_hash(work_dir);
    let target_kinds_file = variant_out_dir.join("target_kinds.json");
    // Execute direct indexing on changed targets, each once its dependencies are built, several at once.
    let compile = |target_id: &str| -> UnitOutcome {
        let Some((crate_name, args, env_vars, _, sidecar)) = crate_map.get(target_id) else { return UnitOutcome::Failed };
        let sanitized_args = sanitize_compiler_args_for_direct(args, &direct_deps, &direct_incr, &recompiled_crates);
        if sanitized_args.is_empty() {
            eprintln!("[ridx-direct] fallback to Cargo: empty sanitized args for '{crate_name}'");
            return UnitOutcome::Failed;
        }
        let mut cmd = Command::new(driver_bin);
        cmd.args(&sanitized_args);
        cmd.envs(crate::indexer::no_build::envs());
        cmd.current_dir(work_dir);
        cmd.env("RIDX_OUT_DIR", variant_out_dir);
        cmd.env("RIDX_CACHE_DIR", cache_dir);
        cmd.env("RIDX_WORKSPACE_ROOT", work_dir);
        if let Some(ref lh) = direct_lock_hash {
            cmd.env("RIDX_CARGO_LOCK_HASH", lh);
        }
        cmd.env("RUSTC_BOOTSTRAP", "1");
        if target_kinds_file.is_file() {
            cmd.env("RIDX_TARGET_KINDS_FILE", &target_kinds_file);
        }
        for (k, v) in env_vars {
            cmd.env(k, v);
        }
        if let Some(ld) = ld_path {
            cmd.env("LD_LIBRARY_PATH", ld);
            cmd.env("DYLD_FALLBACK_LIBRARY_PATH", ld);
        }
        // Compared with its time after the compile, not with the clock: Linux stamps a file with a coarser clock.
        let written = || fs::metadata(sidecar).and_then(|m| m.modified()).ok();
        let (started, before) = (Instant::now(), written());
        let ok = cmd.status().is_ok_and(|s| s.success());
        eprintln!("[ridx-direct] compiled {target_id} in {:.1} s", started.elapsed().as_secs_f64());
        let outcome = if ok {
            UnitOutcome::Built
        } else if written().is_some_and(|t| Some(t) != before) {
            // The driver indexes a workspace crate the compiler stops on (`broken_crate`) and rewrites its sidecar
            // with the hashes of the text it read: the edit fixing it is seen as a change and compiled here too.
            // Cargo would stop on the same errors, after 36 to 40 s on prod-code's workspace (issue #7).
            eprintln!("[ridx-direct] {target_id} has errors: indexed as it is, its dependents keep their last index");
            UnitOutcome::BuiltWithErrors
        } else {
            eprintln!("[ridx-direct] fallback to Cargo: direct rustc execution failed for '{crate_name}'");
            return UnitOutcome::Failed;
        };
        units::refresh_aliases(sidecar, aliases.get(target_id).map_or(&[], Vec::as_slice));
        outcome
    };
    let left_out = graph::run_in_parallel(&sorted_target_ids, &dependencies_of, graph::direct_jobs(), compile)?;
    if !left_out.is_empty() {
        let mut left_out: Vec<String> = left_out.into_iter().collect();
        left_out.sort();
        eprintln!("[ridx-direct] not compiled, a dependency has errors: {}", left_out.join(", "));
    }

    let ws = WorkspaceIndex::open(variant_out_dir).ok()?;
    let stats = ws.stats();
    let manifest_file = base_out_dir.join("ridx_manifest.json");
    let deps_dir = variant_out_dir.join(".deps");
    let sysroot_cas_key = super::manifest::resolve_sysroot_cas_key(&deps_dir, &manifest_file, driver_bin, opts.target.as_deref());

    let prev_crate_features = fs::read(variant_out_dir.join("variant_manifest.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<ridx_format::VariantManifest>(&b).ok())
        .map(|m| m.crate_features)
        .unwrap_or_default();

    let params = ManifestParams {
        variant_id,
        variant_rel_dir,
        base_out_dir,
        variant_out_dir,
        manifest_file: &manifest_file,
        work_dir,
        manifest_path,
        profile_name,
        target: opts.target.clone(),
        features: opts.features.clone(),
        no_default_features: opts.no_default_features,
        all_features: opts.all_features,
        package: opts.package.clone(),
        stats: &stats,
        dep_shards: &dep_shards,
        sysroot_cas_key: sysroot_cas_key.as_deref(),
        cargo_lock_hash: direct_lock_hash.as_deref(),
        crate_features: &prev_crate_features,
        // Reached only from a complete index, and every unit still has its sidecar: rebuilt, or the last one of a unit
        // with errors or left out behind one, whose next change is compiled here too.
        complete: true,
    };

    publish_manifest(params).ok()?;

    let dur = start.elapsed().as_secs_f64() * 1000.0;
    eprintln!("  {} Direct fast-path indexing finished in {:.1} ms ({} changed crates)", "RIDX Success:".green().bold(), dur, sorted_target_ids.len());

    Some(PublishedVariant { base_out_dir: base_out_dir.to_path_buf(), variant_out_dir: variant_out_dir.to_path_buf(), variant_id: variant_id.to_string() })
}

/// The variant the published manifest names (`variants/<id>`), the index a server reads, and whether its check built
/// every crate (a manifest from before the flag counts as partial: one Cargo run settles it).
pub(crate) fn published_variant(base_out_dir: &Path) -> Option<(String, bool)> {
    let bytes = fs::read(base_out_dir.join("ridx_manifest.json")).ok()?;
    let m = serde_json::from_slice::<serde_json::Value>(&bytes).ok()?;
    Some((m["variant_dir"].as_str().unwrap_or_default().to_string(), m["complete"].as_bool() == Some(true)))
}

#[cfg(test)]
mod tests;
