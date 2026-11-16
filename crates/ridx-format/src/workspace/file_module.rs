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

mod candidates;

#[inline]
pub(crate) fn is_clean_posix_bytes(b: &[u8]) -> bool {
    if b.is_empty() || b.starts_with(b"./") || b == b"." || b.ends_with(b"/.") || b.contains(&b'\\') {
        return false;
    }
    let mut i = 0;
    while i + 1 < b.len() {
        if b[i] == b'/' && (b[i + 1] == b'/' || b[i + 1] == b'.') {
            return false;
        }
        i += 1;
    }
    true
}

#[inline]
pub(crate) fn fast_path_starts_with(base: &Path, base_clean: bool, root: &Path) -> bool {
    let r = root.as_os_str().as_encoded_bytes();
    if base_clean && is_clean_posix_bytes(r) {
        let b = base.as_os_str().as_encoded_bytes();
        let r_trimmed = if r.len() > 1 && r.last() == Some(&b'/') { &r[..r.len() - 1] } else { r };
        if b.len() < r_trimmed.len() || !b.starts_with(r_trimmed) {
            return false;
        }
        return b.len() == r_trimmed.len() || r_trimmed == b"/" || b.get(r_trimmed.len()) == Some(&b'/');
    }
    base.starts_with(root)
}

impl WorkspaceIndex {
    /// Returns the index in `self.readers` for the crate that contains or owns the specified file path.
    pub fn reader_index_for_file(&self, file_path: &str) -> Option<usize> {
        let indices = self.reader_indices_for_file(file_path);
        self.reader_index_from_indices(&indices, file_path)
    }

    pub(crate) fn reader_index_from_indices(&self, indices: &[usize], file_path: &str) -> Option<usize> {
        let has_file = |i: usize| self.target_file_id_for_reader(i, file_path).is_some();
        let is_lib = |i: usize| {
            self.target_kinds.get(i).copied().flatten().is_some_and(|k| k.is_lib()) || self.unit_keys.get(i).and_then(|u| u.as_deref()).is_some_and(|uk| uk.contains("(lib)"))
        };
        if let Some(i) = indices.iter().copied().find(|&i| is_lib(i) && has_file(i)) {
            return Some(i);
        }
        if let Some(i) = indices.iter().copied().find(|&i| !is_lib(i) && has_file(i)) {
            return Some(i);
        }
        indices.iter().copied().find(|&i| is_lib(i)).or_else(|| indices.first().copied())
    }

    /// Whether `file_path` is compiled into a binary target (`src/main.rs`, a module of it, a `[[bin]]` path) and not
    /// into the package's library: its `crate::` is the binary, the library is another crate to it.
    pub fn file_in_bin_only(&self, file_path: &str) -> bool {
        let indices = self.reader_indices_for_file(file_path);
        let kind = |i: usize| self.target_kinds.get(i).copied().flatten();
        let has_file = |i: usize| self.target_file_id_for_reader(i, file_path).is_some();
        let in_lib = indices.iter().any(|&i| kind(i).is_some_and(|k| k.is_lib()) && has_file(i));
        let in_bin = indices.iter().any(|&i| kind(i) == Some(super::target_kind::TargetKind::Bin) && has_file(i));
        in_bin && !in_lib
    }

    /// Returns the target kind for the reader at index `idx` if known.
    pub fn target_kind_at(&self, idx: usize) -> Option<super::target_kind::TargetKind> {
        self.target_kinds.get(idx).copied().flatten()
    }

    /// The root a reader's relative stored paths are resolved against: its recorded source root,
    /// else, for a shard opened without any workspace root (a lone `.ridx` file), the directory
    /// the shard lies in. Readers of a rooted workspace that record no source root (the sysroot
    /// shards) have none.
    pub(crate) fn reader_root(&self, idx: usize) -> Option<&Path> {
        if let Some(Some(root)) = self.source_roots.get(idx) {
            return Some(root.as_path());
        }
        if self.workspace_root.is_none() {
            return self.paths.get(idx).and_then(|p| p.parent());
        }
        None
    }

    /// Returns the indices in `self.readers` for all crate units that contain or own the specified file path.
    /// Employs longest prefix matching on `source_roots` so that subpackage targets are returned
    /// rather than broader ancestor workspace roots, invariant to reader insertion order.
    pub fn reader_indices_for_file(&self, file_path: &str) -> Vec<usize> {
        let p = Path::new(file_path);
        let p_is_abs = p.is_absolute();
        let p_clean = is_clean_posix_bytes(p.as_os_str().as_encoded_bytes());
        let abs_p = if p_is_abs { None } else { self.workspace_root.as_ref().map(|ws| ws.join(p)) };
        let abs_p_clean = abs_p.as_ref().is_some_and(|ap| is_clean_posix_bytes(ap.as_os_str().as_encoded_bytes()));

        // 1. Longest prefix match against source_roots (fast lexical path, then canonical fallback)
        if let Some(top) = self.match_source_roots(p, file_path, p_clean, abs_p.as_deref(), abs_p_clean, self.workspace_root.as_deref(), false) {
            return top;
        }
        let canon_p_opt = if p_is_abs || abs_p.is_some() {
            let candidate = abs_p.as_deref().unwrap_or(p);
            let canon_p = crate::paths::canonicalize_cached(candidate);
            let canon_ws = self.workspace_root.as_deref().map(crate::paths::canonicalize_cached);
            if let Some(canon_str) = canon_p.to_str() {
                let canon_clean = is_clean_posix_bytes(canon_p.as_os_str().as_encoded_bytes());
                if let Some(top) = self.match_source_roots(&canon_p, canon_str, canon_clean, None, false, canon_ws.as_deref(), true) {
                    return top;
                }
            }
            Some(canon_p)
        } else {
            None
        };

        let abs_str = abs_p.as_deref().and_then(|ap| ap.to_str());

        // 2. Exact match for workspace-anchored path if relative
        if let Some(ap) = abs_str {
            let indices: Vec<usize> = self
                .readers
                .iter()
                .enumerate()
                .filter(|&(i, _)| self.reader_find_file_id(i, ap).is_some() || self.reader_file_hashes.get(i).is_some_and(|h| h.contains_key(ap)))
                .map(|(i, _)| i)
                .collect();
            if !indices.is_empty() {
                return indices;
            }
        }

        // 3. reader.find_file_id match (direct index match, including canonical alias)
        let canon_str = canon_p_opt.as_deref().and_then(|cp| cp.to_str()).filter(|&cs| cs != file_path);
        let indices: Vec<usize> = (0..self.readers.len())
            .filter(|&i| self.reader_find_file_id(i, file_path).is_some() || canon_str.is_some_and(|cs| self.reader_find_file_id(i, cs).is_some()))
            .collect();
        if !indices.is_empty() {
            return indices;
        }

        // 4. reader_file_hashes exact match
        let indices: Vec<usize> = self.reader_file_hashes.iter().enumerate().filter(|&(_, h)| h.contains_key(file_path)).map(|(i, _)| i).collect();
        if !indices.is_empty() {
            let cname_matches: Vec<usize> = indices
                .iter()
                .copied()
                .filter(|&i| {
                    let c = self.readers[i].crate_name();
                    file_path.contains(&format!("/{c}/")) || file_path.contains(&format!("\\{c}\\"))
                })
                .collect();
            return if cname_matches.is_empty() { indices } else { cname_matches };
        }

        // 5. Crate name contained in path
        let indices: Vec<usize> = self
            .readers
            .iter()
            .enumerate()
            .filter(|&(_, r)| {
                let c = r.crate_name();
                file_path.contains(&format!("/{c}/")) || file_path.contains(&format!("\\{c}\\"))
            })
            .map(|(i, _)| i)
            .collect();
        if !indices.is_empty() {
            return indices;
        }

        // 5. Fallback for single crate workspace
        if self.readers.len() == 1 {
            return vec![0];
        }

        // 6. Fallback for single non-sysroot crate
        let non_sysroot: Vec<usize> = self.readers.iter().enumerate().filter(|(_, r)| !matches!(r.crate_name(), "core" | "alloc" | "std")).map(|(i, _)| i).collect();
        if non_sysroot.len() == 1 {
            return non_sysroot;
        }

        Vec::new()
    }

    /// The names under which the compilation unit of `file_path` links other crates (its `extern` names), when the
    /// index records them; `None` when it does not.
    pub fn extern_crate_names_for_file(&self, file_path: &str) -> Option<Vec<&str>> {
        let idx = self.reader_index_for_file(file_path)?;
        let names = self.reader_extern_units.get(idx).filter(|m| !m.is_empty())?;
        Some(names.keys().map(String::as_str).collect())
    }

    /// The crate-relative module `file_path` is (`flags::complete::fish`, `""` for the crate root), read from the
    /// module-level items the index holds for it (functions, types, traits, consts, statics, modules): the shallowest
    /// of their parents, so an inline `mod tests { … }` of the file does not stand for it. `None` when the file
    /// declares no such item.
    pub fn module_of_file(&self, file_path: &str) -> Option<String> {
        use crate::format::DefKind;
        let idx = self.reader_index_for_file(file_path)?;
        let reader = &self.readers[idx];
        let cname = reader.crate_name();
        let fid = self.target_file_id_for_reader(idx, file_path)?;
        let defs = reader.definitions();
        let mut best: Option<String> = None;
        for &(_, _, def_idx) in self.unit_match.defs_in_file_up_to(idx, fid, u32::MAX) {
            let Some(def) = defs.get(def_idx as usize) else { continue };
            let module_level = matches!(
                DefKind::from(def.kind),
                DefKind::Fn | DefKind::Struct | DefKind::Enum | DefKind::Union | DefKind::Trait | DefKind::Const | DefKind::Static | DefKind::TyAlias | DefKind::Mod
            );
            if !module_level || def.is_macro_expansion() {
                continue;
            }
            let path = reader.get_str(def.path_offset());
            let rel = path.strip_prefix(cname).and_then(|r| r.strip_prefix("::")).unwrap_or(path);
            let parent = rel.rsplit_once("::").map_or("", |(p, _)| p);
            if parent.contains(['<', ' ']) {
                continue;
            }
            if best.as_deref().is_none_or(|b| parent.matches("::").count() + usize::from(!parent.is_empty()) < b.matches("::").count() + usize::from(!b.is_empty())) {
                best = Some(parent.to_string());
            }
        }
        best
    }

    /// Returns the canonical module path corresponding to `file_path` in the owning crate
    /// (e.g. `serde::de::impls` for `serde/src/de/impls.rs`).
    pub fn module_path_for_file(&self, file_path: &str) -> Option<String> {
        let idx = self.reader_index_for_file(file_path)?;
        self.module_path_for_reader(idx, file_path)
    }

    pub(crate) fn module_path_for_reader(&self, idx: usize, file_path: &str) -> Option<String> {
        let reader = &self.readers[idx];
        let cname = reader.crate_name();

        // 1. Inspect definitions declared directly in this file; a reader that does not store
        //    the file contributes nothing here and the path-based fallback below decides.
        if let Some(fid) = self.target_file_id_for_reader(idx, file_path) {
            let defs = reader.definitions();
            for &(_, _, def_idx) in self.unit_match.defs_in_file_up_to(idx, fid, u32::MAX) {
                let Some(def) = defs.get(def_idx as usize) else {
                    continue;
                };
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

    pub(crate) fn enclosing_inline_mod_path(&self, file: &str, line: u32, col: u32) -> Option<&str> {
        for i in self.reader_indices_for_file(file) {
            let reader = &self.readers[i];
            let Some(tf) = self.target_file_for_reader(i, file) else {
                continue;
            };
            let Some(fid) = reader.find_file_id(tf.as_ref()) else {
                continue;
            };
            let defs = reader.definitions();
            let mut best: Option<(&str, u64)> = None;
            for &(_, _, di) in self.unit_match.defs_in_file_up_to(i, fid, line) {
                let Some(def) = defs.get(di as usize) else {
                    continue;
                };
                if crate::format::DefKind::from(def.kind) != crate::format::DefKind::Mod {
                    continue;
                }
                let span = def.span();
                let path = reader.get_str(def.path_offset());
                if !path.is_empty() && span.contains(fid, line, col) {
                    let sz = span.approximate_size();
                    if best.is_none_or(|(_, bsz)| sz < bsz) {
                        best = Some((path, sz));
                    }
                }
            }
            if let Some((p, _)) = best {
                return Some(p);
            }
        }
        None
    }
}
