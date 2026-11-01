/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::{fast_path_starts_with, is_clean_posix_bytes};
use crate::workspace::WorkspaceIndex;
use std::path::Path;

impl WorkspaceIndex {
    /// Resolves `file` directly to its `file_id` in `reader_idx` without allocating intermediate strings.
    pub(crate) fn target_file_id_for_reader(&self, reader_idx: usize, file: &str) -> Option<u32> {
        let _ = self.readers.get(reader_idx)?;
        let p = Path::new(file);
        let p_clean = is_clean_posix_bytes(p.as_os_str().as_encoded_bytes());
        let root_opt = self.reader_root(reader_idx);

        if p.is_absolute() {
            if let Some(fid) = self.reader_find_file_id(reader_idx, file) {
                return Some(fid);
            }
            if let Some(root) = root_opt {
                let matches_root = if root.is_absolute() {
                    fast_path_starts_with(p, p_clean, root)
                } else if let Some(ws_root) = &self.workspace_root {
                    p.starts_with(ws_root.join(root))
                } else {
                    false
                };
                if !matches_root {
                    return None;
                }
            } else if let Some(ws_root) = &self.workspace_root {
                if !fast_path_starts_with(p, p_clean, ws_root) {
                    return None;
                }
            }
            if let Some(ws_root) = &self.workspace_root {
                if let Some(s) = p.strip_prefix(ws_root).ok().and_then(|r| r.to_str()) {
                    if let Some(fid) = self.reader_find_file_id(reader_idx, s) {
                        return Some(fid);
                    }
                }
            }
            if let Some(root) = root_opt {
                if let Some(s) = p.strip_prefix(root).ok().and_then(|r| r.to_str()) {
                    if let Some(fid) = self.reader_find_file_id(reader_idx, s) {
                        return Some(fid);
                    }
                }
            }
            return None;
        }

        if let Some(fid) = self.reader_find_file_id(reader_idx, file) {
            return Some(fid);
        }
        let roots = [self.workspace_root.as_deref(), root_opt];
        for anchor in roots.into_iter().flatten() {
            let joined = anchor.join(p);
            if let Some(s) = joined.to_str() {
                if let Some(fid) = self.reader_find_file_id(reader_idx, s) {
                    return Some(fid);
                }
            }
            for base in roots.into_iter().flatten() {
                if let Some(s) = joined.strip_prefix(base).ok().and_then(|r| r.to_str()) {
                    if let Some(fid) = self.reader_find_file_id(reader_idx, s) {
                        return Some(fid);
                    }
                }
            }
        }
        None
    }

    /// Returns prioritized `(reader_idx, file_id)` pairs for `file` (library targets first).
    pub fn resolved_reader_file_ids(&self, file: &str) -> Vec<(usize, u32)> {
        let mut direct: Vec<(usize, u32)> = self.reader_indices_for_file(file).into_iter().filter_map(|i| self.target_file_id_for_reader(i, file).map(|fid| (i, fid))).collect();
        if !direct.is_empty() {
            direct.sort_by_key(|&(idx, _)| usize::from(!self.target_kinds.get(idx).copied().flatten().is_some_and(|k| k.is_lib())));
            return direct;
        }
        self.candidate_reader_indices_for_file(file).into_iter().filter_map(|i| self.target_file_id_for_reader(i, file).map(|fid| (i, fid))).collect()
    }

    /// Resolves `file` (absolute, relative to the workspace root, or relative to the reader's
    /// source root) to the path string the reader at `reader_idx` stores for it, or `None` when
    /// this reader does not index the file. Every string returned is accepted by that reader's
    /// exact `find_file_id`: the input is re-expressed in the reader's coordinates (relative to
    /// the workspace root or to the source root) and compared whole, never by suffix.
    pub fn target_file_for_reader<'a>(&self, reader_idx: usize, file: &'a str) -> Option<std::borrow::Cow<'a, str>> {
        let _ = self.readers.get(reader_idx)?;
        let p = Path::new(file);
        let p_clean = is_clean_posix_bytes(p.as_os_str().as_encoded_bytes());
        let root_opt = self.reader_root(reader_idx);
        let stored = |s: &str| self.reader_find_file_id(reader_idx, s).is_some();
        let owned = |s: &str| Some(std::borrow::Cow::Owned(s.to_string()));

        if p.is_absolute() {
            // C. Stored as an absolute path: the reader names this very file, whatever the roots.
            if stored(file) {
                return Some(std::borrow::Cow::Borrowed(file));
            }
            // Otherwise the file must lie under the reader's source root, else under the
            // workspace root, before it is re-expressed in the reader's coordinates.
            if let Some(root) = root_opt {
                let matches_root = if root.is_absolute() {
                    fast_path_starts_with(p, p_clean, root)
                } else if let Some(ws_root) = &self.workspace_root {
                    p.starts_with(ws_root.join(root))
                } else {
                    false
                };
                if !matches_root {
                    return None;
                }
            } else if let Some(ws_root) = &self.workspace_root {
                if !fast_path_starts_with(p, p_clean, ws_root) {
                    return None;
                }
            }
            // A. Relative to the workspace root (workspace crates).
            if let Some(ws_root) = &self.workspace_root {
                if let Some(s) = p.strip_prefix(ws_root).ok().and_then(|r| r.to_str()) {
                    if stored(s) {
                        return owned(s);
                    }
                }
            }
            // B. Relative to the source root (dependencies indexed from their package root).
            if let Some(root) = root_opt {
                if let Some(s) = p.strip_prefix(root).ok().and_then(|r| r.to_str()) {
                    if stored(s) {
                        return owned(s);
                    }
                }
            }
            return None;
        }

        // Relative input: the stored form itself; else the path anchored to the workspace root or
        // to the source root and re-expressed relative to each of them, which are the two stored
        // forms (`src/lib.rs` typed for the crate stored as `serde/src/lib.rs` becomes
        // `<ws>/serde/src/lib.rs`, then `serde/src/lib.rs`). A bare relative path that no root
        // anchors to a stored file is not resolved.
        if stored(file) {
            return Some(std::borrow::Cow::Borrowed(file));
        }
        let roots = [self.workspace_root.as_deref(), root_opt];
        for anchor in roots.into_iter().flatten() {
            let joined = anchor.join(p);
            if let Some(s) = joined.to_str() {
                if stored(s) {
                    return owned(s);
                }
            }
            for base in roots.into_iter().flatten() {
                if let Some(s) = joined.strip_prefix(base).ok().and_then(|r| r.to_str()) {
                    if stored(s) {
                        return owned(s);
                    }
                }
            }
        }
        // Readers that know the file only through their recorded hashes (an empty file table):
        // the sidecar records both forms, the absolute one is compared here.
        if let Some(hashes) = self.reader_file_hashes.get(reader_idx) {
            for anchor in roots.into_iter().flatten() {
                if let Some(s) = anchor.join(p).to_str() {
                    if hashes.contains_key(s) {
                        return owned(s);
                    }
                }
            }
        }
        None
    }

    /// Returns reader indices prioritized for querying items in `file`:
    /// matching readers first (ordered with library targets before tests/benchmarks),
    /// followed by any remaining workspace readers.
    pub fn candidate_reader_indices_for_file(&self, file: &str) -> Vec<usize> {
        let raw_candidates = self.reader_indices_for_file(file);
        let mut candidates: Vec<usize> = raw_candidates
            .into_iter()
            .filter(|&idx| {
                let is_non_lib = self.target_kinds.get(idx).copied().flatten().map(|k| !k.is_lib()).unwrap_or(false);
                if is_non_lib {
                    if self.readers[idx].files_count() > 0 {
                        self.target_file_for_reader(idx, file).is_some()
                    } else {
                        true
                    }
                } else {
                    true
                }
            })
            .collect();
        candidates.sort_by_key(|&idx| if self.target_kinds.get(idx).copied().flatten().map(|k| k.is_lib()).unwrap_or(false) { 0 } else { 1 });
        let mut non_dep_others = Vec::new();
        let mut sysroot_others = Vec::new();
        let mut dep_others = Vec::new();

        for idx in 0..self.readers.len() {
            if !candidates.contains(&idx) {
                let is_inaccessible = self.target_kinds.get(idx).copied().flatten().map(|k| !k.is_lib()).unwrap_or(false);
                if !is_inaccessible {
                    let is_sysroot = matches!(self.readers[idx].crate_name(), "core" | "alloc" | "std");
                    let is_dep = self.reader_is_dependency.get(idx).copied().unwrap_or(false);
                    if is_sysroot {
                        sysroot_others.push(idx);
                    } else if !is_dep {
                        non_dep_others.push(idx);
                    } else {
                        dep_others.push(idx);
                    }
                }
            }
        }
        candidates.extend(non_dep_others);
        candidates.extend(sysroot_others);
        candidates.extend(dep_others);
        candidates
    }
}
