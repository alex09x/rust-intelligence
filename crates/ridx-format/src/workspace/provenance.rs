/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

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

/// Checks whether a path contains a `ParentDir` (`..`) following a symlink component,
/// meaning lexical normalization would pop across a symlink boundary and diverge from
/// the true filesystem target.
fn path_traverses_symlink_parent(path: &Path) -> bool {
    let mut current = PathBuf::new();
    let mut saw_symlink = false;
    for c in path.components() {
        match c {
            std::path::Component::Normal(seg) => {
                current.push(seg);
                if let Ok(meta) = fs::symlink_metadata(&current) {
                    if meta.file_type().is_symlink() {
                        saw_symlink = true;
                    }
                }
            }
            std::path::Component::ParentDir => {
                if saw_symlink {
                    return true;
                }
                current.pop();
            }
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                current.push(c);
            }
            std::path::Component::CurDir => {}
        }
    }
    false
}

/// Matches an entry path from an index shard or sidecar map against a requested file path.
/// Resolves real filesystem identity for both entry and request before any lexical collapse,
/// guaranteeing that lexical collapse never overrides known unequal canonical paths.
fn file_entry_matches(
    entry_str: &str,
    reader_origin: Option<&Path>,
    req_path: &Path,
    req_canon: Option<&Path>,
) -> bool {
    if entry_str.is_empty() {
        return false;
    }

    let entry_p = Path::new(entry_str);

    // 1. Resolve original uncollapsed entry path under reader's origin
    let entry_orig = if entry_p.is_absolute() {
        entry_p.to_path_buf()
    } else if let Some(orig) = reader_origin {
        orig.join(entry_p)
    } else {
        entry_p.to_path_buf()
    };

    // 2. Resolve canonical identities on the filesystem before any lexical collapse
    let entry_canon = entry_orig.canonicalize().ok();
    let effective_req_canon = req_canon.map(|p| p.to_path_buf()).or_else(|| {
        if req_path.is_absolute() {
            req_path.canonicalize().ok()
        } else {
            None
        }
    });

    // 3. Authoritative filesystem identity comparison
    if let (Some(ref c_entry), Some(ref c_req)) = (&entry_canon, &effective_req_canon) {
        if c_entry == c_req {
            return true;
        }
        // When both filesystem identities are known and unequal,
        // lexical collapse MUST NOT override known unequal canonical paths.
        return false;
    }

    // 4. Literal string match (only when canonical comparison did not rule it out)
    if entry_str == req_path.to_str().unwrap_or("") {
        return true;
    }

    // 5. Lexical fallback: only allowed where it cannot reinterpret a symlink traversal
    if path_traverses_symlink_parent(&entry_orig)
        || path_traverses_symlink_parent(req_path)
        || reader_origin.is_some_and(|orig| path_traverses_symlink_parent(&orig.join(req_path)))
    {
        return false;
    }

    let norm_entry = normalize_path(&entry_orig);

    if req_path.is_absolute() {
        let norm_req = normalize_path(req_path);
        if norm_entry == norm_req {
            return true;
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
            if let (Some(ref c_entry), Ok(c_orig)) = (&entry_canon, orig.canonicalize()) {
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
        let explicit_origin = self.workspace_root.as_deref().or(self.root_dir.as_deref());

        let (ws_abs, req_canon) = if p.is_absolute() {
            (None, p.canonicalize().ok())
        } else if let Some(origin) = explicit_origin {
            let abs = origin.join(p);
            let canon = abs.canonicalize().ok();
            (Some(abs), canon)
        } else {
            (None, p.canonicalize().ok())
        };
        let ws_canon = req_canon.clone();

        let mut candidate_indices: Vec<usize> = Vec::with_capacity(self.readers.len());
        if let Some(ref c_p) = req_canon {
            for (i, _) in self.readers.iter().enumerate() {
                if let Some(Some(ref root)) = self.source_roots.get(i) {
                    if let Ok(c_root) = root.canonicalize() {
                        if c_p.starts_with(&c_root) && !candidate_indices.contains(&i) {
                            candidate_indices.push(i);
                        }
                    }
                }
            }
        }
        if let Some(ref abs) = ws_abs {
            let norm_abs = normalize_path(abs);
            for (i, _) in self.readers.iter().enumerate() {
                if let Some(Some(ref root)) = self.source_roots.get(i) {
                    let norm_root = normalize_path(root);
                    if norm_abs.starts_with(&norm_root) && !candidate_indices.contains(&i) {
                        candidate_indices.push(i);
                    }
                }
            }
        } else if p.is_absolute() {
            let norm_p = normalize_path(p);
            for (i, _) in self.readers.iter().enumerate() {
                if let Some(Some(ref root)) = self.source_roots.get(i) {
                    let norm_root = normalize_path(root);
                    if norm_p.starts_with(&norm_root) && !candidate_indices.contains(&i) {
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
        let explicit_origin = self.workspace_root.as_deref().or(self.root_dir.as_deref());

        let (ws_abs, req_canon) = if p.is_absolute() {
            (None, p.canonicalize().ok())
        } else if let Some(origin) = explicit_origin {
            let abs = origin.join(p);
            let canon = abs.canonicalize().ok();
            (Some(abs), canon)
        } else {
            (None, p.canonicalize().ok())
        };
        let ws_canon = req_canon.clone();

        // Order candidate readers:
        // 1. Dependency readers whose source_root contains p
        // 2. Workspace member readers (non-dependency)
        // 3. Other dependency readers
        let mut candidate_indices: Vec<usize> = Vec::with_capacity(self.readers.len());
        if let Some(ref c_p) = req_canon {
            for (i, _) in self.readers.iter().enumerate() {
                if let Some(Some(ref root)) = self.source_roots.get(i) {
                    if let Ok(c_root) = root.canonicalize() {
                        if c_p.starts_with(&c_root) && !candidate_indices.contains(&i) {
                            candidate_indices.push(i);
                        }
                    }
                }
            }
        }
        if let Some(ref abs) = ws_abs {
            let norm_abs = normalize_path(abs);
            for (i, _) in self.readers.iter().enumerate() {
                if let Some(Some(ref root)) = self.source_roots.get(i) {
                    let norm_root = normalize_path(root);
                    if norm_abs.starts_with(&norm_root) && !candidate_indices.contains(&i) {
                        candidate_indices.push(i);
                    }
                }
            }
        } else if p.is_absolute() {
            let norm_p = normalize_path(p);
            for (i, _) in self.readers.iter().enumerate() {
                if let Some(Some(ref root)) = self.source_roots.get(i) {
                    let norm_root = normalize_path(root);
                    if norm_p.starts_with(&norm_root) && !candidate_indices.contains(&i) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writer::IndexWriter;
    use std::fs::File;

    struct TestDir(PathBuf);
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn new_test_dir(prefix: &str) -> TestDir {
        static CNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let id = CNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!("{}_{}_{}", prefix, std::process::id(), id));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        TestDir(path)
    }

    #[test]
    fn test_normalize_path_variants() {
        assert_eq!(normalize_path(Path::new("a/b/../c")), PathBuf::from("a/c"));
        assert_eq!(normalize_path(Path::new("./a/./b")), PathBuf::from("a/b"));
        assert_eq!(
            normalize_path(Path::new("/a/b/../../c")),
            PathBuf::from("/c")
        );
        assert_eq!(normalize_path(Path::new("")), PathBuf::from(""));
        assert_eq!(
            normalize_path(Path::new("a/../../b")),
            PathBuf::from("../b")
        );
    }

    #[test]
    fn test_path_traverses_symlink_parent() {
        assert!(!path_traverses_symlink_parent(Path::new(
            "/tmp/normal/path"
        )));
        assert!(!path_traverses_symlink_parent(Path::new(
            "relative/path/../foo"
        )));
    }

    #[test]
    fn test_file_entry_matches_empty_and_lexical() {
        assert!(!file_entry_matches("", None, Path::new("/tmp/test"), None));
        assert!(file_entry_matches(
            "/tmp/foo",
            None,
            Path::new("/tmp/foo"),
            None
        ));
        assert!(!file_entry_matches(
            "/tmp/foo",
            None,
            Path::new("/tmp/bar"),
            None
        ));

        let rel_entry = "src/lib.rs";
        let base = Path::new("/workspace");
        assert!(file_entry_matches(
            rel_entry,
            Some(base),
            Path::new("/workspace/src/lib.rs"),
            None
        ));
    }

    #[test]
    fn test_shard_mtime_and_source_hash_for_file() {
        let temp_dir = std::env::temp_dir().join(format!("ridx_prov_test_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);
        let src_file = temp_dir.join("src/lib.rs");
        let _ = fs::create_dir_all(src_file.parent().unwrap());
        let _ = fs::write(&src_file, b"pub fn hello() {}\n");

        let mut writer = crate::writer::IndexWriter::new("prov_crate", "0.1.0");
        let hash = crate::format::normalized_source_hash(b"pub fn hello() {}\n");
        let mut map = HashMap::new();
        map.insert(src_file.to_str().unwrap().to_string(), hash);
        writer.set_file_hashes(map);
        writer.add_definition(crate::writer::RawDefinition {
            def_id: 1,
            kind: crate::format::DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            name: "hello".to_string(),
            path: "prov_crate::hello".to_string(),
            signature: None,
            file: src_file.to_str().unwrap().to_string(),
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 16,
        });
        let ridx_path = temp_dir.join("prov.ridx");
        let mut f = fs::File::create(&ridx_path).unwrap();
        writer.write_to(&mut f).unwrap();
        drop(f);

        let ws = WorkspaceIndex::open(&ridx_path).unwrap();
        assert_eq!(
            ws.source_hash_for_file(src_file.to_str().unwrap()),
            Some(hash)
        );
        assert!(ws
            .shard_mtime_for_file(src_file.to_str().unwrap())
            .is_some());
        assert_eq!(ws.source_hash_for_file("/nonexistent/file.rs"), None);
        assert_eq!(ws.shard_mtime_for_file("/nonexistent/file.rs"), None);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_shard_mtime_and_source_hash_with_dependency_source_root() {
        let temp_dir = std::env::temp_dir().join(format!("ridx_prov_dep_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);

        let dep_root = temp_dir.join("dep_src");
        let _ = fs::create_dir_all(&dep_root);
        let dep_file = dep_root.join("dep.rs");
        let _ = fs::write(&dep_file, b"pub fn dep() {}\n");

        let mut dep_writer = crate::writer::IndexWriter::new("dep_lib", "0.1.0");
        dep_writer.set_unit_key("dep_lib:0.1.0");
        dep_writer.set_generation("1111");
        let hash = crate::format::normalized_source_hash(b"pub fn dep() {}\n");
        let mut map = HashMap::new();
        map.insert("dep.rs".to_string(), hash);
        dep_writer.set_file_hashes(map);
        dep_writer.add_definition(crate::writer::RawDefinition {
            def_id: 1,
            kind: crate::format::DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            name: "dep".to_string(),
            path: "dep_lib::dep".to_string(),
            signature: None,
            file: "dep.rs".to_string(),
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 10,
        });

        let cache_dir = temp_dir.join("cache");
        let _ = fs::create_dir_all(&cache_dir);
        let dep_ridx = cache_dir.join("dep.ridx");
        let mut f = fs::File::create(&dep_ridx).unwrap();
        dep_writer.write_to(&mut f).unwrap();
        drop(f);

        let v_manifest = temp_dir.join("variant_manifest.json");
        let manifest_data = serde_json::json!({
            "version": "2.0",
            "format_version": crate::format::CURRENT_FORMAT_VERSION,
            "extractor_version": crate::format::EXTRACTOR_VERSION,
            "active_variant_id": "v1",
            "variant_dir": "variants/v1",
            "workspace_root": temp_dir.display().to_string(),
            "dependency_shards": [{
                "name": "dep_lib",
                "version": "0.1.0",
                "unit_key": "dep_lib:0.1.0",
                "generation": "1111",
                "object_path": dep_ridx.display().to_string(),
                "source_root": dep_root.display().to_string(),
                "is_dependency": true,
                "dependencies": [],
                "extern_units": {},
                "file_hashes": { "dep.rs": hash }
            }]
        });
        fs::write(&v_manifest, serde_json::to_string(&manifest_data).unwrap()).unwrap();

        let ws = WorkspaceIndex::open(&temp_dir).unwrap();

        // Query with absolute path to dep.rs
        let full_dep_path = dep_file.display().to_string();
        assert_eq!(ws.source_hash_for_file(&full_dep_path), Some(hash));
        assert!(ws.shard_mtime_for_file(&full_dep_path).is_some());

        // Query with relative path when ws has workspace_root
        assert_eq!(ws.source_hash_for_file("dep_src/dep.rs"), Some(hash));
        assert!(ws.shard_mtime_for_file("dep_src/dep.rs").is_some());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_provenance_edge_cases_and_sidecar_fallback() {
        // 1. normalize_path with root .. and leading components
        assert_eq!(normalize_path(Path::new("/..")), PathBuf::from("/"));
        assert_eq!(normalize_path(Path::new("a/b/../../c")), PathBuf::from("c"));
        assert_eq!(
            normalize_path(Path::new("../../a")),
            PathBuf::from("../../a")
        );

        // 2. Symlink parent traversal
        let temp_dir = std::env::temp_dir().join(format!("ridx_prov_edge_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);

        let real_dir = temp_dir.join("real");
        let _ = fs::create_dir_all(&real_dir);
        let link_dir = temp_dir.join("link");
        #[cfg(unix)]
        let _ = std::os::unix::fs::symlink(&real_dir, &link_dir);

        #[cfg(unix)]
        {
            let p_with_symlink = link_dir.join("../real");
            assert!(path_traverses_symlink_parent(&p_with_symlink));
        }

        // 3. file_entry_matches relative matches
        assert!(file_entry_matches(
            "src/lib.rs",
            None,
            Path::new("src/lib.rs"),
            None
        ));
        assert!(!file_entry_matches(
            "src/lib.rs",
            None,
            Path::new("src/other.rs"),
            None
        ));
        assert!(!file_entry_matches("", None, Path::new("src/lib.rs"), None));

        // 4. Shard mtime and source hash when file not found
        let test_idx = temp_dir.join("test.ridx");
        let mut writer = crate::writer::IndexWriter::new("prov_crate", "0.1.0");
        let mut map = HashMap::new();
        map.insert("src/lib.rs".to_string(), 12345u64);
        writer.set_file_hashes(map);
        let mut f = fs::File::create(&test_idx).unwrap();
        writer.write_to(&mut f).unwrap();
        drop(f);

        let ws = WorkspaceIndex::open(&test_idx).unwrap();
        assert_eq!(ws.shard_mtime_for_file("nonexistent.rs"), None);
        assert_eq!(ws.source_hash_for_file("nonexistent.rs"), None);

        // 5. load_file_metadata directly
        let meta_file = temp_dir.join("test.meta");
        fs::write(&meta_file, b"{ corrupted_json").unwrap();
        let embedded = HashMap::new();
        let (sroot, ukey, gen, deps, ext, hashes, mpath, mmtime) =
            WorkspaceIndex::load_file_metadata(
                &test_idx,
                &temp_dir,
                "prov_crate",
                "uk",
                "gen",
                &embedded,
            );
        assert_eq!(sroot, None);
        assert_eq!(ukey, Some("uk".to_string()));
        assert_eq!(gen, Some("gen".to_string()));
        assert!(deps.is_empty());
        assert!(ext.is_empty());
        assert!(hashes.is_empty());
        assert!(mpath.is_some());
        assert_eq!(mmtime, None);

        let (_, ukey_empty, gen_empty, _, _, _, _, _) = WorkspaceIndex::load_file_metadata(
            &test_idx,
            &temp_dir,
            "prov_crate",
            "",
            "",
            &embedded,
        );
        assert_eq!(ukey_empty, None);
        assert_eq!(gen_empty, None);

        let _ = fs::remove_dir_all(&temp_dir);

        // 6. Secondary candidate matching in shard_mtime_for_file and source_hash_for_file (lines 267-271, 389-393)
        let temp2 = std::env::temp_dir().join(format!("ridx_prov_sec_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp2);
        let test_idx2 = temp2.join("test2.ridx");
        let mut writer2 = crate::writer::IndexWriter::new("prov_crate2", "0.1.0");
        let mut f2 = fs::File::create(&test_idx2).unwrap();
        writer2.write_to(&mut f2).unwrap();
        drop(f2);

        let mut ws_cand = WorkspaceIndex::open(&test_idx2).unwrap();
        ws_cand.workspace_root = Some(temp2.clone());
        ws_cand.source_roots[0] = Some(PathBuf::from("/other_dep_root"));
        let sub_lib = temp2.join("sub/lib.rs");
        ws_cand.reader_file_hashes[0].insert(sub_lib.to_str().unwrap().to_string(), 88888u64);
        assert!(ws_cand.shard_mtime_for_file("sub/lib.rs").is_some());
        assert_eq!(ws_cand.source_hash_for_file("sub/lib.rs"), Some(88888));

        // 7. Non-normalized entry matching (lines 151-158)
        let sub_dir = temp2.join("sub_dir");
        let _ = fs::create_dir_all(&sub_dir);
        let lib_sub = sub_dir.join("lib.rs");
        let _ = fs::write(&lib_sub, b"// content");
        let non_norm_entry = temp2.join("sub_dir/../sub_dir/lib.rs");
        assert!(file_entry_matches(
            non_norm_entry.to_str().unwrap(),
            Some(&temp2),
            Path::new("sub_dir/lib.rs"),
            None,
        ));

        // 8. CurDir in symlink traversal (line 67) & relative entry match with no reader_origin (line 142)
        assert!(!path_traverses_symlink_parent(Path::new(
            "./sub_dir/lib.rs"
        )));
        assert!(file_entry_matches(
            "sub_dir/lib.rs",
            None,
            Path::new("sub_dir/lib.rs"),
            None
        ));

        // 9. Sidecar with h == 0 or empty key (line 384)
        ws_cand.reader_file_hashes[0].insert("".to_string(), 12345);
        ws_cand.reader_file_hashes[0].insert("dummy.rs".to_string(), 0);
        assert_eq!(ws_cand.source_hash_for_file("dummy.rs"), None);

        // 10. Shard mtime and source hash matching ws_abs in reader (lines 247, 366)
        let mut w_reader_abs = crate::writer::IndexWriter::new("abs_crate", "0.1.0");
        let abs_file_str = temp2.join("sub_dir/lib.rs").to_str().unwrap().to_string();
        let hash = crate::format::normalized_source_hash(b"// content");
        let mut fmap = HashMap::new();
        fmap.insert(abs_file_str.clone(), hash);
        w_reader_abs.set_file_hashes(fmap);
        w_reader_abs.add_definition(crate::writer::RawDefinition {
            def_id: 1,
            kind: crate::DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            name: "abs_fn".to_string(),
            path: "abs_crate::abs_fn".to_string(),
            signature: None,
            file: abs_file_str,
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 10,
        });
        let abs_idx_file = temp2.join("abs.ridx");
        let mut f_abs = fs::File::create(&abs_idx_file).unwrap();
        w_reader_abs.write_to(&mut f_abs).unwrap();
        drop(f_abs);
        let mut ws_abs = WorkspaceIndex::open(&abs_idx_file).unwrap();
        ws_abs.workspace_root = Some(temp2.clone());
        assert!(ws_abs.shard_mtime_for_file("sub_dir/lib.rs").is_some());
        assert_eq!(
            ws_abs.source_hash_for_file("sub_dir/lib.rs"),
            Some(crate::format::normalized_source_hash(b"// content"))
        );

        // 11. load_file_metadata with computed digest generation matching and mismatching (lines 440-448)
        let deps_dir = temp2.join(".deps");
        fs::create_dir_all(&deps_dir).unwrap();
        let mut fhashes = HashMap::new();
        fhashes.insert("sub_dir/lib.rs".to_string(), 55555u64);
        let expected_digest = crate::format::compute_source_snapshot_digest(&fhashes);
        let expected_gen_str = format!("{:016x}", expected_digest);

        let dep_info = crate::workspace::DependencyShardInfo {
            name: "computed_crate".to_string(),
            version: "0.1.0".to_string(),
            unit_key: "comp_uk".to_string(),
            generation: None,
            object_path: abs_idx_file.display().to_string(),
            source_root: Some(temp2.display().to_string()),
            is_dependency: true,
            dependencies: vec![],
            extern_units: HashMap::new(),
            file_hashes: fhashes,
        };
        fs::write(
            deps_dir.join("computed_crate.json"),
            serde_json::to_string(&dep_info).unwrap(),
        )
        .unwrap();

        // Mismatching expected generation (lines 445-447)
        let (sroot_mismatch, _, _, _, _, _, _, _) = WorkspaceIndex::load_file_metadata(
            &temp2.join("computed_crate.ridx"),
            &temp2,
            "computed_crate",
            "comp_uk",
            "wrong_gen_12345678",
            &HashMap::new(),
        );
        assert_eq!(sroot_mismatch, None);

        // Matching expected generation (lines 441-444, 463-472)
        let (sroot_match, ukey_match, gen_match, _, _, _, _, _) =
            WorkspaceIndex::load_file_metadata(
                &temp2.join("computed_crate.ridx"),
                &temp2,
                "computed_crate",
                "comp_uk",
                &expected_gen_str,
                &HashMap::new(),
            );
        assert_eq!(sroot_match, Some(temp2.clone()));
        assert_eq!(ukey_match, Some("comp_uk".to_string()));
        assert_eq!(gen_match, Some(expected_gen_str));

        let _ = fs::remove_dir_all(&temp2);
    }

    #[test]
    fn test_provenance_additional_coverage() {
        // Line 142: both entry and req are relative and match
        assert!(file_entry_matches(
            "src/lib.rs",
            None,
            Path::new("src/lib.rs"),
            None
        ));

        // Line 127: path traverses symlink parent
        let temp = new_test_dir("prov_cov_symlink");
        let real_dir = temp.0.join("real");
        fs::create_dir_all(&real_dir).unwrap();
        let symlink_dir = temp.0.join("link");
        #[cfg(unix)]
        {
            let _ = std::os::unix::fs::symlink(&real_dir, &symlink_dir);
            let bad_path = symlink_dir.join("../escape.rs");
            assert!(!file_entry_matches("src/lib.rs", None, &bad_path, None));
        }

        // Test shard_mtime_for_file and source_hash_for_file with absolute path matching dep source_root (lines 209, 327)
        let dep_dir = new_test_dir("prov_cov_dep");
        let dep_src = dep_dir.0.join("src");
        fs::create_dir_all(&dep_src).unwrap();
        let dep_file = dep_src.join("lib.rs");
        fs::write(&dep_file, b"fn dep() {}").unwrap();

        let idx_file = dep_dir.0.join("dep.ridx");
        let mut w = IndexWriter::new("prov_dep", "0.1.0");
        w.set_unit_key("prov_dep_key");
        w.get_or_insert_file("");
        w.get_or_insert_file("src/lib.rs");
        let mut hashes = HashMap::new();
        hashes.insert("src/lib.rs".to_string(), 77777u64);
        w.set_file_hashes(hashes);
        let mut f = File::create(&idx_file).unwrap();
        w.write_to(&mut f).unwrap();
        drop(f);

        let mut ws = WorkspaceIndex::open(&idx_file).unwrap();
        ws.source_roots = vec![Some(dep_dir.0.clone())];
        ws.reader_is_dependency = vec![true];

        // Absolute query matching dep_dir (lines 209, 327)
        let mtime = ws.shard_mtime_for_file(&dep_file.to_string_lossy());
        assert!(mtime.is_some());
        let hash = ws.source_hash_for_file(&dep_file.to_string_lossy());
        assert_eq!(hash, Some(77777u64));

        // Sidecar only file for shard_mtime_for_file (lines 265, 270)
        let mut sidecar_map = HashMap::new();
        sidecar_map.insert("src/sidecar_only.rs".to_string(), 88888u64);
        ws.reader_file_hashes = vec![sidecar_map];
        let sidecar_mtime = ws.shard_mtime_for_file("src/sidecar_only.rs");
        assert!(sidecar_mtime.is_some());

        // Relative path joined with workspace_root matching embedded table (line 366)
        ws.workspace_root = Some(dep_dir.0.clone());
        let ws_abs_hash = ws.source_hash_for_file("src/lib.rs");
        assert_eq!(ws_abs_hash, Some(77777u64));

        // Direct file_entry_matches edge cases (lines 142, 154, 156, 158)
        assert!(file_entry_matches(
            "./src/lib.rs",
            None,
            Path::new("src/lib.rs"),
            None
        ));
        assert!(file_entry_matches(
            &dep_file.to_string_lossy(),
            Some(&dep_dir.0),
            Path::new("src/lib.rs"),
            None
        ));

        // Relative query with workspace_root set matching norm_root (lines 198-199, 316-317)
        let mtime_ws = ws.shard_mtime_for_file("src/lib.rs");
        assert!(mtime_ws.is_some());
        let hash_ws = ws.source_hash_for_file("src/lib.rs");
        assert_eq!(hash_ws, Some(77777u64));

        // Symlinked entry_str for file_entry_matches (lines 154-156)
        #[cfg(unix)]
        {
            let symlink_dep = dep_dir.0.join("symlink_dep");
            if std::os::unix::fs::symlink(&dep_dir.0, &symlink_dep).is_ok() {
                let symlink_file = symlink_dep.join("src/lib.rs");
                assert!(file_entry_matches(
                    &symlink_file.to_string_lossy(),
                    Some(&dep_dir.0),
                    Path::new("src/lib.rs"),
                    None
                ));
            }
        }

        // load_file_metadata with mismatching computed generation (line 448) and invalid JSON (line 474)
        let meta_test_dir = new_test_dir("meta_test_cov");
        let deps_dir = meta_test_dir.0.join(".deps");
        fs::create_dir_all(&deps_dir).unwrap();

        // Invalid JSON file (line 474)
        fs::write(deps_dir.join("sample.json"), b"invalid json content").unwrap();
        let _ = WorkspaceIndex::load_file_metadata(
            &meta_test_dir.0.join("sample.ridx"),
            &meta_test_dir.0,
            "sample",
            "",
            "",
            &HashMap::new(),
        );

        // Valid JSON with mismatching computed generation (line 448)
        let mut fh = HashMap::new();
        fh.insert("src/lib.rs".to_string(), 11111u64);
        let dep_info_mismatch = DependencyShardInfo {
            name: "sample".to_string(),
            version: "0.1.0".to_string(),
            unit_key: "uk_sample".to_string(),
            generation: None,
            object_path: "".to_string(),
            source_root: None,
            is_dependency: true,
            dependencies: vec![],
            extern_units: HashMap::new(),
            file_hashes: fh,
        };
        fs::write(
            deps_dir.join("sample.json"),
            serde_json::to_string(&dep_info_mismatch).unwrap(),
        )
        .unwrap();
        let _ = WorkspaceIndex::load_file_metadata(
            &meta_test_dir.0.join("sample.ridx"),
            &meta_test_dir.0,
            "sample",
            "uk_sample",
            "different_gen_than_computed",
            &HashMap::new(),
        );

        // ws_abs matching where direct relative query against reader origin fails (lines 247, 269, 366, 391)
        let ws_parent = new_test_dir("ws_abs_parent");
        let ws_sub = ws_parent.0.join("sub_pkg");
        let _ = fs::create_dir_all(ws_sub.join("src"));
        let abs_lib = ws_sub.join("src/virtual_lib.rs");

        let mut abs_writer = IndexWriter::new("sub_pkg", "0.1.0");
        let abs_lib_str = abs_lib.to_string_lossy().to_string();
        abs_writer.get_or_insert_file(&abs_lib_str);
        abs_writer.add_file_hash(&abs_lib_str, 99999u64);
        let abs_idx_file = ws_sub.join("sub_pkg.ridx");
        let mut f_abs = fs::File::create(&abs_idx_file).unwrap();
        abs_writer.write_to(&mut f_abs).unwrap();
        drop(f_abs);

        let abs_reader = crate::reader::IndexReader::open(&abs_idx_file).unwrap();
        let mut ws_abs_idx = WorkspaceIndex::open(&abs_idx_file).unwrap();
        ws_abs_idx.workspace_root = Some(ws_parent.0.clone());
        ws_abs_idx.source_roots = vec![Some(ws_sub.clone())];
        ws_abs_idx.readers = vec![abs_reader];
        ws_abs_idx.mtimes = vec![Some(std::time::SystemTime::now())];

        // 1. Phase 1 ws_abs match (lines 247, 366)
        let mtime_p1 = ws_abs_idx.shard_mtime_for_file("sub_pkg/src/virtual_lib.rs");
        assert!(mtime_p1.is_some());
        let hash_p1 = ws_abs_idx.source_hash_for_file("sub_pkg/src/virtual_lib.rs");
        assert_eq!(hash_p1, Some(99999u64));

        // 2. Phase 2 ws_abs match via reader_file_hashes (lines 269, 391)
        let mut sidecar_map2 = HashMap::new();
        let abs_sidecar = ws_sub
            .join("src/virtual_sidecar.rs")
            .to_string_lossy()
            .to_string();
        sidecar_map2.insert(abs_sidecar, 55555u64);
        ws_abs_idx.reader_file_hashes = vec![sidecar_map2];
        let mtime_p2 = ws_abs_idx.shard_mtime_for_file("sub_pkg/src/virtual_sidecar.rs");
        assert!(mtime_p2.is_some());
        let hash_p2 = ws_abs_idx.source_hash_for_file("sub_pkg/src/virtual_sidecar.rs");
        assert_eq!(hash_p2, Some(55555u64));
    }
}
