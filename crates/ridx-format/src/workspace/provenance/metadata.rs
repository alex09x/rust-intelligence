/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Sidecar JSON metadata loading for `WorkspaceIndex` shards.

use super::super::manifest::DependencyShardInfo;
use super::super::target_kind::TargetKind;
use super::super::WorkspaceIndex;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

impl WorkspaceIndex {
    pub(crate) fn load_file_metadata(
        file_path: &Path,
        base_dir: &Path,
        crate_name: &str,
        expected_unit_key: &str,
        expected_generation: &str,
        embedded_hashes: &HashMap<String, u64>,
    ) -> LoadedFileMetadata {
        let stem = file_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let stem_without_lib = stem.strip_prefix("lib").unwrap_or(stem);
        let deps_dir = base_dir.join(".deps");
        let cand_json = [deps_dir.join(format!("{stem_without_lib}.json")), deps_dir.join(format!("{stem}.json")), deps_dir.join(format!("{crate_name}.json"))];
        for c in &cand_json {
            if c.is_file() {
                if let Ok(content) = fs::read(c) {
                    if let Ok(info) = serde_json::from_slice::<DependencyShardInfo>(&content) {
                        if !expected_unit_key.is_empty() && info.unit_key != expected_unit_key {
                            continue;
                        }
                        if !expected_generation.is_empty() {
                            let sidecar_gen = info.generation.as_deref().unwrap_or("");
                            if !sidecar_gen.is_empty() {
                                if sidecar_gen != expected_generation {
                                    continue;
                                }
                            } else if !info.file_hashes.is_empty() {
                                let computed_digest = crate::format::compute_source_snapshot_digest(&info.file_hashes);
                                let computed_gen = format!("{:016x}", computed_digest);
                                if computed_gen != expected_generation {
                                    continue;
                                }
                            }
                        }
                        let mtime = fs::metadata(c).and_then(|m| m.modified()).ok();
                        let mut combined_hashes = info.file_hashes;
                        let sroot_path = info.source_root.as_deref().map(Path::new);
                        crate::format::merge_embedded_hashes_authoritative(&mut combined_hashes, embedded_hashes, sroot_path);
                        let gen = if !expected_generation.is_empty() { Some(expected_generation.to_string()) } else { info.generation };
                        let effective_source_root = info.source_root.as_ref().map(PathBuf::from).or_else(|| info.env.get("CARGO_MANIFEST_DIR").map(PathBuf::from));

                        let effective_target_kind = info.target_kind.or_else(|| {
                            let base_kind = if info.env.contains_key("CARGO_BIN_NAME") { Some("bin") } else { None };
                            if base_kind.is_some() || !info.compiler_args.is_empty() {
                                let manifest_dir = effective_source_root.as_deref();
                                Some(super::target_kind::classify_target_with_manifest(base_kind, &info.compiler_args, manifest_dir))
                            } else {
                                None
                            }
                        });

                        return LoadedFileMetadata {
                            source_root: effective_source_root,
                            unit_key: (!info.unit_key.is_empty()).then_some(info.unit_key),
                            target_kind: effective_target_kind,
                            is_dependency: info.is_dependency,
                            generation: gen,
                            dependencies: info.dependencies,
                            extern_units: info.extern_units,
                            file_hashes: combined_hashes,
                            meta_path: Some(c.clone()),
                            meta_mtime: mtime,
                        };
                    }
                }
            }
        }
        let gen = if !expected_generation.is_empty() { Some(expected_generation.to_string()) } else { None };
        let ukey = if !expected_unit_key.is_empty() { Some(expected_unit_key.to_string()) } else { None };
        LoadedFileMetadata {
            source_root: None,
            unit_key: ukey,
            target_kind: None,
            is_dependency: false,
            generation: gen,
            dependencies: Vec::new(),
            extern_units: HashMap::new(),
            file_hashes: embedded_hashes.clone(),
            meta_path: Some(cand_json[0].clone()),
            meta_mtime: None,
        }
    }
}

/// Strongly-typed file metadata resolved from sidecar JSON files.
#[derive(Debug, Clone)]
pub(crate) struct LoadedFileMetadata {
    pub source_root: Option<PathBuf>,
    pub unit_key: Option<String>,
    pub target_kind: Option<TargetKind>,
    pub is_dependency: bool,
    pub generation: Option<String>,
    pub dependencies: Vec<String>,
    pub extern_units: HashMap<String, String>,
    pub file_hashes: HashMap<String, u64>,
    pub meta_path: Option<PathBuf>,
    pub meta_mtime: Option<std::time::SystemTime>,
}
