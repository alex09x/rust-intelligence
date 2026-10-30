/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Manifest generation and atomic publication for indexed variants.

use std::fs;
use std::path::Path;

pub struct ManifestParams<'a> {
    pub variant_id: &'a str,
    pub variant_rel_dir: &'a str,
    pub base_out_dir: &'a Path,
    pub variant_out_dir: &'a Path,
    pub manifest_file: &'a Path,
    pub work_dir: &'a Path,
    pub manifest_path: &'a Path,
    pub profile_name: &'a str,
    pub target: Option<String>,
    pub features: Option<String>,
    pub no_default_features: bool,
    pub all_features: bool,
    pub package: Option<String>,
    pub stats: &'a ridx_format::WorkspaceStats,
    pub dep_shards: &'a [ridx_format::DependencyShardInfo],
    pub sysroot_cas_key: Option<&'a str>,
}

pub fn publish_manifest(params: ManifestParams<'_>) -> Result<(), String> {
    let config_hashes = crate::indexer::direct::config::compute_workspace_config_hashes(params.work_dir);

    let prev_gen = if params.manifest_file.exists() {
        fs::read(params.manifest_file).ok().and_then(|bytes| serde_json::from_slice::<ridx_format::VariantManifest>(&bytes).ok()).map(|m| m.generation).unwrap_or(0)
    } else {
        0
    };
    let next_gen = prev_gen + 1;

    let manifest_data = serde_json::json!({
        "version": "2.0",
        "format_version": ridx_format::CURRENT_FORMAT_VERSION,
        "generation": next_gen,
        "extractor_version": ridx_format::EXTRACTOR_VERSION,
        "active_variant_id": params.variant_id,
        "variant_dir": params.variant_rel_dir,
        "base_out_dir": params.base_out_dir.display().to_string(),
        "workspace_root": params.work_dir.display().to_string(),
        "manifest_path": params.manifest_path.display().to_string(),
        "profile": params.profile_name,
        "target": params.target,
        "features": params.features,
        "no_default_features": params.no_default_features,
        "all_features": params.all_features,
        "package_filter": params.package,
        "config_hashes": config_hashes,
        "timestamp_epoch_secs": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        "total_crates": params.stats.total_crates,
        "total_defs": params.stats.total_defs,
        "total_refs": params.stats.total_refs,
        "total_types": params.stats.total_types,
        "dependency_shards": params.dep_shards,
        "sysroot_cas_key": params.sysroot_cas_key,
        "crates": params.stats.crate_summaries.iter().map(|c| serde_json::json!({
            "name": c.name,
            "version": c.version,
            "file": c.path.file_name().and_then(|s| s.to_str()).unwrap_or(""),
            "file_size": c.file_size,
            "is_dependency": c.is_dependency,
        })).collect::<Vec<_>>()
    });

    let serialized = serde_json::to_string_pretty(&manifest_data).unwrap();

    // Also save variant-specific manifest inside variant_out_dir
    let variant_manifest_file = params.variant_out_dir.join("variant_manifest.json");
    let _ = fs::write(&variant_manifest_file, &serialized);

    // Write to temp file then rename for atomic manifest swap
    let tmp_manifest = params.base_out_dir.join(format!("ridx_manifest.json.tmp.{}", std::process::id()));
    if let Err(e) = fs::write(&tmp_manifest, serialized) {
        return Err(format!("failed to write temporary variant manifest {:?}: {e}", tmp_manifest));
    }

    if let Err(e) = fs::rename(&tmp_manifest, params.manifest_file) {
        let _ = fs::remove_file(&tmp_manifest);
        return Err(format!("failed to publish active variant manifest {:?}: {e}", params.manifest_file));
    }

    Ok(())
}

/// Deterministically resolves the sysroot CAS key for a variant:
/// 1. Authoritative: queries the active driver binary directly (`driver_bin --sysroot-cas-key`).
///    Cross-checks against `.deps/*.json` sidecars in `deps_dir`, warning on divergence.
/// 2. Fallback: inspects `.deps/*.json` sidecar files in the variant directory, failing closed if keys conflict.
/// 3. Fallback: reads `sysroot_cas_key` from the existing manifest file.
pub fn resolve_sysroot_cas_key(deps_dir: &Path, manifest_file: &Path, driver_bin: &Path, target: Option<&str>) -> Option<String> {
    // 1. Authoritative: query active driver binary directly
    let mut cmd = std::process::Command::new(driver_bin);
    cmd.arg("--sysroot-cas-key");
    if let Some(tgt) = target {
        cmd.arg("--target").arg(tgt);
    }
    if let Some(ld) = super::cargo::compute_sysroot_ld_path() {
        cmd.env("LD_LIBRARY_PATH", &ld);
        cmd.env("DYLD_FALLBACK_LIBRARY_PATH", &ld);
    }
    let driver_key = if let Ok(out) = cmd.output() {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                Some(s)
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    // 2. Inspect sidecars and verify key consistency
    let mut sidecar_key: Option<String> = None;
    let mut sidecar_conflict = false;
    let mut driver_sidecar_mismatch = false;

    if deps_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(deps_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.extension().and_then(|s| s.to_str()) == Some("json") {
                    if let Ok(content) = fs::read(&p) {
                        if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&content) {
                            if let Some(k) = val["sysroot_cas_key"].as_str() {
                                if let Some(existing) = &sidecar_key {
                                    if existing != k {
                                        sidecar_conflict = true;
                                    }
                                } else {
                                    sidecar_key = Some(k.to_string());
                                }
                                if let Some(dk) = &driver_key {
                                    if dk != k {
                                        driver_sidecar_mismatch = true;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if driver_sidecar_mismatch {
        eprintln!("warning: sysroot_cas_key mismatch between driver ({:?}) and sidecars in {}", driver_key, deps_dir.display());
    }

    if let Some(dk) = driver_key {
        return Some(dk);
    }

    if sidecar_conflict {
        // Conflicting keys across sidecars without authoritative driver -> fail closed
        return None;
    }

    if sidecar_key.is_some() {
        return sidecar_key;
    }

    // 3. Fallback: read from existing manifest
    if let Ok(bytes) = fs::read(manifest_file) {
        if let Ok(m) = serde_json::from_slice::<ridx_format::VariantManifest>(&bytes) {
            if let Some(k) = m.sysroot_cas_key {
                return Some(k);
            }
        }
    }

    None
}
