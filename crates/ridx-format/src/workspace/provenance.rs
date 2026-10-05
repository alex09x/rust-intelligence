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

        // 1. Exact match across all readers
        for map in &self.reader_file_hashes {
            if let Some(&h) = map.get(file_path) {
                return Some(h);
            }
        }

        // 2. Canonicalized match across all readers
        if let Ok(canon) = p.canonicalize() {
            let canon_str = canon.to_string_lossy();
            for map in &self.reader_file_hashes {
                if let Some(&h) = map.get(canon_str.as_ref()) {
                    return Some(h);
                }
            }
        }

        // 3. Owner-resolved match: identify which reader owns this file and match against its relative path
        for (i, reader) in self.readers.iter().enumerate() {
            let owner_root = self
                .source_roots
                .get(i)
                .and_then(|r| r.as_ref())
                .or(self.workspace_root.as_ref());
            if let Some(root) = owner_root {
                if let Ok(rel) = p.strip_prefix(root) {
                    let rel_str = rel.to_string_lossy();
                    if let Some(map) = self.reader_file_hashes.get(i) {
                        if let Some(&h) = map.get(rel_str.as_ref()) {
                            return Some(h);
                        }
                    }
                }
            }

            // Check if the reader indexed this file directly
            let is_abs = p.is_absolute();
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
                if let Some(map) = self.reader_file_hashes.get(i) {
                    if let Some(&h) = map.get(target_file) {
                        return Some(h);
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
                        let mut combined_hashes = embedded_hashes.clone();
                        for (k, v) in info.file_hashes {
                            combined_hashes.entry(k).or_insert(v);
                        }
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
