/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Manifest resolution, workspace auto-discovery, and bounded `.ridx` file collection.

use super::{DependencyShardInfo, VariantManifest, WorkspaceIndex};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub(super) type OpenManifestInfo = (PathBuf, Option<std::time::SystemTime>, String, Vec<DependencyShardInfo>, Option<PathBuf>, Option<String>);

impl WorkspaceIndex {
    pub(super) fn resolve_open_manifest(search_dir: &Path) -> io::Result<(PathBuf, Option<OpenManifestInfo>)> {
        let manifest_file = search_dir.join("ridx_manifest.json");
        let res = if manifest_file.is_file() {
            let content = fs::read(&manifest_file)?;
            let manifest = serde_json::from_slice::<VariantManifest>(&content)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("Failed to parse variant manifest {:?}: {e}", manifest_file)))?;

            let mtime = fs::metadata(&manifest_file).and_then(|m| m.modified()).ok();
            if !manifest.active_variant_id.is_empty() {
                let expected_variant_dir = format!("variants/{}", manifest.active_variant_id);
                let vdir_str = manifest.variant_dir.trim();
                let vpath = Path::new(vdir_str);
                if vdir_str.is_empty()
                    || vpath.is_absolute()
                    || vpath.components().any(|c| c == std::path::Component::ParentDir)
                    || vpath == Path::new(".")
                    || vdir_str != expected_variant_dir
                {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "Active variant manifest {:?} specifies variant_dir {:?} which does not match active_variant_id {:?} (expected {:?})",
                            manifest_file, manifest.variant_dir, manifest.active_variant_id, expected_variant_dir
                        ),
                    ));
                }

                let vdir = search_dir.join(&expected_variant_dir);
                if vdir == search_dir || !vdir.is_dir() {
                    return Err(io::Error::new(
                        io::ErrorKind::NotFound,
                        format!("Active variant directory {:?} specified in {:?} does not exist. Run 'ridx index' to build this variant.", vdir, manifest_file),
                    ));
                }

                if manifest.format_version != crate::format::CURRENT_FORMAT_VERSION {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "Format version mismatch in manifest {:?}: format_version {} != current {}",
                            manifest_file,
                            manifest.format_version,
                            crate::format::CURRENT_FORMAT_VERSION
                        ),
                    ));
                }

                match manifest.extractor_version.as_deref() {
                    Some(ver) if ver == crate::format::EXTRACTOR_VERSION => {}
                    Some(ver) => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("Extractor version mismatch in manifest {:?}: extractor_version {} != current {}", manifest_file, ver, crate::format::EXTRACTOR_VERSION),
                        ));
                    }
                    None => {
                        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Extractor version missing in manifest {:?}", manifest_file)));
                    }
                }

                (
                    vdir,
                    Some((
                        manifest_file,
                        mtime,
                        manifest.active_variant_id,
                        manifest.dependency_shards,
                        if !manifest.workspace_root.is_empty() { Some(PathBuf::from(manifest.workspace_root)) } else { None },
                        manifest.sysroot_cas_key,
                    )),
                )
            } else {
                // Legacy manifest without active_variant_id: scan search_dir directly
                (search_dir.to_path_buf(), None)
            }
        } else if search_dir.join("variant_manifest.json").is_file() {
            let v_manifest_file = search_dir.join("variant_manifest.json");
            let content = fs::read(&v_manifest_file)?;
            let manifest = serde_json::from_slice::<VariantManifest>(&content)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("Failed to parse variant manifest {:?}: {e}", v_manifest_file)))?;
            let mtime = fs::metadata(&v_manifest_file).and_then(|m| m.modified()).ok();
            if manifest.format_version != crate::format::CURRENT_FORMAT_VERSION {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "Format version mismatch in manifest {:?}: format_version {} != current {}",
                        v_manifest_file,
                        manifest.format_version,
                        crate::format::CURRENT_FORMAT_VERSION
                    ),
                ));
            }
            match manifest.extractor_version.as_deref() {
                Some(ver) if ver == crate::format::EXTRACTOR_VERSION => {}
                Some(ver) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("Extractor version mismatch in manifest {:?}: extractor_version {} != current {}", v_manifest_file, ver, crate::format::EXTRACTOR_VERSION),
                    ));
                }
                None => {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Extractor version missing in manifest {:?}", v_manifest_file)));
                }
            }
            (
                search_dir.to_path_buf(),
                Some((
                    v_manifest_file,
                    mtime,
                    manifest.active_variant_id,
                    manifest.dependency_shards,
                    if !manifest.workspace_root.is_empty() { Some(PathBuf::from(manifest.workspace_root)) } else { None },
                    manifest.sysroot_cas_key,
                )),
            )
        } else {
            (search_dir.to_path_buf(), None)
        };
        Ok(res)
    }

    /// Auto-discovers .ridx indices starting from current directory up to root or target/ridx.
    pub fn auto_discover() -> io::Result<Self> {
        if let Ok(dir) = std::env::var("RIDX_INDEX_DIR") {
            let p = PathBuf::from(dir);
            if p.exists() {
                return Self::open(p);
            }
        }

        let cur = std::env::current_dir()?;
        let mut candidates = vec![cur.join("target/ridx"), cur.join("../target/ridx")];
        let mut nearest_pkg = None;
        let mut enclosing_ws = None;
        for anc in cur.ancestors() {
            let cargo_toml = anc.join("Cargo.toml");
            if cargo_toml.is_file() {
                if nearest_pkg.is_none() {
                    nearest_pkg = Some(anc.to_path_buf());
                }
                if let Ok(content) = fs::read_to_string(&cargo_toml) {
                    if content.contains("[workspace]") {
                        enclosing_ws = Some(anc.to_path_buf());
                        break;
                    }
                }
            }
        }
        if let Some(ws_root) = enclosing_ws.or(nearest_pkg) {
            let ws_target = ws_root.join("target/ridx");
            if !candidates.contains(&ws_target) {
                candidates.push(ws_target);
            }
        }

        let has_ridx_indicators = cur.join("ridx_manifest.json").is_file()
            || cur.join("variant_manifest.json").is_file()
            || fs::read_dir(&cur).ok().map(|mut rd| rd.any(|e| e.map(|e| e.path().extension().is_some_and(|ext| ext == "ridx")).unwrap_or(false))).unwrap_or(false);
        if has_ridx_indicators {
            candidates.push(cur.clone());
        }

        for cand in candidates {
            if cand.is_dir() {
                if let Ok(ws) = Self::open(&cand) {
                    return Ok(ws);
                }
            }
        }

        Err(io::Error::new(io::ErrorKind::NotFound, "Could not auto-discover target/ridx directory. Specify index path explicitly."))
    }
}

pub(crate) fn collect_ridx_files(dir: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    collect_ridx_files_bounded(dir, files, 0, 2)
}

fn collect_ridx_files_bounded(dir: &Path, files: &mut Vec<PathBuf>, depth: usize, max_depth: usize) -> io::Result<()> {
    if depth > max_depth {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let p = entry.path();
        if p.is_file() {
            if p.extension().is_some_and(|ext| ext == "ridx") {
                files.push(p);
            }
        } else if p.is_dir() {
            if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                if name.starts_with('.') || name == "variants" || name == "target" || name == "node_modules" || name == "vendor" {
                    continue;
                }
            }
            let _ = collect_ridx_files_bounded(&p, files, depth + 1, max_depth);
        }
    }
    Ok(())
}
