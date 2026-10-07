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
}

pub fn publish_manifest(params: ManifestParams<'_>) -> Result<(), String> {
    let config_hashes =
        crate::indexer::direct::config::compute_workspace_config_hashes(params.work_dir);

    let manifest_data = serde_json::json!({
        "version": "2.0",
        "format_version": ridx_format::CURRENT_FORMAT_VERSION,
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
    let tmp_manifest = params
        .base_out_dir
        .join(format!("ridx_manifest.json.tmp.{}", std::process::id()));
    if let Err(e) = fs::write(&tmp_manifest, serialized) {
        return Err(format!(
            "failed to write temporary variant manifest {:?}: {e}",
            tmp_manifest
        ));
    }

    if let Err(e) = fs::rename(&tmp_manifest, params.manifest_file) {
        let _ = fs::remove_file(&tmp_manifest);
        return Err(format!(
            "failed to publish active variant manifest {:?}: {e}",
            params.manifest_file
        ));
    }

    Ok(())
}
