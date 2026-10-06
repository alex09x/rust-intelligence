/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Workspace discovery, path resolution, and index opening helpers.

use crate::indexer::{run_indexing, IndexOptions};
use colored::*;
use ridx_format::WorkspaceIndex;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn parse_location(loc: &str) -> Option<(&str, u32, u32)> {
    let parts: Vec<&str> = loc.rsplitn(3, ':').collect();
    if parts.len() == 3 {
        let col: u32 = parts[0].parse().ok()?;
        let line: u32 = parts[1].parse().ok()?;
        let file = parts[2];
        Some((file, line, col))
    } else {
        None
    }
}

pub(crate) fn find_enclosing_workspace(start: &Path) -> Option<PathBuf> {
    let mut cur = if start.is_dir() {
        Some(start)
    } else {
        start.parent()
    };
    while let Some(dir) = cur {
        if dir.join("Cargo.toml").is_file() {
            return Some(dir.to_path_buf());
        }
        cur = dir.parent();
    }
    None
}

pub(crate) fn find_manifest_for_path(p: &Path) -> Option<(PathBuf, ridx_format::VariantManifest)> {
    let mut candidate_dirs = Vec::new();
    if p.is_dir() {
        candidate_dirs.push(p.to_path_buf());
        candidate_dirs.push(p.join("target/ridx"));
        let mut cur = p.parent();
        while let Some(parent) = cur {
            candidate_dirs.push(parent.to_path_buf());
            cur = parent.parent();
        }
    } else if let Some(parent) = p.parent() {
        candidate_dirs.push(parent.to_path_buf());
        let mut cur = parent.parent();
        while let Some(grandparent) = cur {
            candidate_dirs.push(grandparent.to_path_buf());
            cur = grandparent.parent();
        }
    }

    // 1. Check if a shard-specific variant_manifest.json exists in candidate dirs (innermost first)
    for dir in &candidate_dirs {
        let v_manifest_file = dir.join("variant_manifest.json");
        if v_manifest_file.is_file() {
            if let Ok(content) = fs::read(&v_manifest_file) {
                if let Ok(manifest) =
                    serde_json::from_slice::<ridx_format::VariantManifest>(&content)
                {
                    return Some((dir.clone(), manifest));
                }
            }
        }
    }

    // 2. Check root ridx_manifest.json
    for dir in candidate_dirs {
        let manifest_file = dir.join("ridx_manifest.json");
        if manifest_file.is_file() {
            if let Ok(content) = fs::read(&manifest_file) {
                if let Ok(manifest) =
                    serde_json::from_slice::<ridx_format::VariantManifest>(&content)
                {
                    // If p specifies a variant directory or shard under variants/<id>, verify that
                    // it matches manifest.active_variant_id. If p targets an inactive variant shard
                    // whose recipe is not the root active manifest, return None rather than substituting
                    // an unrelated variant's recipe.
                    let p_str = p.to_string_lossy();
                    if let Some(pos) = p_str.find("variants/") {
                        let sub = &p_str[pos + "variants/".len()..];
                        let var_id = sub.split('/').next().unwrap_or("");
                        if !var_id.is_empty()
                            && !manifest.active_variant_id.is_empty()
                            && var_id != manifest.active_variant_id
                        {
                            return None;
                        }
                    }
                    return Some((dir, manifest));
                }
            }
        }
    }
    None
}

pub(crate) fn try_resolve_workspace(path_opt: Option<PathBuf>) -> Result<WorkspaceIndex, String> {
    match path_opt {
        Some(p) => match WorkspaceIndex::open(&p) {
            Ok(ws) => Ok(ws),
            Err(e) => {
                if ridx_format::is_version_mismatch(&e) || e.kind() == std::io::ErrorKind::NotFound
                {
                    eprintln!(
                        "{} Index at {:?} is missing or incompatible with format v{}. Automatically reindexing...",
                        "Notice:".yellow().bold(), p, ridx_format::CURRENT_FORMAT_VERSION
                    );
                    let (
                        work_dir,
                        out_dir_opt,
                        package,
                        release,
                        features,
                        no_default_features,
                        all_features,
                        target,
                    ) = if let Some((manifest_dir, manifest)) = find_manifest_for_path(&p) {
                        let ws_dir = if !manifest.workspace_root.is_empty()
                            && Path::new(&manifest.workspace_root).exists()
                        {
                            PathBuf::from(manifest.workspace_root)
                        } else if let Some(ws) = find_enclosing_workspace(&p) {
                            ws
                        } else {
                            return Err(format!(
                                "Cannot find Cargo workspace for index at {:?}.",
                                p
                            ));
                        };
                        let base_dir = if let Some(ref b) = manifest.base_out_dir {
                            PathBuf::from(b)
                        } else if manifest_dir
                            .parent()
                            .and_then(|pr| pr.file_name())
                            .and_then(|s| s.to_str())
                            == Some("variants")
                        {
                            manifest_dir
                                .parent()
                                .unwrap()
                                .parent()
                                .unwrap()
                                .to_path_buf()
                        } else if manifest_dir.file_name().and_then(|s| s.to_str())
                            == Some("variants")
                        {
                            manifest_dir.parent().unwrap().to_path_buf()
                        } else {
                            manifest_dir
                        };
                        (
                            ws_dir,
                            Some(base_dir),
                            manifest.package_filter,
                            manifest.profile == "release",
                            manifest.features,
                            manifest.no_default_features,
                            manifest.all_features,
                            manifest.target,
                        )
                    } else {
                        let p_str = p.to_string_lossy();
                        if p_str.contains("variants/") {
                            return Err(format!(
                                "Cannot automatically reindex inactive variant shard at {:?}: no matching variant manifest found.\nRun 'ridx index' with the appropriate build configuration for this variant.",
                                p
                            ));
                        }
                        if let Some(ws) = find_enclosing_workspace(&p) {
                            (ws, None, None, false, None, false, false, None)
                        } else {
                            return Err(format!(
                                "Cannot determine Cargo workspace for {:?}. No Cargo.toml found in path or parent directories.\nSpecify the path to a Cargo workspace or run 'ridx index' first.",
                                p
                            ));
                        }
                    };

                    let published = run_indexing(IndexOptions {
                        path: work_dir,
                        out_dir_opt,
                        package,
                        release,
                        features,
                        no_default_features,
                        all_features,
                        target,
                    });

                    // Reopen the rebuilt variant or shard rather than failing on an obsolete artifact path:
                    let mut cand_paths = Vec::new();
                    if p.extension().and_then(|s| s.to_str()) == Some("ridx") {
                        if let Some(file_name) = p.file_name() {
                            cand_paths.push(published.variant_out_dir.join(file_name));
                        }
                    }
                    cand_paths.push(published.variant_out_dir.clone());
                    cand_paths.push(published.base_out_dir.clone());
                    cand_paths.push(p.clone());

                    for cand in cand_paths {
                        if let Ok(ws) = WorkspaceIndex::open(&cand) {
                            return Ok(ws);
                        }
                    }

                    return Err(format!(
                        "Failed to open reindexed workspace after variant publish {:?}.",
                        published.variant_out_dir
                    ));
                }
                Err(format!("Failed to open index {:?}: {e}", p))
            }
        },
        None => match WorkspaceIndex::auto_discover() {
            Ok(ws) => Ok(ws),
            Err(e) => {
                let cur = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
                let (
                    work_dir,
                    out_dir_opt,
                    package,
                    release,
                    features,
                    no_default_features,
                    all_features,
                    target,
                ) = if let Some((manifest_dir, manifest)) = find_manifest_for_path(&cur) {
                    let ws_dir = if !manifest.workspace_root.is_empty()
                        && Path::new(&manifest.workspace_root).exists()
                    {
                        PathBuf::from(manifest.workspace_root)
                    } else if let Some(ws) = find_enclosing_workspace(&cur) {
                        ws
                    } else {
                        return Err(format!(
                            "Cannot find Cargo workspace for manifest at {:?}.",
                            manifest_dir
                        ));
                    };
                    (
                        ws_dir,
                        Some(manifest_dir),
                        manifest.package_filter,
                        manifest.profile == "release",
                        manifest.features,
                        manifest.no_default_features,
                        manifest.all_features,
                        manifest.target,
                    )
                } else if let Some(ws) = find_enclosing_workspace(&cur) {
                    (ws, None, None, false, None, false, false, None)
                } else {
                    return Err(format!(
                        "{e}\nSpecify the path to a .ridx file or directory, or run from a workspace with target/ridx/."
                    ));
                };

                eprintln!(
                    "{} No valid RIDX index found for current workspace (expected format v{}). Automatically indexing...",
                    "Notice:".yellow().bold(), ridx_format::CURRENT_FORMAT_VERSION
                );
                run_indexing(IndexOptions {
                    path: work_dir,
                    out_dir_opt,
                    package,
                    release,
                    features,
                    no_default_features,
                    all_features,
                    target,
                });
                match WorkspaceIndex::auto_discover() {
                    Ok(ws) => Ok(ws),
                    Err(e2) => Err(format!("Failed to auto-discover reindexed workspace: {e2}")),
                }
            }
        },
    }
}

pub(crate) fn resolve_workspace(path_opt: Option<PathBuf>) -> WorkspaceIndex {
    match try_resolve_workspace(path_opt) {
        Ok(ws) => ws,
        Err(e) => {
            eprintln!("{}: {e}", "Error".red().bold());
            #[cfg(not(test))]
            std::process::exit(1);
            #[cfg(test)]
            panic!("{e}");
        }
    }
}

pub(crate) fn try_resolve_index_and_target(
    arg1: String,
    arg2: Option<String>,
) -> Result<(WorkspaceIndex, String), String> {
    match arg2 {
        Some(target) => Ok((try_resolve_workspace(Some(PathBuf::from(arg1)))?, target)),
        None => Ok((try_resolve_workspace(None)?, arg1)),
    }
}

pub(crate) fn resolve_index_and_target(
    arg1: String,
    arg2: Option<String>,
) -> (WorkspaceIndex, String) {
    try_resolve_index_and_target(arg1, arg2).unwrap_or_else(|e| {
        eprintln!("{}: {e}", "Error".red().bold());
        #[cfg(not(test))]
        std::process::exit(1);
        #[cfg(test)]
        panic!("{e}");
    })
}
