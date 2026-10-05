//! Source provenance, hash resolution, and shard metadata loading.

use super::manifest::DependencyShardInfo;
use super::WorkspaceIndex;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

impl WorkspaceIndex {
    /// Returns the file modification timestamp of the index shard containing the given source file.
    pub fn shard_mtime_for_file(&self, file_path: &str) -> Option<std::time::SystemTime> {
        let p = Path::new(file_path);
        let is_abs = p.is_absolute();

        for (i, reader) in self.readers.iter().enumerate() {
            let target_file = if let Some(Some(root)) = self.source_roots.get(i) {
                if is_abs {
                    if let Ok(rel) = p.strip_prefix(root) {
                        rel.to_str().unwrap_or(file_path)
                    } else {
                        continue;
                    }
                } else {
                    file_path
                }
            } else {
                if is_abs {
                    let belongs_to_dep = self.source_roots.iter().any(|r| {
                        if let Some(r) = r.as_ref() {
                            p.starts_with(r)
                        } else {
                            false
                        }
                    });
                    if belongs_to_dep {
                        continue;
                    }
                }
                file_path
            };

            if reader.find_file_id(target_file).is_some() {
                return self.mtimes.get(i).copied().flatten();
            }
        }
        None
    }

    /// Returns the compiled source hash (from rustc compilation provenance) for a file if available.
    pub fn source_hash_for_file(&self, file_path: &str) -> Option<u64> {
        let p = Path::new(file_path);
        let is_abs = p.is_absolute();

        // 1. If absolute path, check if any dependency shard owns this file (specific source_root)
        if is_abs {
            for (i, reader) in self.readers.iter().enumerate() {
                if let Some(Some(root)) = self.source_roots.get(i) {
                    let matched_rel = p
                        .strip_prefix(root)
                        .ok()
                        .map(|r| r.to_string_lossy().to_string())
                        .or_else(|| {
                            if let (Ok(canon_p), Ok(canon_root)) =
                                (p.canonicalize(), root.canonicalize())
                            {
                                canon_p
                                    .strip_prefix(canon_root)
                                    .ok()
                                    .map(|r| r.to_string_lossy().to_string())
                            } else {
                                None
                            }
                        });

                    if let Some(rel_str) = matched_rel {
                        // Check owning reader's embedded FileEntry table (highest priority)
                        if let Some(fid) = reader.find_file_id(&rel_str) {
                            let h = reader.file_source_hash(fid);
                            if h != 0 {
                                return Some(h);
                            }
                        }
                        if let Some(fid) = reader.find_file_id(file_path) {
                            let h = reader.file_source_hash(fid);
                            if h != 0 {
                                return Some(h);
                            }
                        }
                        if let Some(map) = self.reader_file_hashes.get(i) {
                            if let Some(&h) = map.get(&rel_str) {
                                return Some(h);
                            }
                            if let Some(&h) = map.get(file_path) {
                                return Some(h);
                            }
                        }
                        return None;
                    }
                }
            }

            // 2. Absolute path in workspace root (not belonging to any dependency)
            let belongs_to_any_dep = self.source_roots.iter().any(|r| {
                if let Some(r) = r.as_ref() {
                    p.starts_with(r)
                } else {
                    false
                }
            });

            if !belongs_to_any_dep {
                if let Some(ref ws_root) = self.workspace_root {
                    let matched_rel = p
                        .strip_prefix(ws_root)
                        .ok()
                        .map(|r| r.to_string_lossy().to_string())
                        .or_else(|| {
                            if let (Ok(canon_p), Ok(canon_root)) =
                                (p.canonicalize(), ws_root.canonicalize())
                            {
                                canon_p
                                    .strip_prefix(canon_root)
                                    .ok()
                                    .map(|r| r.to_string_lossy().to_string())
                            } else {
                                None
                            }
                        });

                    if let Some(rel_str) = matched_rel {
                        for (i, reader) in self.readers.iter().enumerate() {
                            if self.source_roots.get(i).and_then(|r| r.as_ref()).is_none() {
                                if let Some(fid) = reader.find_file_id(&rel_str) {
                                    let h = reader.file_source_hash(fid);
                                    if h != 0 {
                                        return Some(h);
                                    }
                                }
                                if let Some(fid) = reader.find_file_id(file_path) {
                                    let h = reader.file_source_hash(fid);
                                    if h != 0 {
                                        return Some(h);
                                    }
                                }
                                if let Some(map) = self.reader_file_hashes.get(i) {
                                    if let Some(&h) = map.get(&rel_str) {
                                        return Some(h);
                                    }
                                    if let Some(&h) = map.get(file_path) {
                                        return Some(h);
                                    }
                                }
                            }
                        }
                    }
                }

                // Fallback exact or canonical match across reader_file_hashes for workspace crates
                for map in &self.reader_file_hashes {
                    if let Some(&h) = map.get(file_path) {
                        return Some(h);
                    }
                }
                if let Ok(canon) = p.canonicalize() {
                    let canon_str = canon.to_string_lossy();
                    for map in &self.reader_file_hashes {
                        if let Some(&h) = map.get(canon_str.as_ref()) {
                            return Some(h);
                        }
                    }
                }
            }
        } else {
            // 3. Relative path query
            // First check workspace crates (non-dependency)
            for (i, reader) in self.readers.iter().enumerate() {
                if !self.reader_is_dependency.get(i).copied().unwrap_or(false) {
                    if let Some(fid) = reader.find_file_id(file_path) {
                        let h = reader.file_source_hash(fid);
                        if h != 0 {
                            return Some(h);
                        }
                    }
                    if let Some(map) = self.reader_file_hashes.get(i) {
                        if let Some(&h) = map.get(file_path) {
                            return Some(h);
                        }
                    }
                }
            }

            // Then check dependency crates
            for (i, reader) in self.readers.iter().enumerate() {
                if self.reader_is_dependency.get(i).copied().unwrap_or(false) {
                    if let Some(fid) = reader.find_file_id(file_path) {
                        let h = reader.file_source_hash(fid);
                        if h != 0 {
                            return Some(h);
                        }
                    }
                    if let Some(map) = self.reader_file_hashes.get(i) {
                        if let Some(&h) = map.get(file_path) {
                            return Some(h);
                        }
                    }
                }
            }
        }

        None
    }

    #[allow(clippy::type_complexity)]
    pub(crate) fn load_file_metadata(
        file_path: &Path,
        base_dir: &Path,
        crate_name: &str,
        expected_unit_key: &str,
        expected_generation: &str,
        embedded_hashes: &HashMap<String, u64>,
    ) -> (
        Option<PathBuf>,
        Option<String>,
        Option<String>,
        Vec<String>,
        HashMap<String, String>,
        HashMap<String, u64>,
        Option<PathBuf>,
        Option<std::time::SystemTime>,
    ) {
        let stem = file_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let stem_without_lib = stem.strip_prefix("lib").unwrap_or(stem);
        let deps_dir = base_dir.join(".deps");
        let cand_json = [
            deps_dir.join(format!("{stem_without_lib}.json")),
            deps_dir.join(format!("{stem}.json")),
            deps_dir.join(format!("{crate_name}.json")),
        ];
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
                                let computed_digest = crate::format::compute_source_snapshot_digest(
                                    &info.file_hashes,
                                );
                                let computed_gen = format!("{:016x}", computed_digest);
                                if computed_gen != expected_generation {
                                    continue;
                                }
                            }
                        }
                        let mtime = fs::metadata(c).and_then(|m| m.modified()).ok();
                        let mut combined_hashes = info.file_hashes;
                        let sroot_path = info.source_root.as_deref().map(Path::new);
                        crate::format::merge_embedded_hashes_authoritative(
                            &mut combined_hashes,
                            embedded_hashes,
                            sroot_path,
                        );
                        let gen = if !expected_generation.is_empty() {
                            Some(expected_generation.to_string())
                        } else {
                            info.generation
                        };
                        return (
                            info.source_root.as_ref().map(PathBuf::from),
                            Some(info.unit_key),
                            gen,
                            info.dependencies,
                            info.extern_units,
                            combined_hashes,
                            Some(c.clone()),
                            mtime,
                        );
                    }
                }
            }
        }
        let gen = if !expected_generation.is_empty() {
            Some(expected_generation.to_string())
        } else {
            None
        };
        let ukey = if !expected_unit_key.is_empty() {
            Some(expected_unit_key.to_string())
        } else {
            None
        };
        (
            None,
            ukey,
            gen,
            Vec::new(),
            HashMap::new(),
            embedded_hashes.clone(),
            Some(cand_json[0].clone()),
            None,
        )
    }
}
