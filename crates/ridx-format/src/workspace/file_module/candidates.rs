/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use crate::workspace::WorkspaceIndex;
use std::path::Path;

impl WorkspaceIndex {
    /// Resolves `file` (which may be absolute or relative) to the target file path
    /// recognized by the reader at `reader_idx` (as verified by `reader.find_file_id`),
    /// or `None` if this reader does not index the file.
    pub fn target_file_for_reader<'a>(
        &self,
        reader_idx: usize,
        file: &'a str,
    ) -> Option<std::borrow::Cow<'a, str>> {
        let reader = self.readers.get(reader_idx)?;

        let p = Path::new(file);
        if p.is_absolute() {
            // Absolute path validation: must be within source_root (if defined) or workspace_root
            let root_opt = self.source_roots.get(reader_idx).and_then(|r| r.as_deref());
            if let Some(root) = root_opt {
                let matches_root = if root.is_absolute() {
                    p.starts_with(root)
                } else if let Some(ws_root) = &self.workspace_root {
                    p.starts_with(ws_root.join(root))
                } else {
                    false
                };
                if !matches_root {
                    return None;
                }
            } else if let Some(ws_root) = &self.workspace_root {
                if !p.starts_with(ws_root) {
                    return None;
                }
            }

            // A. Workspace root relative (workspace members indexed from workspace root)
            if let Some(ws_root) = &self.workspace_root {
                if let Ok(rel) = p.strip_prefix(ws_root) {
                    if let Some(s) = rel.to_str() {
                        if reader.find_file_id(s).is_some() {
                            return Some(std::borrow::Cow::Owned(s.to_string()));
                        }
                    }
                }
            }

            // B. Source root relative (external dependencies indexed from package root)
            if let Some(root) = root_opt {
                if let Ok(rel) = p.strip_prefix(root) {
                    if let Some(s) = rel.to_str() {
                        if reader.find_file_id(s).is_some() {
                            return Some(std::borrow::Cow::Owned(s.to_string()));
                        }
                    }
                }
            }

            // C. Direct absolute match (if index stored absolute paths)
            if reader.find_file_id(file).is_some() {
                return Some(std::borrow::Cow::Borrowed(file));
            }
        } else {
            // Relative path
            let has_file = |s: &str| {
                reader.find_file_id(s).is_some()
                    || self
                        .reader_file_hashes
                        .get(reader_idx)
                        .is_some_and(|h| h.contains_key(s))
            };

            if let Some(ws_root) = &self.workspace_root {
                let joined = ws_root.join(p);
                if let Some(s) = joined.to_str() {
                    if has_file(s) {
                        return Some(std::borrow::Cow::Owned(s.to_string()));
                    }
                }
            }

            if let Some(Some(root)) = self.source_roots.get(reader_idx) {
                let joined = root.join(p);
                if let Some(s) = joined.to_str() {
                    if has_file(s) {
                        return Some(std::borrow::Cow::Owned(s.to_string()));
                    }
                }
                if let Ok(rel) = p.strip_prefix(root) {
                    if let Some(s) = rel.to_str() {
                        if reader.find_file_id(s).is_some() {
                            return Some(std::borrow::Cow::Owned(s.to_string()));
                        }
                    }
                }
            }

            if reader.find_file_id(file).is_some() {
                return Some(std::borrow::Cow::Borrowed(file));
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
                let is_non_lib = self
                    .target_kinds
                    .get(idx)
                    .copied()
                    .flatten()
                    .map(|k| !k.is_lib())
                    .unwrap_or(false);
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
        candidates.sort_by_key(|&idx| {
            if self
                .target_kinds
                .get(idx)
                .copied()
                .flatten()
                .map(|k| k.is_lib())
                .unwrap_or(false)
            {
                0
            } else {
                1
            }
        });
        let mut non_dep_others = Vec::new();
        let mut sysroot_others = Vec::new();
        let mut dep_others = Vec::new();

        for idx in 0..self.readers.len() {
            if !candidates.contains(&idx) {
                let is_inaccessible = self
                    .target_kinds
                    .get(idx)
                    .copied()
                    .flatten()
                    .map(|k| !k.is_lib())
                    .unwrap_or(false);
                if !is_inaccessible {
                    let is_sysroot =
                        matches!(self.readers[idx].crate_name(), "core" | "alloc" | "std");
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
