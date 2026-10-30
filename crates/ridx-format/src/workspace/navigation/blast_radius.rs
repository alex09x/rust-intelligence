/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Precomputed and on-demand direct reference blast-radius counters (`refs`, `files`, `crates`).
//!
//! Deduplicates files by normalized workspace-relative file path (so a file indexed in both `lib`
//! and `test` shards is counted once) and deduplicates crates by Cargo package root (so integration
//! test targets and build scripts in the same package do not inflate crate counts).

use super::reference_record::{is_raw_ident_keyword, matches_reference_target, ReferenceTarget};
use super::*;
use crate::format::fnv1a_64;
use std::collections::{HashMap, HashSet};
use std::path::Path;

const FIELD_KEY_SALT: u64 = 0x9e37_79b9_7f4a_7c15;
const MACRO_KEY_SALT: u64 = 0xbf58_476d_1ce4_e5b9;

/// Summary of direct references to a symbol across the workspace for enriched hover.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DirectBlastRadius {
    pub direct_refs: usize,
    pub files: usize,
    pub crates: usize,
}

#[derive(Clone, Copy)]
struct RawBlastEntry {
    key_hash: u64,
    loc_key: u64,
    file_id: u32,
    pkg_id: u16,
}

/// Precomputed `O(1)` lookup table mapping `(canonical_global_path, is_field)` hashes to
/// deduplicated `(direct_refs, files, crates)` counts.
#[derive(Debug, Clone, Default)]
pub struct BlastRadiusIndex {
    counts: HashMap<u64, DirectBlastRadius>,
}

#[inline]
fn fnv1a_64_concat(prefix: &str, sep: &str, suffix: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in prefix.as_bytes().iter().chain(sep.as_bytes()).chain(suffix.as_bytes()) {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[inline]
fn hash_with_kind(base_hash: u64, kind: DefKind) -> u64 {
    match kind {
        DefKind::Field => base_hash ^ FIELD_KEY_SALT,
        DefKind::Macro => base_hash ^ MACRO_KEY_SALT,
        _ => base_hash,
    }
}

/// Normalizes a shard file string to a canonical workspace-relative path so that the same file
/// indexed across multiple compilation targets (`lib`, `lib-test`, integration tests) shares one key.
pub(crate) fn normalize_workspace_file_key(ws: &WorkspaceIndex, reader_idx: usize, r_file: &str) -> String {
    let ws_root = ws.workspace_root.as_deref().or(ws.root_dir.as_deref());
    let sroot = ws.source_roots.get(reader_idx).and_then(|r| r.as_deref());
    let p = Path::new(r_file);
    if p.is_absolute() {
        if let Some(root) = ws_root {
            if let Ok(rel) = p.strip_prefix(root) {
                return rel.to_string_lossy().trim_start_matches("./").to_string();
            }
        }
        return r_file.to_string();
    }
    if let (Some(sr), Some(root)) = (sroot, ws_root) {
        if let Ok(sr_rel) = sr.strip_prefix(root) {
            if !sr_rel.as_os_str().is_empty() && !p.starts_with(sr_rel) {
                return sr_rel.join(p).to_string_lossy().trim_start_matches("./").to_string();
            }
        }
    }
    r_file.trim_start_matches("./").to_string()
}

/// Derives the Cargo package identifier for a normalized workspace file path, grouping `lib`,
/// unit tests, `build.rs`, examples, benches, and `tests/*.rs` integration targets under one package.
pub(crate) fn package_key_for_file(ws: &WorkspaceIndex, reader_idx: usize, norm_file: &str) -> String {
    for marker in ["/src/", "/tests/", "/benches/", "/examples/"] {
        if let Some(pos) = norm_file.find(marker) {
            if pos > 0 {
                return norm_file[..pos].to_string();
            }
        }
    }
    if let Some(prefix) = norm_file.strip_suffix("/build.rs") {
        if !prefix.is_empty() {
            return prefix.to_string();
        }
    }
    if norm_file.starts_with("src/") || norm_file.starts_with("tests/") || norm_file.starts_with("benches/") || norm_file.starts_with("examples/") || norm_file == "build.rs" {
        return "<root>".to_string();
    }
    if let Some(sr) = ws.source_roots.get(reader_idx).and_then(|r| r.as_deref()) {
        return sr.to_string_lossy().to_string();
    }
    let cname = ws.readers[reader_idx].crate_name();
    cname.strip_suffix("_test").unwrap_or(cname).to_string()
}

impl BlastRadiusIndex {
    /// Builds the precomputed blast-radius table across all non-external workspace shards.
    pub fn build(ws: &WorkspaceIndex) -> Self {
        let mut known_crates: HashSet<&str> = HashSet::with_capacity(ws.readers.len() * 2 + 8);
        for k in ["std", "core", "alloc", "proc_macro", "test"] {
            known_crates.insert(k);
        }
        for r in &ws.readers {
            known_crates.insert(r.crate_name());
        }
        for ext_map in &ws.reader_extern_units {
            for k in ext_map.keys() {
                known_crates.insert(k.as_str());
            }
        }

        let mut file_interner: HashMap<String, u32> = HashMap::new();
        let mut pkg_interner: HashMap<String, u16> = HashMap::new();
        let mut entries: Vec<RawBlastEntry> = Vec::new();

        for (i, reader) in ws.readers.iter().enumerate() {
            if ws.is_reader_external(i) {
                continue;
            }
            let fcount = reader.files_count();
            let mut file_map: Vec<(u32, u16)> = Vec::with_capacity(fcount as usize);
            for fid in 0..fcount {
                let raw_f = reader.get_file(fid);
                let norm_f = normalize_workspace_file_key(ws, i, raw_f);
                let pkg_k = package_key_for_file(ws, i, &norm_f);
                let next_fid = file_interner.len() as u32;
                let g_fid = *file_interner.entry(norm_f).or_insert(next_fid);
                let next_pid = pkg_interner.len() as u16;
                let g_pid = *pkg_interner.entry(pkg_k).or_insert(next_pid);
                file_map.push((g_fid, g_pid));
            }

            let cname = reader.crate_name();
            for r in reader.references() {
                let r_kind = DefKind::from(r.kind);
                let s = r.span();
                if r_kind != DefKind::Macro && s.start_line() != s.end_line() {
                    continue;
                }
                let raw_target = reader.get_str(r.target_path_offset());
                if raw_target.is_empty() {
                    continue;
                }
                let name = raw_target.rsplit("::").next().unwrap_or(raw_target);
                if r_kind != DefKind::Macro {
                    let width = s.end_col().saturating_sub(s.start_col());
                    let clean_name = name.strip_prefix("r#").unwrap_or(name);
                    let name_chars = clean_name.chars().count() as u32;
                    if r.is_macro_expansion() && width != name_chars && width != name.chars().count() as u32 {
                        continue;
                    }
                    if !r.is_macro_expansion() && width == name_chars + 2 && !is_raw_ident_keyword(clean_name) {
                        continue;
                    }
                }
                if !reader.is_name_reference(r) {
                    continue;
                }
                let Some(&(g_fid, g_pid)) = file_map.get(s.file_id() as usize) else {
                    continue;
                };
                let loc_key = ((g_fid as u64) << 36) | (((s.start_line() as u64) & 0xFFFFF) << 16) | (((s.start_col() as u64) & 0xFF) << 8) | ((s.end_col() as u64) & 0xFF);

                let has_crate_prefix = raw_target.strip_prefix(cname).and_then(|rest| rest.strip_prefix("::")).is_some();
                let base_hash = if has_crate_prefix {
                    fnv1a_64(raw_target.as_bytes())
                } else {
                    let first_seg = raw_target.split("::").next().unwrap_or(raw_target);
                    if r.target_def_id() == u32::MAX && raw_target.contains("::") && known_crates.contains(first_seg) {
                        fnv1a_64(raw_target.as_bytes())
                    } else {
                        fnv1a_64_concat(cname, "::", raw_target)
                    }
                };

                if r_kind == DefKind::Other {
                    for k in [DefKind::Struct, DefKind::Field] {
                        entries.push(RawBlastEntry { key_hash: hash_with_kind(base_hash, k), loc_key, file_id: g_fid, pkg_id: g_pid });
                    }
                } else {
                    entries.push(RawBlastEntry { key_hash: hash_with_kind(base_hash, r_kind), loc_key, file_id: g_fid, pkg_id: g_pid });
                }
            }
        }

        entries.sort_unstable_by_key(|e| (e.key_hash, e.loc_key));
        let mut counts = HashMap::new();
        let mut files_set: HashSet<u32> = HashSet::new();
        let mut pkgs_set: HashSet<u16> = HashSet::new();

        let mut idx = 0;
        while idx < entries.len() {
            let kh = entries[idx].key_hash;
            files_set.clear();
            pkgs_set.clear();
            let mut direct_refs = 0usize;
            let mut prev_loc = u64::MAX;
            while idx < entries.len() && entries[idx].key_hash == kh {
                let e = &entries[idx];
                if e.loc_key != prev_loc {
                    prev_loc = e.loc_key;
                    direct_refs += 1;
                    files_set.insert(e.file_id);
                    pkgs_set.insert(e.pkg_id);
                }
                idx += 1;
            }
            counts.insert(kh, DirectBlastRadius { direct_refs, files: files_set.len(), crates: pkgs_set.len() });
        }

        Self { counts }
    }

    #[inline]
    pub fn lookup(&self, global_path: &str, kind: DefKind) -> DirectBlastRadius {
        let kh = hash_with_kind(fnv1a_64(global_path.as_bytes()), kind);
        self.counts.get(&kh).copied().unwrap_or_default()
    }
}

impl WorkspaceIndex {
    /// Returns the direct-reference blast radius for `def` across workspace shards (`O(1)` via
    /// precomputed `BlastRadiusIndex`, falling back to `HashSet` scan when the index is unbuilt).
    pub fn direct_blast_radius(&self, def: &ResolvedDefinition<'_>) -> DirectBlastRadius {
        if def.path.is_empty() || def.kind == DefKind::Local {
            return DirectBlastRadius::default();
        }
        let target_crate = def.reader_idx.and_then(|idx| self.readers.get(idx)).map(|r| r.crate_name()).unwrap_or_else(|| def.path.split("::").next().unwrap_or(def.path));
        let local_path = def.path.strip_prefix(&format!("{target_crate}::")).unwrap_or(def.path);
        let global_path = format!("{target_crate}::{local_path}");
        if self.path_index.is_current(self) {
            return self.path_index.blast_radius.lookup(&global_path, def.kind);
        }

        let disambiguate_line = def.reader_idx.is_some_and(|t| {
            let r = &self.readers[t];
            r.has_multiple_definitions_at_path(def.path) || r.has_multiple_definitions_at_path(local_path) || r.has_multiple_definitions_at_path(&global_path)
        });

        let ref_target = ReferenceTarget {
            kind: Some(def.kind),
            name: Some(def.name),
            coords: (!def.file.is_empty() && def.start_line > 0).then(|| (def.file.to_string(), def.file.to_string(), def.start_line)),
            disambiguate_line,
        };

        let mut seen_refs: HashSet<(String, u32, u32, u32)> = HashSet::new();
        let mut files: HashSet<String> = HashSet::new();
        let mut crates: HashSet<String> = HashSet::new();

        for (i, reader) in self.readers.iter().enumerate() {
            if self.is_reader_external(i) {
                continue;
            }
            let cname = reader.crate_name();
            let queries: &[&str] = if cname == target_crate && local_path != global_path { &[local_path, global_path.as_str()] } else { &[global_path.as_str()] };
            for &q in queries {
                for r in reader.find_references(q) {
                    if !matches_reference_target(reader, r, &ref_target) {
                        continue;
                    }
                    let s = r.span();
                    let norm_f = normalize_workspace_file_key(self, i, reader.get_file(s.file_id()));
                    if seen_refs.insert((norm_f.clone(), s.start_line(), s.start_col(), s.end_col())) {
                        let pkg_k = package_key_for_file(self, i, &norm_f);
                        files.insert(norm_f);
                        crates.insert(pkg_k);
                    }
                }
            }
        }

        DirectBlastRadius { direct_refs: seen_refs.len(), files: files.len(), crates: crates.len() }
    }
}
