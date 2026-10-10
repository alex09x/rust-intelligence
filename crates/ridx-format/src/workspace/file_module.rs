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
        // 1. Prefer an explicit library target kind (Lib)
        indices
            .iter()
            .copied()
            .find(|&i| {
                self.target_kinds
                    .get(i)
                    .copied()
                    .flatten()
                    .map(|k| k.is_lib())
                    .unwrap_or(false)
            })
            // 2. Fallback to unit_key containing "(lib)"
            .or_else(|| {
                indices.iter().copied().find(|&i| {
                    self.unit_keys
                        .get(i)
                        .and_then(|u| u.as_deref())
                        .map(|uk| uk.contains("(lib)"))
                        .unwrap_or(false)
                })
            })
            // 3. Fallback to first matching reader index
            .or_else(|| indices.into_iter().next())
    }

    /// Returns the target kind for the reader at index `idx` if known.
    pub fn target_kind_at(&self, idx: usize) -> Option<super::target_kind::TargetKind> {
        self.target_kinds.get(idx).copied().flatten()
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
                        || abs_p
                            .as_deref()
                            .map(|ap| ap.starts_with(root))
                            .unwrap_or(false)
                        || (p.is_absolute()
                            && self
                                .workspace_root
                                .as_ref()
                                .map(|ws| p.starts_with(ws.join(root)))
                                .unwrap_or(false));
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

        // 2. reader.find_file_id match (direct index match)
        let mut indices = Vec::new();
        for (i, reader) in self.readers.iter().enumerate() {
            if reader.find_file_id(file_path).is_some() {
                indices.push(i);
            }
        }
        if !indices.is_empty() {
            return indices;
        }

        // 3. reader_file_hashes exact match
        for (i, hashes) in self.reader_file_hashes.iter().enumerate() {
            if hashes.contains_key(file_path) {
                indices.push(i);
            }
        }
        if !indices.is_empty() {
            let cname_matches: Vec<usize> = indices
                .iter()
                .copied()
                .filter(|&i| {
                    let cname = self.readers[i].crate_name();
                    file_path.contains(&format!("/{cname}/"))
                        || file_path.contains(&format!("\\{cname}\\"))
                })
                .collect();
            if !cname_matches.is_empty() {
                return cname_matches;
            }
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
        let target_file = self.target_file_for_reader(idx, file_path);
        let target_str = target_file.as_deref().unwrap_or(file_path);
        if let Some(fid) = reader.find_file_id(target_str) {
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
            if reader.find_file_id(file).is_some() {
                return Some(std::borrow::Cow::Borrowed(file));
            }

            if let Some(Some(root)) = self.source_roots.get(reader_idx) {
                let joined = root.join(p);
                if let Some(s) = joined.to_str() {
                    if reader.find_file_id(s).is_some() {
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
        }

        None
    }

    /// Returns reader indices prioritized for querying items in `file`:
    /// matching readers first (ordered with library targets before tests/benchmarks),
    /// followed by any remaining workspace readers.
    pub fn candidate_reader_indices_for_file(&self, file: &str) -> Vec<usize> {
        let mut candidates = self.reader_indices_for_file(file);
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
        for idx in 0..self.readers.len() {
            if !candidates.contains(&idx) {
                candidates.push(idx);
            }
        }
        candidates
    }
}
