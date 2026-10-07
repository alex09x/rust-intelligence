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

use super::manifest::{publish_manifest, ManifestParams};
use super::types::{IndexOptions, PublishedVariant};
use colored::*;
use ridx_format::WorkspaceIndex;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

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
    let mut workspace_crates: Vec<(String, Vec<String>, HashMap<String, String>, HashMap<String, u64>, PathBuf)> = Vec::new();

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

    // Identify all modified files in the workspace
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

    // Determine which crates own the modified files
    let mut changed_crates = Vec::new();
    let mut changed_crate_names = HashSet::new();

    if !modified_files.is_empty() {
        for (crate_name, args, env_vars, _, json_path) in &workspace_crates {
            // Find root source file in args to determine crate directory
            let root_rs = args.iter().find(|a| a.ends_with(".rs"));
            let crate_dir_prefix = if let Some(rs) = root_rs {
                let p = Path::new(rs);
                if let Some(parent) = p.parent() {
                    if let Some(crate_dir) = parent.parent() {
                        crate_dir.to_string_lossy().to_string()
                    } else {
                        parent.to_string_lossy().to_string()
                    }
                } else {
                    String::new()
                }
            } else {
                String::new()
            };

            let owns_modified = modified_files.iter().any(|f| {
                if crate_dir_prefix.is_empty() {
                    true
                } else {
                    f.contains(&crate_dir_prefix)
                }
            });

            if owns_modified {
                changed_crates.push((crate_name, args, env_vars, json_path));
                changed_crate_names.insert(crate_name.clone());
            }
        }
    }

    // If more than 2 distinct workspace crates changed, let Cargo orchestrate
    if changed_crate_names.len() > 2 {
        return None;
    }

    // Execute direct indexing on changed crates
    for (_, args, env_vars, _) in &changed_crates {
        let mut cmd = Command::new(driver_bin);
        if args.len() > 1 {
            cmd.args(&args[1..]);
        } else {
            return None;
        }
        cmd.current_dir(work_dir);
        cmd.env("RIDX_OUT_DIR", variant_out_dir);
        cmd.env("RIDX_CACHE_DIR", cache_dir);
        cmd.env("RIDX_WORKSPACE_ROOT", work_dir);
        cmd.env("RUSTC_BOOTSTRAP", "1");
        for (k, v) in *env_vars {
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
        changed_crate_names.len()
    );

    Some(PublishedVariant {
        base_out_dir: base_out_dir.to_path_buf(),
        variant_out_dir: variant_out_dir.to_path_buf(),
        variant_id: variant_id.to_string(),
    })
}
