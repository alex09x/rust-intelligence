/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Execution and orchestration of Cargo indexing with compiler driver wrapping.

use super::builder::{build_check_command, IndexBuilder};
use super::cargo::{extract_config_target, find_cargo_configs};
use super::direct::try_direct_crate_indexing;
use super::driver::find_driver_binary;
use super::manifest::{publish_manifest, ManifestParams};
use super::types::{IndexOptions, PublishedVariant};
use super::variant::compute_variant_id;
use colored::*;
use ridx_format::WorkspaceIndex;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

pub fn try_run_indexing(opts: IndexOptions) -> Result<PublishedVariant, String> {
    let path = opts.path.clone();
    let out_dir_opt = opts.out_dir_opt.clone();
    let package = opts.package.clone();
    let release = opts.release;
    let features = opts.features.clone();
    let no_default_features = opts.no_default_features;
    let all_features = opts.all_features;
    let target = opts.target.clone();

    let (work_dir, manifest_path) = if path.is_file() {
        if path.file_name().and_then(|s| s.to_str()) != Some("Cargo.toml") {
            return Err(format!("target path {:?} is a file but not a Cargo.toml manifest.", path));
        }
        let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let dir = canon.parent().unwrap_or_else(|| Path::new("/")).to_path_buf();
        (dir, canon)
    } else if path.is_dir() {
        let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let manifest = canon.join("Cargo.toml");
        if !manifest.is_file() {
            return Err(format!("no Cargo.toml found in directory {:?}.", path));
        }
        (canon, manifest)
    } else {
        return Err(format!("path {:?} does not exist.", path));
    };

    let start = Instant::now();
    let base_out_dir = match out_dir_opt {
        Some(d) => {
            if d.is_absolute() {
                d
            } else {
                std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")).join(d)
            }
        }
        None => work_dir.join("target/ridx"),
    };
    if let Err(e) = fs::create_dir_all(&base_out_dir) {
        return Err(format!("failed to create output directory {:?}: {e}", base_out_dir));
    }
    let base_out_dir = base_out_dir.canonicalize().unwrap_or(base_out_dir);

    let driver_bin = match opts.driver_bin.clone() {
        Some(b) if b.is_file() => b,
        Some(_) => {
            return Err("could not find ridx-driver binary. Please build ridx-driver first (e.g. cargo build -p ridx-driver) or place it in PATH.".to_string());
        }
        None => match find_driver_binary() {
            Some(b) => b,
            None => {
                return Err("could not find ridx-driver binary. Please build ridx-driver first (e.g. cargo build -p ridx-driver) or place it in PATH.".to_string());
            }
        },
    };

    let profile_name = if release { "release" } else { "dev" };
    let variant_id = compute_variant_id(profile_name, target.as_deref(), package.as_deref(), features.as_deref(), no_default_features, all_features, &manifest_path);

    let config_target = find_cargo_configs(&work_dir).iter().find_map(|cfg| fs::read_to_string(cfg).ok().and_then(|c| extract_config_target(&c)));
    let effective_target = target.clone().or_else(|| std::env::var("CARGO_BUILD_TARGET").ok()).or(config_target);

    let variant_rel_dir = format!("variants/{}", variant_id);
    let variant_out_dir = base_out_dir.join(&variant_rel_dir);
    if let Err(e) = fs::create_dir_all(&variant_out_dir) {
        return Err(format!("failed to create variant directory {:?}: {e}", variant_out_dir));
    }
    let variant_out_dir = variant_out_dir.canonicalize().unwrap_or(variant_out_dir);

    // The variant active before this run: unaffected crates are seeded from here on feature switches,
    // and a crate that fails to build now keeps its shards from there.
    let previous_variant = fs::read(base_out_dir.join("ridx_manifest.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|m| m["variant_dir"].as_str().map(|d| base_out_dir.join(d)))
        .filter(|d| d != &variant_out_dir && d.is_dir());

    let cargo_cache_id = super::feature_graph::compute_shared_cargo_cache_id(profile_name, effective_target.as_deref());
    let cargo_target_dir = work_dir.join("target/ridx_cargo").join(&cargo_cache_id);
    let _ = fs::create_dir_all(&cargo_target_dir);
    let check_started = std::time::SystemTime::now();

    eprintln!("{}", "═══════════════════════════════════════════════════════════════════════".dimmed());
    eprintln!("  {} {}", "RIDX Workspace Indexer:".bold(), work_dir.display().to_string().cyan());
    eprintln!("{}", "═══════════════════════════════════════════════════════════════════════".dimmed());
    eprintln!("  Work Directory : {}", work_dir.display().to_string().green());
    eprintln!("  Manifest Path  : {}", manifest_path.display().to_string().green());
    eprintln!("  Driver Binary  : {}", driver_bin.display().to_string().green());
    eprintln!("  Index Output   : {}", variant_out_dir.display().to_string().green());
    eprintln!("  Active Variant : {}", variant_id.cyan().bold());
    eprintln!("  Build Profile  : {}", profile_name.cyan());
    if let Some(ref feats) = features {
        eprintln!("  Features       : {}", feats.cyan());
    }
    if no_default_features {
        eprintln!("  No Default Feat: true");
    }
    if all_features {
        eprintln!("  All Features   : true");
    }
    if let Some(ref tgt) = target {
        eprintln!("  Target Triple  : {}", tgt.cyan());
    }
    let cache_dir = opts.cache_dir.clone().unwrap_or_else(ridx_format::resolve_cache_dir);
    let _ = fs::create_dir_all(cache_dir.join("objects"));

    let ld_path = super::cargo::compute_sysroot_ld_path();

    let deps_dir = variant_out_dir.join(".deps");
    if deps_dir.is_dir() {
        if let Some(res) = try_direct_crate_indexing(
            &deps_dir,
            &variant_out_dir,
            &base_out_dir,
            &work_dir,
            &manifest_path,
            &driver_bin,
            &cache_dir,
            ld_path.as_deref(),
            &variant_id,
            &variant_rel_dir,
            profile_name,
            &opts,
            start,
        ) {
            return Ok(res);
        }
    }

    let builder = IndexBuilder::resolve(opts.builder);

    let feature_graph = super::feature_graph::ResolvedFeatureGraph::resolve_with_builder(
        builder,
        &manifest_path,
        effective_target.as_deref(),
        package.as_deref(),
        features.as_deref(),
        no_default_features,
        all_features,
    );
    let initial_lock_hash = super::feature_graph::compute_cargo_lock_hash(&work_dir);
    if let Some(ref fg) = feature_graph {
        super::feature_graph::seed_unaffected_and_prune_invalidated(
            fg,
            &base_out_dir,
            &variant_out_dir,
            previous_variant.as_deref(),
            &cargo_target_dir,
            profile_name,
            initial_lock_hash.as_deref(),
        );
    } else if !variant_has_shards(&variant_out_dir) {
        let _ = fs::remove_dir_all(&cargo_target_dir);
        let _ = fs::create_dir_all(&cargo_target_dir);
    }

    eprintln!("  Action         : {} check with RUSTC_WRAPPER...", builder.as_str());

    let target_kinds_map = super::shards::extract_target_kinds(&manifest_path);
    let target_kinds_file = variant_out_dir.join("target_kinds.json");
    if let Ok(json) = serde_json::to_string(&target_kinds_map) {
        let _ = fs::write(&target_kinds_file, json);
    }

    let (program, args, envs) =
        build_check_command(&opts, builder, &manifest_path, &work_dir, &driver_bin, &variant_out_dir, &cache_dir, &cargo_target_dir, &target_kinds_file, ld_path.as_deref());

    let run_check = |args: &[String]| -> Result<(bool, String, String, Option<i32>), String> {
        let mut cmd = Command::new(&program);
        cmd.current_dir(&work_dir);
        cmd.args(args);
        for (k, v) in &envs {
            cmd.env(k, v);
        }
        if let Some(ref lh) = initial_lock_hash {
            cmd.env("RIDX_CARGO_LOCK_HASH", lh);
        }
        let failed = |e: std::io::Error| format!("failed to execute {} check: {e}", builder.as_str());
        if opts.capture_child_output {
            let output = cmd.output().map_err(failed)?;
            let stderr_str = String::from_utf8_lossy(&output.stderr);
            let tail: Vec<&str> = stderr_str.lines().rev().take(20).collect();
            Ok((output.status.success(), output.status.to_string(), tail.into_iter().rev().collect::<Vec<_>>().join("\n"), output.status.code()))
        } else {
            let status = cmd.status().map_err(failed)?;
            Ok((status.success(), status.to_string(), String::new(), status.code()))
        }
    };

    let (mut ok, mut status, mut stderr_tail, code) = run_check(&args)?;
    // Exit status 2 is a rejected command line: a Mitts release without `--keep-going`. It stops at the first
    // failed unit, so after a failure the libraries and binaries that unit kept from building are indexed alone.
    if !ok && builder == IndexBuilder::Mitts && code == Some(2) {
        (ok, status, stderr_tail, _) = run_check(&super::builder::without_keep_going(&args))?;
        if !ok {
            eprintln!("  warning: mitts check exited with status {status}; indexing libraries and binaries without the other targets");
            let (lib_ok, lib_status, _, _) = run_check(&super::builder::lib_and_bin_fallback_args(&args))?;
            if !lib_ok {
                eprintln!("  warning: mitts check of libraries and binaries exited with status {lib_status}");
            }
        }
    }
    // The crates that built are indexed even when another failed: publish them.
    if !ok && variant_has_shards(&variant_out_dir) {
        eprintln!("  warning: {} check exited with status {status}; publishing the crates that built", builder.as_str());
        keep_last_good_shards(&variant_out_dir, check_started, previous_variant.as_deref());
    } else if !ok {
        if stderr_tail.is_empty() {
            return Err(format!("{} check exited with status {status}", builder.as_str()));
        }
        return Err(format!("{} check exited with status {status}\n{stderr_tail}", builder.as_str()));
    }

    if !all_features {
        let feats: Vec<String> = features.as_deref().unwrap_or_default().split([',', ' ']).filter(|f| !f.is_empty()).map(str::to_string).collect();
        super::required_features::index_skipped_targets(&program, &envs, &work_dir, &manifest_path, &variant_out_dir, &feats, no_default_features);
    }

    let elapsed = start.elapsed();

    let ws = match WorkspaceIndex::open(&variant_out_dir) {
        Ok(w) => w,
        Err(e) => {
            return Err(format!("failed to open generated variant index {:?}: {e}", variant_out_dir));
        }
    };

    let stats = ws.stats();

    // Record build variant fingerprint manifest atomically in base_out_dir
    let manifest_file = base_out_dir.join("ridx_manifest.json");

    // Collect external dependency shards recorded by compiler driver
    let deps_dir = variant_out_dir.join(".deps");
    let (dep_shards, _ws_shards) = super::shards::collect_shards_from_deps_dir(&deps_dir);
    let final_lock_hash = super::feature_graph::compute_cargo_lock_hash(&work_dir).or(initial_lock_hash);
    let empty_features = std::collections::BTreeMap::new();
    let crate_features = feature_graph.as_ref().map(|fg| &fg.crate_features).unwrap_or(&empty_features);

    let sysroot_cas_key = super::manifest::resolve_sysroot_cas_key(&deps_dir, &manifest_file, &driver_bin, effective_target.as_deref());

    publish_manifest(ManifestParams {
        variant_id: &variant_id,
        variant_rel_dir: &variant_rel_dir,
        base_out_dir: &base_out_dir,
        variant_out_dir: &variant_out_dir,
        manifest_file: &manifest_file,
        work_dir: &work_dir,
        manifest_path: &manifest_path,
        profile_name,
        target: effective_target,
        features,
        no_default_features,
        all_features,
        package,
        stats: &stats,
        dep_shards: &dep_shards,
        sysroot_cas_key: sysroot_cas_key.as_deref(),
        cargo_lock_hash: final_lock_hash.as_deref(),
        crate_features,
    })?;

    eprintln!("\n{} Indexed workspace in {:.2} s (active variant: {})", "Success:".green().bold(), elapsed.as_secs_f64(), variant_id.cyan().bold());

    eprintln!(
        "Indexed {} crate(s), {} total definitions, {} references, {} types ({:.2} KB total index size)",
        stats.total_crates.to_string().bold(),
        stats.total_defs.to_string().green(),
        stats.total_refs.to_string().green(),
        stats.total_types.to_string().green(),
        stats.total_bytes as f64 / 1024.0
    );

    if !dep_shards.is_empty() {
        eprintln!("Linked {} external dependency shard(s) from global cache", dep_shards.len().to_string().cyan().bold());
    }

    Ok(PublishedVariant { base_out_dir, variant_out_dir, variant_id })
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

/// Whether the variant directory holds any shard.
fn variant_has_shards(dir: &Path) -> bool {
    fs::read_dir(dir).into_iter().flatten().flatten().any(|e| e.path().extension().is_some_and(|x| x == "ridx"))
}

pub fn run_indexing(opts: IndexOptions) -> PublishedVariant {
    match try_run_indexing(opts) {
        Ok(pv) => pv,
        Err(e) => {
            eprintln!("{}: {e}", "Error".red().bold());
            #[cfg(not(test))]
            std::process::exit(1);
            #[cfg(test)]
            panic!("{e}");
        }
    }
}
