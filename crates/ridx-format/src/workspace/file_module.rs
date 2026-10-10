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
        let p = Path::new(file_path);
        let is_abs = p.is_absolute();
        for (i, _) in self.readers.iter().enumerate() {
            if let Some(Some(root)) = self.source_roots.get(i) {
                if is_abs && p.starts_with(root) {
                    return Some(i);
                }
            }
        }
        for (i, hashes) in self.reader_file_hashes.iter().enumerate() {
            if hashes.contains_key(file_path) {
                return Some(i);
            }
        }
        for (i, reader) in self.readers.iter().enumerate() {
            let cname = reader.crate_name();
            if file_path.contains(&format!("/{cname}/"))
                || file_path.contains(&format!("\\{cname}\\"))
            {
                return Some(i);
            }
        }
        for (i, reader) in self.readers.iter().enumerate() {
            if reader.find_file_id(file_path).is_some() {
                return Some(i);
            }
        }
        if self.readers.len() == 1 {
            return Some(0);
        }
        let non_sysroot: Vec<usize> = self
            .readers
            .iter()
            .enumerate()
            .filter(|(_, r)| !matches!(r.crate_name(), "core" | "alloc" | "std"))
            .map(|(i, _)| i)
            .collect();
        if non_sysroot.len() == 1 {
            return Some(non_sysroot[0]);
        }
        None
    }

    /// Returns the indices in `self.readers` for all crate units that contain or own the specified file path.
    pub fn reader_indices_for_file(&self, file_path: &str) -> Vec<usize> {
        let p = Path::new(file_path);
        let is_abs = p.is_absolute();
        let mut indices = Vec::new();
        for (i, _) in self.readers.iter().enumerate() {
            if let Some(Some(root)) = self.source_roots.get(i) {
                if is_abs && p.starts_with(root) {
                    indices.push(i);
                }
            }
        }
        if !indices.is_empty() {
            return indices;
        }
        for (i, hashes) in self.reader_file_hashes.iter().enumerate() {
            if hashes.contains_key(file_path) {
                indices.push(i);
            }
        }
        if !indices.is_empty() {
            return indices;
        }
        for (i, reader) in self.readers.iter().enumerate() {
            if reader.find_file_id(file_path).is_some() {
                indices.push(i);
            }
        }
        if !indices.is_empty() {
            return indices;
        }
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
        if let Some(idx) = self.reader_index_for_file(file_path) {
            indices.push(idx);
        }
        indices
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
