/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Dependency and workspace shard discovery and cargo metadata target kind extraction.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Queries `cargo metadata --no-deps` and builds a mapping from canonical target source root path
/// to primary cargo target kind (e.g. "lib", "bin", "bench", "test", "example", "custom-build").
pub fn extract_target_kinds(manifest_path: &Path) -> HashMap<PathBuf, String> {
    let mut map = HashMap::new();
    let Ok(output) = Command::new("cargo")
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--manifest-path",
        ])
        .arg(manifest_path)
        .output()
    else {
        return map;
    };
    if !output.status.success() {
        return map;
    }
    let Ok(meta) = serde_json::from_slice::<serde_json::Value>(&output.stdout) else {
        return map;
    };
    if let Some(packages) = meta.get("packages").and_then(|p| p.as_array()) {
        for pkg in packages {
            if let Some(targets) = pkg.get("targets").and_then(|t| t.as_array()) {
                for target in targets {
                    let src_path = target.get("src_path").and_then(|s| s.as_str());
                    let kind = target
                        .get("kind")
                        .and_then(|k| k.as_array())
                        .and_then(|arr| arr.first())
                        .and_then(|k| k.as_str());
                    if let (Some(src), Some(k)) = (src_path, kind) {
                        let path = PathBuf::from(src);
                        let canon = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
                        map.insert(canon, k.to_string());
                        map.insert(path, k.to_string());
                    }
                }
            }
        }
    }
    map
}

/// Discovers and validates all dependency and workspace shards from the variant's `.deps/` directory.
/// Returns a tuple of `(dependency_shards, workspace_shards)`.
pub fn collect_shards_from_deps_dir(
    deps_dir: &Path,
) -> (
    Vec<ridx_format::DependencyShardInfo>,
    Vec<ridx_format::DependencyShardInfo>,
) {
    let mut dep_shards: Vec<ridx_format::DependencyShardInfo> = Vec::new();
    let mut ws_shards: Vec<ridx_format::DependencyShardInfo> = Vec::new();

    if !deps_dir.is_dir() {
        return (dep_shards, ws_shards);
    }

    if let Ok(entries) = fs::read_dir(deps_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let Ok(content) = fs::read(&path) else {
                continue;
            };
            let Ok(mut info) = serde_json::from_slice::<ridx_format::DependencyShardInfo>(&content)
            else {
                continue;
            };

            if info.is_dependency {
                let obj_p = Path::new(&info.object_path);
                if obj_p.is_file() {
                    if let Ok(r) = ridx_format::IndexReader::open(obj_p) {
                        let bin_gen = r.generation();
                        if !bin_gen.is_empty() {
                            let sidecar_gen = info.generation.as_deref().unwrap_or("");
                            let gen_matches = if !sidecar_gen.is_empty() {
                                sidecar_gen == bin_gen
                            } else if !info.file_hashes.is_empty() {
                                let computed_digest =
                                    ridx_format::compute_source_snapshot_digest(&info.file_hashes);
                                format!("{:016x}", computed_digest) == bin_gen
                            } else {
                                false
                            };
                            if !gen_matches {
                                info.file_hashes.clear();
                                info.dependencies.clear();
                                info.extern_units.clear();
                            }
                            info.generation = Some(bin_gen.to_string());
                        }
                        let embedded = r.embedded_file_hashes();
                        let sroot_path = info.source_root.as_deref().map(Path::new);
                        ridx_format::merge_embedded_hashes_authoritative(
                            &mut info.file_hashes,
                            &embedded,
                            sroot_path,
                        );
                    }
                }
                if !dep_shards.iter().any(|d| d.unit_key == info.unit_key) {
                    dep_shards.push(info);
                }
            } else if !ws_shards.iter().any(|w| w.unit_key == info.unit_key) {
                ws_shards.push(info);
            }
        }
    }

    dep_shards.sort_by(|a, b| a.name.cmp(&b.name));
    ws_shards.sort_by(|a, b| a.name.cmp(&b.name));

    (dep_shards, ws_shards)
}
