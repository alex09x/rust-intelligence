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

use super::manifest::{publish_manifest, ManifestParams};
use super::types::{IndexOptions, PublishedVariant};
use args::sanitize_compiler_args_for_direct;
use colored::*;
use config::{has_untracked_rs_files, verify_build_config_unchanged};
use graph::{build_workspace_dependency_graphs, topo_sort_crates};
use ridx_format::WorkspaceIndex;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

pub type WorkspaceCrateEntry = (
    String,
    String,
    Vec<String>,
    HashMap<String, String>,
    HashMap<String, u64>,
    PathBuf,
);

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
    let mut workspace_crates: Vec<WorkspaceCrateEntry> = Vec::new();

    let entries = fs::read_dir(deps_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }

        let content = fs::read(&path).ok()?;
        let val: serde_json::Value = serde_json::from_slice(&content).ok()?;

        let is_dep = val
            .get("is_dependency")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if is_dep {
            if let Ok(info) = serde_json::from_value::<ridx_format::DependencyShardInfo>(val) {
                dep_shards.push(info);
            }
        } else {
            let crate_name = val
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let args: Vec<String> = val
                .get("compiler_args")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();

            if args.is_empty() {
                eprintln!("[ridx-direct] fallback to Cargo: compiler args missing for crate '{crate_name}'");
                return None;
            }

            let file_hashes: HashMap<String, u64> = val
                .get("file_hashes")
                .and_then(|v| v.as_object())
                .map(|obj| {
                    obj.iter()
                        .filter_map(|(k, v)| v.as_u64().map(|h| (k.clone(), h)))
                        .collect()
                })
                .unwrap_or_default();

            let env_vars: HashMap<String, String> = val
                .get("env")
                .and_then(|v| v.as_object())
                .map(|obj| {
                    obj.iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect()
                })
                .unwrap_or_default();

            let target_id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(&crate_name)
                .to_string();
            workspace_crates.push((target_id, crate_name, args, env_vars, file_hashes, path));
        }
    }

    if workspace_crates.is_empty() {
        eprintln!("[ridx-direct] fallback to Cargo: no workspace crates found in .deps");
        return None;
    }

    // Verify build configuration (Cargo.toml, Cargo.lock, build.rs, options) is unchanged
    let manifest_mtime = verify_build_config_unchanged(work_dir, base_out_dir, opts)?;

    // Identify all modified files and tracked files in the workspace
    let mut modified_files = HashSet::new();
    let mut all_checked_files = HashSet::new();
    for (_, _, _, _, file_hashes, _) in &workspace_crates {
        for (file_str, &recorded_hash) in file_hashes {
            if !all_checked_files.insert(file_str.clone()) {
                continue;
            }
            let p = Path::new(file_str);
            if !p.is_file() {
                modified_files.insert(file_str.clone());
            } else if let Ok(bytes) = fs::read(p) {
                let current_hash = ridx_format::fnv1a_64(&bytes);
                if current_hash != recorded_hash {
                    modified_files.insert(file_str.clone());
                }
            } else {
                modified_files.insert(file_str.clone());
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

    // Build dependency and reverse-dependency graphs among workspace targets
    let (dependencies_of, rdeps_of) = build_workspace_dependency_graphs(&workspace_crates);

    // Map target_ids back to workspace crate data
    let crate_map: HashMap<String, _> = workspace_crates
        .into_iter()
        .map(|(tid, name, args, env_vars, file_hashes, path)| {
            (tid, (name, args, env_vars, file_hashes, path))
        })
        .collect();

    // Determine directly modified target_ids
    let mut initial_changed = HashSet::new();
    if !modified_files.is_empty() {
        for (target_id, (_, _, _, file_hashes, _)) in &crate_map {
            if file_hashes.keys().any(|f| modified_files.contains(f)) {
                initial_changed.insert(target_id.clone());
            }
        }
    }

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
    let affected_packages: HashSet<String> = affected_target_ids
        .iter()
        .filter_map(|tid| {
            crate_map.get(tid).map(|(name, _, env, ..)| {
                env.get("CARGO_PKG_NAME")
                    .cloned()
                    .unwrap_or_else(|| name.clone())
            })
        })
        .collect();

    // Proc-macro crates produce dynamic libraries needed at compile time by downstream crates.
    // Direct indexing compiles with --emit=metadata which only produces .rmeta, causing downstream
    // compilation to fail if a proc-macro was modified or recompiled.
    for tid in &affected_target_ids {
        if let Some((crate_name, args, ..)) = crate_map.get(tid) {
            let is_proc_macro = args
                .windows(2)
                .any(|w| w[0] == "--crate-type" && w[1] == "proc-macro")
                || args.iter().any(|a| a == "--crate-type=proc-macro");
            if is_proc_macro {
                eprintln!(
                    "[ridx-direct] fallback to Cargo: affected crate '{crate_name}' is a proc-macro"
                );
                return None;
            }
        }
    }

    // If more than 10 distinct workspace packages changed, let Cargo orchestrate
    if affected_packages.len() > 10 {
        eprintln!(
            "[ridx-direct] fallback to Cargo: too many affected packages ({} > 10: {:?})",
            affected_packages.len(),
            affected_packages
        );
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
    let _ = fs::remove_dir_all(&direct_deps);
    fs::create_dir_all(&direct_deps).ok()?;
    fs::create_dir_all(&direct_incr).ok()?;

    // Execute direct indexing on changed targets in topological order
    for target_id in &sorted_target_ids {
        let (crate_name, args, env_vars, _, _) = crate_map.get(target_id)?;
        let sanitized_args =
            sanitize_compiler_args_for_direct(args, &direct_deps, &direct_incr, &recompiled_crates);

        let mut cmd = Command::new(driver_bin);
        if sanitized_args.is_empty() {
            eprintln!("[ridx-direct] fallback to Cargo: empty sanitized args for '{crate_name}'");
            return None;
        }
        cmd.args(&sanitized_args);
        cmd.current_dir(work_dir);
        cmd.env("RIDX_OUT_DIR", variant_out_dir);
        cmd.env("RIDX_CACHE_DIR", cache_dir);
        cmd.env("RIDX_WORKSPACE_ROOT", work_dir);
        cmd.env("RUSTC_BOOTSTRAP", "1");
        for (k, v) in env_vars {
            cmd.env(k, v);
        }
        if let Some(ld) = ld_path {
            cmd.env("LD_LIBRARY_PATH", ld);
            cmd.env("DYLD_FALLBACK_LIBRARY_PATH", ld);
        }

        let status = cmd.status().ok()?;
        if !status.success() {
            eprintln!(
                "[ridx-direct] fallback to Cargo: direct rustc execution failed for '{crate_name}'"
            );
            return None;
        }
    }

    let ws = WorkspaceIndex::open(variant_out_dir).ok()?;
    let stats = ws.stats();
    let manifest_file = base_out_dir.join("ridx_manifest.json");
    let prev_cas_key = fs::read(&manifest_file)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<ridx_format::VariantManifest>(&bytes).ok())
        .and_then(|m| m.sysroot_cas_key);

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
        sysroot_cas_key: prev_cas_key.as_deref(),
    };

    publish_manifest(params).ok()?;

    let dur = start.elapsed().as_secs_f64() * 1000.0;
    eprintln!(
        "  {} Direct fast-path indexing finished in {:.1} ms ({} changed crates)",
        "RIDX Success:".green().bold(),
        dur,
        sorted_target_ids.len()
    );

    Some(PublishedVariant {
        base_out_dir: base_out_dir.to_path_buf(),
        variant_out_dir: variant_out_dir.to_path_buf(),
        variant_id: variant_id.to_string(),
    })
}
