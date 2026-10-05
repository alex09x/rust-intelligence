//! Source provenance, hash resolution, and shard metadata loading.

use super::manifest::DependencyShardInfo;
use super::WorkspaceIndex;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Normalizes a path by removing redundant `.` and resolving `..` components lexically.
fn normalize_path(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut components = Vec::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if let Some(Component::Normal(_)) = components.last() {
                    components.pop();
                } else if !components
                    .iter()
                    .any(|comp| matches!(comp, Component::RootDir | Component::Prefix(_)))
                {
                    components.push(c);
                }
            }
            _ => components.push(c),
        }
    }
    components.into_iter().collect()
}

/// Matches an entry path from an index shard or sidecar map against a requested file path,
/// resolving the entry under the reader's explicit path origin and checking exact normalized
/// and canonical path identity rather than ambiguous suffix matches.
fn file_entry_matches(
    entry_str: &str,
    reader_origin: Option<&Path>,
    req_path: &Path,
    req_canon: Option<&Path>,
) -> bool {
    if entry_str.is_empty() {
        return false;
    }
    // 1. Literal string match
    if entry_str == req_path.to_str().unwrap_or("") {
        return true;
    }

    let entry_p = Path::new(entry_str);

    // 2. Compute normalized entry path under reader's explicit path origin
    let norm_entry = if entry_p.is_absolute() {
        normalize_path(entry_p)
    } else if let Some(orig) = reader_origin {
        normalize_path(&orig.join(entry_p))
    } else {
        normalize_path(entry_p)
    };

    if req_path.is_absolute() {
        let norm_req = normalize_path(req_path);
        if norm_entry == norm_req {
            return true;
        }

        // Canonical comparison when absolute paths exist on disk
        if let Some(c_req) = req_canon {
            if let Ok(c_entry) = norm_entry.canonicalize() {
                if c_entry == *c_req {
                    return true;
                }
            }
        }
    } else {
        // Requested path is relative (e.g. "src/lib.rs")
        let norm_req = normalize_path(req_path);
        if norm_entry.is_relative() {
            if norm_entry == norm_req {
                return true;
            }
        } else if let Some(orig) = reader_origin {
            let norm_orig = normalize_path(orig);
            if let Ok(rel) = norm_entry.strip_prefix(&norm_orig) {
                if rel == norm_req || normalize_path(rel) == norm_req {
                    return true;
                }
            }
            if let (Ok(c_entry), Ok(c_orig)) = (norm_entry.canonicalize(), orig.canonicalize()) {
                if let Ok(rel) = c_entry.strip_prefix(&c_orig) {
                    if rel == norm_req || normalize_path(rel) == norm_req {
                        return true;
                    }
                }
            }
        }
    }

    false
}

impl WorkspaceIndex {
    /// Returns the file modification timestamp of the index shard containing the given source file.
    pub fn shard_mtime_for_file(&self, file_path: &str) -> Option<std::time::SystemTime> {
        let p = Path::new(file_path);
        let req_canon = p.canonicalize().ok();
        let ws_abs = if p.is_relative() {
            self.workspace_root
                .as_ref()
                .map(|ws_root| normalize_path(&ws_root.join(p)))
                .or_else(|| {
                    self.root_dir
                        .as_ref()
                        .map(|rdir| normalize_path(&rdir.join(p)))
                })
        } else {
            None
        };
        let ws_canon = ws_abs.as_ref().and_then(|a| a.canonicalize().ok());

        let mut candidate_indices: Vec<usize> = Vec::with_capacity(self.readers.len());
        if p.is_absolute() {
            let norm_p = normalize_path(p);
            for (i, _) in self.readers.iter().enumerate() {
                if let Some(Some(ref root)) = self.source_roots.get(i) {
                    let norm_root = normalize_path(root);
                    if norm_p.starts_with(&norm_root) {
                        candidate_indices.push(i);
                    }
                }
            }
        }
        for (i, _) in self.readers.iter().enumerate() {
            let is_dep = self.reader_is_dependency.get(i).copied().unwrap_or(false)
                || self.source_roots.get(i).and_then(|r| r.as_ref()).is_some();
            if !is_dep && !candidate_indices.contains(&i) {
                candidate_indices.push(i);
            }
        }
        for (i, _) in self.readers.iter().enumerate() {
            if !candidate_indices.contains(&i) {
                candidate_indices.push(i);
            }
        }

        // Phase 1: Search authoritative embedded FileEntry table across candidate readers
        for &i in &candidate_indices {
            let reader = &self.readers[i];
            let origin = self
                .source_roots
                .get(i)
                .and_then(|r| r.as_deref())
                .or(self.workspace_root.as_deref())
                .or(self.root_dir.as_deref());

            for fid in 0..reader.files_count() {
                let entry_str = reader.get_file(fid);
                if entry_str.is_empty() {
                    continue;
                }
                if file_entry_matches(entry_str, origin, p, req_canon.as_deref()) {
                    return self.mtimes.get(i).copied().flatten();
                }
                if let Some(ref abs_req) = ws_abs {
                    if file_entry_matches(entry_str, origin, abs_req, ws_canon.as_deref()) {
                        return self.mtimes.get(i).copied().flatten();
                    }
                }
            }
        }

        // Phase 2: Fallback to sidecar metadata maps across candidate readers
        for &i in &candidate_indices {
            let origin = self
                .source_roots
                .get(i)
                .and_then(|r| r.as_deref())
                .or(self.workspace_root.as_deref())
                .or(self.root_dir.as_deref());

            if let Some(map) = self.reader_file_hashes.get(i) {
                for k in map.keys() {
                    if file_entry_matches(k, origin, p, req_canon.as_deref()) {
                        return self.mtimes.get(i).copied().flatten();
                    }
                    if let Some(ref abs_req) = ws_abs {
                        if file_entry_matches(k, origin, abs_req, ws_canon.as_deref()) {
                            return self.mtimes.get(i).copied().flatten();
                        }
                    }
                }
            }
        }

        None
    }

    /// Returns the compiled source hash (from rustc compilation provenance) for a file if available.
    pub fn source_hash_for_file(&self, file_path: &str) -> Option<u64> {
        let p = Path::new(file_path);
        let req_canon = p.canonicalize().ok();
        let ws_abs = if p.is_relative() {
            self.workspace_root
                .as_ref()
                .map(|ws_root| normalize_path(&ws_root.join(p)))
                .or_else(|| {
                    self.root_dir
                        .as_ref()
                        .map(|rdir| normalize_path(&rdir.join(p)))
                })
        } else {
            None
        };
        let ws_canon = ws_abs.as_ref().and_then(|a| a.canonicalize().ok());

        // Order candidate readers:
        // 1. Dependency readers whose source_root contains p
        // 2. Workspace member readers (non-dependency)
        // 3. Other dependency readers
        let mut candidate_indices: Vec<usize> = Vec::with_capacity(self.readers.len());
        if p.is_absolute() {
            let norm_p = normalize_path(p);
            for (i, _) in self.readers.iter().enumerate() {
                if let Some(Some(ref root)) = self.source_roots.get(i) {
                    let norm_root = normalize_path(root);
                    if norm_p.starts_with(&norm_root) {
                        candidate_indices.push(i);
                    }
                }
            }
        }
        for (i, _) in self.readers.iter().enumerate() {
            let is_dep = self.reader_is_dependency.get(i).copied().unwrap_or(false)
                || self.source_roots.get(i).and_then(|r| r.as_ref()).is_some();
            if !is_dep && !candidate_indices.contains(&i) {
                candidate_indices.push(i);
            }
        }
        for (i, _) in self.readers.iter().enumerate() {
            if !candidate_indices.contains(&i) {
                candidate_indices.push(i);
            }
        }

        // Phase 1: Search authoritative embedded FileEntry table across candidate readers
        for &i in &candidate_indices {
            let reader = &self.readers[i];
            let origin = self
                .source_roots
                .get(i)
                .and_then(|r| r.as_deref())
                .or(self.workspace_root.as_deref())
                .or(self.root_dir.as_deref());

            for fid in 0..reader.files_count() {
                let entry_str = reader.get_file(fid);
                let h = reader.file_source_hash(fid);
                if h == 0 || entry_str.is_empty() {
                    continue;
                }
                if file_entry_matches(entry_str, origin, p, req_canon.as_deref()) {
                    return Some(h);
                }
                if let Some(ref abs_req) = ws_abs {
                    if file_entry_matches(entry_str, origin, abs_req, ws_canon.as_deref()) {
                        return Some(h);
                    }
                }
            }
        }

        // Phase 2: Fallback to sidecar metadata maps across candidate readers
        for &i in &candidate_indices {
            let origin = self
                .source_roots
                .get(i)
                .and_then(|r| r.as_deref())
                .or(self.workspace_root.as_deref())
                .or(self.root_dir.as_deref());

            if let Some(map) = self.reader_file_hashes.get(i) {
                for (k, &h) in map {
                    if h == 0 || k.is_empty() {
                        continue;
                    }
                    if file_entry_matches(k, origin, p, req_canon.as_deref()) {
                        return Some(h);
                    }
                    if let Some(ref abs_req) = ws_abs {
                        if file_entry_matches(k, origin, abs_req, ws_canon.as_deref()) {
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
