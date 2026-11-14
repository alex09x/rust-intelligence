/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Computation of stable, hermetic variant cache identifiers.

use super::cargo::{collect_all_workspace_manifests, extract_config_target, find_cargo_configs};
use std::fs;
use std::path::Path;
use std::process::Command;

pub fn compute_variant_id(
    profile: &str,
    target: Option<&str>,
    package: Option<&str>,
    features: Option<&str>,
    no_default_features: bool,
    all_features: bool,
    manifest_path: &Path,
) -> String {
    compute_variant_id_with_env(
        profile,
        target,
        package,
        features,
        no_default_features,
        all_features,
        manifest_path,
        std::env::var("CARGO_BUILD_TARGET").ok().as_deref(),
        std::env::var("RUSTFLAGS").ok().as_deref(),
        std::env::var("CARGO_ENCODED_RUSTFLAGS").ok().as_deref(),
        std::env::var("RUSTUP_TOOLCHAIN").ok().as_deref(),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn compute_variant_id_with_env(
    profile: &str,
    target: Option<&str>,
    package: Option<&str>,
    features: Option<&str>,
    no_default_features: bool,
    all_features: bool,
    manifest_path: &Path,
    env_target: Option<&str>,
    rustflags: Option<&str>,
    encoded_flags: Option<&str>,
    toolchain: Option<&str>,
) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    profile.hash(&mut hasher);

    let canon_manifest = ridx_format::canonicalize_best_effort(manifest_path);
    let work_dir = canon_manifest.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));

    // 1. Cargo configuration files (.cargo/config.toml, .cargo/config)
    let config_files = find_cargo_configs(work_dir);
    let mut config_target = None;
    for cfg in &config_files {
        ridx_format::canonicalize_best_effort(cfg).display().to_string().hash(&mut hasher);
        if let Ok(content) = fs::read_to_string(cfg) {
            content.hash(&mut hasher);
            if config_target.is_none() {
                config_target = extract_config_target(&content);
            }
        }
    }

    // 2. Effective target: CLI target -> CARGO_BUILD_TARGET -> .cargo/config target -> "host"
    let effective_target = target.or(env_target).or(config_target.as_deref()).unwrap_or("host");
    effective_target.hash(&mut hasher);

    // 3. Package filter and normalized feature flags
    package.unwrap_or("workspace").hash(&mut hasher);
    super::feature_graph::normalize_feature_list(features).hash(&mut hasher);
    no_default_features.hash(&mut hasher);
    all_features.hash(&mut hasher);

    // 4. All workspace member manifests (Cargo.toml in workspace root and subdirectories)
    let mut all_manifests = Vec::new();
    collect_all_workspace_manifests(work_dir, &mut all_manifests);
    if !all_manifests.contains(&canon_manifest) {
        all_manifests.push(canon_manifest.clone());
    }
    all_manifests.sort();
    for m in all_manifests {
        m.strip_prefix(work_dir).unwrap_or(&m).display().to_string().hash(&mut hasher);
        if let Ok(bytes) = fs::read(&m) {
            bytes.hash(&mut hasher);
        }
    }

    // 5. Cargo.lock 64-bit FNV-1a digest (captures resolved dependency graph, versions, sources, active features)
    if let Some(lock_hash) = super::feature_graph::compute_cargo_lock_hash(work_dir) {
        lock_hash.hash(&mut hasher);
    }

    // 6. Toolchain version and host details via rustc -Vv
    if let Ok(out) = Command::new("rustc").arg("-Vv").output() {
        if out.status.success() {
            out.stdout.hash(&mut hasher);
        }
    } else if let Some(toolchain) = toolchain {
        toolchain.hash(&mut hasher);
    }
    if let Some(toolchain) = toolchain {
        toolchain.hash(&mut hasher);
    }

    // 7. Flags affecting rustc compilation
    if let Some(rustflags) = rustflags {
        rustflags.hash(&mut hasher);
    }
    if let Some(encoded_flags) = encoded_flags {
        encoded_flags.hash(&mut hasher);
    }

    // 8. RIDX binary format and extractor version
    ridx_format::CURRENT_FORMAT_VERSION.hash(&mut hasher);
    ridx_format::EXTRACTOR_VERSION.hash(&mut hasher);

    let hash = hasher.finish();
    format!("{profile}-{:016x}", hash)
}
