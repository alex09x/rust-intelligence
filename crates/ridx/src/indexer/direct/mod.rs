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

            workspace_crates.push((crate_name, args, env_vars, file_hashes, path));
        }
    }

    if workspace_crates.is_empty() {
        return None;
    }

    // Verify build configuration (Cargo.toml, Cargo.lock, build.rs, options) is unchanged
    verify_build_config_unchanged(work_dir, base_out_dir, opts, &workspace_crates)?;

    // Identify all modified files and tracked files in the workspace
    let mut modified_files = HashSet::new();
    let mut all_checked_files = HashSet::new();
    for (_, _, _, file_hashes, _) in &workspace_crates {
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

    // Check for untracked .rs files in crate source directories
    for (_, _, env_vars, _, _) in &workspace_crates {
        if let Some(mdir_str) = env_vars.get("CARGO_MANIFEST_DIR") {
            let src_dir = Path::new(mdir_str).join("src");
            if has_untracked_rs_files(&src_dir, &all_checked_files) {
                return None;
            }
        }
    }

    // Build dependency and reverse-dependency graphs among workspace crates
    let (dependencies_of, rdeps_of) = build_workspace_dependency_graphs(&workspace_crates);

    // Determine directly modified crates
    let mut initial_changed = HashSet::new();
    if !modified_files.is_empty() {
        for (crate_name, _, _, file_hashes, _) in &workspace_crates {
            if file_hashes.keys().any(|f| modified_files.contains(f)) {
                initial_changed.insert(crate_name.clone());
            }
        }
    }

    if initial_changed.is_empty() {
        return None;
    }

    // Expand with reverse dependencies (crates that depend on modified crates)
    let mut affected_crate_names = initial_changed.clone();
    let mut queue: VecDeque<String> = initial_changed.into_iter().collect();
    while let Some(cname) = queue.pop_front() {
        if let Some(rdeps) = rdeps_of.get(&cname) {
            for rdep in rdeps {
                if affected_crate_names.insert(rdep.clone()) {
                    queue.push_back(rdep.clone());
                }
            }
        }
    }

    // If more than 3 distinct workspace crates changed, let Cargo orchestrate
    if affected_crate_names.len() > 3 {
        return None;
    }

    // Topological sort of affected crates (dependencies before dependents)
    let sorted_names = topo_sort_crates(&affected_crate_names, &dependencies_of)?;

    // Map sorted names back to workspace crate data
    let crate_map: HashMap<String, _> = workspace_crates
        .into_iter()
        .map(|(name, args, env_vars, file_hashes, path)| {
            (name, (args, env_vars, file_hashes, path))
        })
        .collect();

    // Prepare private isolated directories for direct build artifacts and incremental cache
    let direct_dir = work_dir.join("target/ridx-direct");
    let direct_deps = direct_dir.join("deps");
    let direct_incr = direct_dir.join("incremental");
    fs::create_dir_all(&direct_deps).ok()?;
    fs::create_dir_all(&direct_incr).ok()?;

    // Execute direct indexing on changed crates in topological order
    for crate_name in &sorted_names {
        let (args, env_vars, _, _) = crate_map.get(crate_name)?;
        let sanitized_args = sanitize_compiler_args_for_direct(args, &direct_deps, &direct_incr);

        let mut cmd = Command::new(driver_bin);
        if sanitized_args.is_empty() {
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
            return None;
        }
    }

    let ws = WorkspaceIndex::open(variant_out_dir).ok()?;
    let stats = ws.stats();
    let manifest_file = base_out_dir.join("ridx_manifest.json");

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
    };

    publish_manifest(params).ok()?;

    let dur = start.elapsed().as_secs_f64() * 1000.0;
    eprintln!(
        "  {} Direct fast-path indexing finished in {:.1} ms ({} changed crates)",
        "RIDX Success:".green().bold(),
        dur,
        sorted_names.len()
    );

    Some(PublishedVariant {
        base_out_dir: base_out_dir.to_path_buf(),
        variant_out_dir: variant_out_dir.to_path_buf(),
        variant_id: variant_id.to_string(),
    })
}
