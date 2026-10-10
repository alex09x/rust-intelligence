/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! File-to-crate reader mapping and canonical module path resolution.

use super::WorkspaceIndex;
use std::path::Path;

impl WorkspaceIndex {
    /// Returns the index in `self.readers` for the crate that contains or owns the specified file path.
    pub fn reader_index_for_file(&self, file_path: &str) -> Option<usize> {
        let indices = self.reader_indices_for_file(file_path);
        // Prefer a unit whose key indicates a library target or primary unit, if distinguishable
        indices
            .iter()
            .copied()
            .find(|&i| {
                self.unit_keys
                    .get(i)
                    .and_then(|u| u.as_deref())
                    .map(|uk| uk.contains("(lib)"))
                    .unwrap_or(false)
            })
            .or_else(|| indices.into_iter().next())
    }

    /// Returns the indices in `self.readers` for all crate units that contain or own the specified file path.
    /// Employs longest prefix matching on `source_roots` so that subpackage targets are returned
    /// rather than broader ancestor workspace roots, invariant to reader insertion order.
    pub fn reader_indices_for_file(&self, file_path: &str) -> Vec<usize> {
        let p = Path::new(file_path);
        let abs_p = if p.is_absolute() {
            None
        } else {
            self.workspace_root.as_ref().map(|ws| ws.join(p))
        };

        // 1. Longest prefix match against source_roots
        let mut matching_roots: Vec<(usize, usize, usize)> = Vec::new(); // (reader_idx, comps, len)
        for (i, _) in self.readers.iter().enumerate() {
            if let Some(Some(root)) = self.source_roots.get(i) {
                if !root.as_os_str().is_empty() {
                    let matches = p.starts_with(root)
                        || abs_p.as_deref().map(|ap| ap.starts_with(root)).unwrap_or(false);
                    if matches {
                        matching_roots.push((i, root.components().count(), root.as_os_str().len()));
                    }
                }
            }
        }
        if let Some((_, max_comps, max_len)) = matching_roots
            .iter()
            .max_by_key(|(_, comps, len)| (*comps, *len))
            .copied()
        {
            return matching_roots
                .into_iter()
                .filter(|(_, comps, len)| *comps == max_comps && *len == max_len)
                .map(|(i, _, _)| i)
                .collect();
        }

        // 2. reader_file_hashes exact match
        let mut indices = Vec::new();
        for (i, hashes) in self.reader_file_hashes.iter().enumerate() {
            if hashes.contains_key(file_path) {
                indices.push(i);
            }
        }
        if !indices.is_empty() {
            return indices;
        }

        // 3. reader.find_file_id match
        for (i, reader) in self.readers.iter().enumerate() {
            if reader.find_file_id(file_path).is_some() {
                indices.push(i);
            }
        }
        if !indices.is_empty() {
            return indices;
        }

        // 4. Crate name contained in path
        for (i, reader) in self.readers.iter().enumerate() {
            let cname = reader.crate_name();
            if file_path.contains(&format!("/{cname}/"))
                || file_path.contains(&format!("\\{cname}\\"))
            {
                indices.push(i);
            }
        }
        if !indices.is_empty() {
            return indices;
        }

        // 5. Fallback for single crate workspace
        if self.readers.len() == 1 {
            return vec![0];
        }

        // 6. Fallback for single non-sysroot crate
        let non_sysroot: Vec<usize> = self
            .readers
            .iter()
            .enumerate()
            .filter(|(_, r)| !matches!(r.crate_name(), "core" | "alloc" | "std"))
            .map(|(i, _)| i)
            .collect();
        if non_sysroot.len() == 1 {
            return non_sysroot;
        }

        Vec::new()
    }

    /// Returns the canonical module path corresponding to `file_path` in the owning crate
    /// (e.g. `serde::de::impls` for `serde/src/de/impls.rs`).
    pub fn module_path_for_file(&self, file_path: &str) -> Option<String> {
        let idx = self.reader_index_for_file(file_path)?;
        let reader = &self.readers[idx];
        let cname = reader.crate_name();

        // 1. Inspect definitions declared directly in this file
        if let Some(fid) = reader.find_file_id(file_path) {
            for def in reader.definitions() {
                let span = def.span();
                if span.file_id == fid {
                    let path = reader.get_str(def.path_offset());
                    let name = reader.get_str(def.name_offset());
                    let suffix = format!("::{name}");
                    if let Some(parent) = path.strip_suffix(&suffix) {
                        if parent == cname || parent.starts_with(&format!("{cname}::")) {
                            return Some(parent.to_string());
                        }
                    }
                }
            }
        }

        // 2. Fallback based on relative file path
        let norm = file_path.replace('\\', "/");
        let p = Path::new(&norm);
        let file_stem = p.file_stem()?.to_str()?;

        if file_stem == "lib" || file_stem == "main" {
            return Some(cname.to_string());
        }

        let sub = if let Some(pos) = norm.find("/src/") {
            &norm[pos + 5..]
        } else if let Some(stripped) = norm.strip_prefix("src/") {
            stripped
        } else {
            &norm
        };

        let sub_p = Path::new(sub);
        let mut segs = vec![cname];
        if let Some(parent) = sub_p.parent() {
            for comp in parent.components() {
                if let Some(s) = comp.as_os_str().to_str() {
                    if !s.is_empty() && s != "." {
                        segs.push(s);
                    }
                }
            }
        }
        if file_stem != "mod" {
            segs.push(file_stem);
        }
        Some(segs.join("::"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::IndexReader;
    use crate::writer::IndexWriter;
    use std::collections::{HashMap, HashSet};
    use std::fs::File;
    use std::path::PathBuf;

    #[test]
    fn test_longest_prefix_matching_order_independence() {
        let temp_dir = tempfile::tempdir().unwrap();

        // 1. Create 3 ridx index files
        // Root package: "root_pkg"
        let root_ridx = temp_dir.path().join("root_pkg.ridx");
        let mut root_writer = IndexWriter::new("root_pkg", "0.1.0");
        let mut f_root = File::create(&root_ridx).unwrap();
        root_writer.write_to(&mut f_root).unwrap();
        drop(f_root);

        // Nested package: "x" (lib target)
        let x_lib_ridx = temp_dir.path().join("x_lib.ridx");
        let mut x_lib_writer = IndexWriter::new("x", "0.1.0");
        let mut f_x_lib = File::create(&x_lib_ridx).unwrap();
        x_lib_writer.write_to(&mut f_x_lib).unwrap();
        drop(f_x_lib);

        // Nested package: "x" (test target)
        let x_test_ridx = temp_dir.path().join("x_test.ridx");
        let mut x_test_writer = IndexWriter::new("x", "0.1.0");
        let mut f_x_test = File::create(&x_test_ridx).unwrap();
        x_test_writer.write_to(&mut f_x_test).unwrap();
        drop(f_x_test);

        let root_src = PathBuf::from("/repo");
        let x_src = PathBuf::from("/repo/crates/x");

        // Order A: [root_pkg, x_lib, x_test]
        let ws_a = WorkspaceIndex {
            readers: vec![
                IndexReader::open(&root_ridx).unwrap(),
                IndexReader::open(&x_lib_ridx).unwrap(),
                IndexReader::open(&x_test_ridx).unwrap(),
            ],
            paths: vec![root_ridx.clone(), x_lib_ridx.clone(), x_test_ridx.clone()],
            mtimes: vec![None, None, None],
            source_roots: vec![Some(root_src.clone()), Some(x_src.clone()), Some(x_src.clone())],
            unit_keys: vec![
                Some("root_pkg 0.1.0 (lib)".into()),
                Some("x 0.1.0 (lib)".into()),
                Some("x 0.1.0 (test)".into()),
            ],
            reader_generations: vec![None, None, None],
            reader_dependencies: vec![Vec::new(), Vec::new(), Vec::new()],
            reader_extern_units: vec![HashMap::new(), HashMap::new(), HashMap::new()],
            reader_file_hashes: vec![HashMap::new(), HashMap::new(), HashMap::new()],
            reader_is_dependency: vec![false, false, false],
            meta_paths: vec![None, None, None],
            meta_mtimes: vec![None, None, None],
            last_reloaded_files: HashSet::new(),
            root_dir: None,
            workspace_root: Some(root_src.clone()),
            manifest_path: None,
            manifest_mtime: None,
            manifest_len: None,
            manifest_hash: None,
            manifest_generation: None,
            manifest_ino: None,
            active_variant_id: None,
            sysroot_cas_key: None,
        };

        // Order B: [x_lib, x_test, root_pkg] (reversed order)
        let ws_b = WorkspaceIndex {
            readers: vec![
                IndexReader::open(&x_lib_ridx).unwrap(),
                IndexReader::open(&x_test_ridx).unwrap(),
                IndexReader::open(&root_ridx).unwrap(),
            ],
            paths: vec![x_lib_ridx.clone(), x_test_ridx.clone(), root_ridx.clone()],
            mtimes: vec![None, None, None],
            source_roots: vec![Some(x_src.clone()), Some(x_src.clone()), Some(root_src.clone())],
            unit_keys: vec![
                Some("x 0.1.0 (lib)".into()),
                Some("x 0.1.0 (test)".into()),
                Some("root_pkg 0.1.0 (lib)".into()),
            ],
            reader_generations: vec![None, None, None],
            reader_dependencies: vec![Vec::new(), Vec::new(), Vec::new()],
            reader_extern_units: vec![HashMap::new(), HashMap::new(), HashMap::new()],
            reader_file_hashes: vec![HashMap::new(), HashMap::new(), HashMap::new()],
            reader_is_dependency: vec![false, false, false],
            meta_paths: vec![None, None, None],
            meta_mtimes: vec![None, None, None],
            last_reloaded_files: HashSet::new(),
            root_dir: None,
            workspace_root: Some(root_src.clone()),
            manifest_path: None,
            manifest_mtime: None,
            manifest_len: None,
            manifest_hash: None,
            manifest_generation: None,
            manifest_ino: None,
            active_variant_id: None,
            sysroot_cas_key: None,
        };

        let file_in_x = "/repo/crates/x/src/lib.rs";
        let file_in_root = "/repo/src/lib.rs";

        // Query file in nested member `crates/x`:
        // Order A: readers 1 and 2 belong to `x`
        assert_eq!(ws_a.reader_indices_for_file(file_in_x), vec![1, 2]);
        assert_eq!(ws_a.reader_index_for_file(file_in_x), Some(1));
        assert_eq!(ws_a.crate_name_for_file(file_in_x), Some("x"));

        // Order B: readers 0 and 1 belong to `x`
        assert_eq!(ws_b.reader_indices_for_file(file_in_x), vec![0, 1]);
        assert_eq!(ws_b.reader_index_for_file(file_in_x), Some(0));
        assert_eq!(ws_b.crate_name_for_file(file_in_x), Some("x"));

        // Query file in root crate:
        // Order A: reader 0 belongs to root_pkg
        assert_eq!(ws_a.reader_indices_for_file(file_in_root), vec![0]);
        assert_eq!(ws_a.reader_index_for_file(file_in_root), Some(0));
        assert_eq!(ws_a.crate_name_for_file(file_in_root), Some("root_pkg"));

        // Order B: reader 2 belongs to root_pkg
        assert_eq!(ws_b.reader_indices_for_file(file_in_root), vec![2]);
        assert_eq!(ws_b.reader_index_for_file(file_in_root), Some(2));
        assert_eq!(ws_b.crate_name_for_file(file_in_root), Some("root_pkg"));
    }
}
