/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Build configuration verification and untracked file detection for direct fast-path indexing.

use super::WorkspaceCrateEntry;
use crate::indexer::types::IndexOptions;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Checks if any build configuration files or build options were modified since the last index.
/// Returns Some(manifest_mtime) if configuration is unchanged, or None to fall back to Cargo.
pub fn verify_build_config_unchanged(
    work_dir: &Path,
    base_out_dir: &Path,
    opts: &IndexOptions,
    workspace_crates: &[WorkspaceCrateEntry],
) -> Option<SystemTime> {
    let manifest_file = base_out_dir.join("ridx_manifest.json");
    let manifest_mtime = fs::metadata(&manifest_file)
        .ok()
        .and_then(|m| m.modified().ok())?;

    // Check build options match previously published manifest
    if let Ok(manifest_bytes) = fs::read(&manifest_file) {
        if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&manifest_bytes) {
            let p_feat = val.get("features").and_then(|v| v.as_str());
            let p_nodef = val
                .get("no_default_features")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let p_allfeat = val
                .get("all_features")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let p_target = val.get("target").and_then(|v| v.as_str());
            let p_pkg = val.get("package_filter").and_then(|v| v.as_str());

            if opts.features.as_deref() != p_feat
                || opts.no_default_features != p_nodef
                || opts.all_features != p_allfeat
                || opts.target.as_deref() != p_target
                || opts.package.as_deref() != p_pkg
            {
                return None;
            }
        } else {
            return None;
        }
    } else {
        return None;
    }

    // Check workspace-level build configuration files
    let config_files = [
        work_dir.join("Cargo.toml"),
        work_dir.join("Cargo.lock"),
        work_dir.join("rust-toolchain"),
        work_dir.join("rust-toolchain.toml"),
        work_dir.join(".cargo/config"),
        work_dir.join(".cargo/config.toml"),
    ];
    for cf in &config_files {
        if cf.is_file() {
            if let Ok(meta) = fs::metadata(cf) {
                if let Ok(mtime) = meta.modified() {
                    if mtime > manifest_mtime {
                        return None;
                    }
                }
            }
        }
    }

    // Check crate-level configuration files (Cargo.toml, build.rs)
    for (_, _, env_vars, _, _) in workspace_crates {
        if let Some(mdir_str) = env_vars.get("CARGO_MANIFEST_DIR") {
            let mdir = Path::new(mdir_str);
            let crate_toml = mdir.join("Cargo.toml");
            let crate_build = mdir.join("build.rs");
            for cf in &[crate_toml, crate_build] {
                if cf.is_file() {
                    if let Ok(meta) = fs::metadata(cf) {
                        if let Ok(mtime) = meta.modified() {
                            if mtime > manifest_mtime {
                                return None;
                            }
                        }
                    }
                }
            }
        }
    }

    Some(manifest_mtime)
}

/// Recursively checks if any `.rs` file in `src_dir` is not in `tracked`.
/// Returns true if an untracked `.rs` file was found (triggering fallback to Cargo).
pub fn has_untracked_rs_files(src_dir: &Path, tracked: &HashSet<String>) -> bool {
    if !src_dir.is_dir() {
        return false;
    }
    let mut stack = vec![src_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().and_then(|s| s.to_str()) == Some("rs") {
                    let p_str = p.display().to_string();
                    let canon_str = p
                        .canonicalize()
                        .map(|cp| cp.display().to_string())
                        .unwrap_or_else(|_| p_str.clone());
                    if !tracked.contains(&p_str) && !tracked.contains(&canon_str) {
                        return true;
                    }
                }
            }
        }
    }
    false
}
