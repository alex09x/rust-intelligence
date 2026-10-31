/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Precomputed indices for cross-unit and cross-crate reference matching.
//!
//! Provides O(1) unit matching across workspace crates, avoiding quadratic scans
//! over readers and extern unit mappings.

use crate::format::{DefEntry, DefKind};
use crate::reader::{IndexReader, ResolvedDefinition};
use std::collections::{HashMap, HashSet};

/// Precomputed lookup index for workspace unit matching and file-scoped definition positions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnitMatchIndex {
    reader_fingerprints: Vec<(u32, u32, u64, u32)>,
    crate_version_counts: HashMap<String, usize>,
    crate_defined_units: HashMap<String, HashSet<String>>,
    crate_all_units: HashMap<String, HashSet<String>>,
    unit_to_readers: HashMap<String, Vec<usize>>,
    crate_to_readers: HashMap<String, Vec<usize>>,
    file_to_readers: HashMap<String, Vec<(usize, u32)>>,
    reader_has_special_files: Vec<bool>,
    defs_by_pos: Vec<Vec<(u32, u32, u32)>>,
    assoc_ty_defs: Vec<Vec<u32>>,
    reachable_reader_bits: Vec<Vec<u64>>,
    dep_graph_counts: Vec<(usize, usize)>,
}

#[inline]
fn reader_fingerprint<B: AsRef<[u8]>>(r: &IndexReader<B>) -> (u32, u32, u64, u32) {
    let h = r.header();
    (h.files_count, h.defs_count, h.string_pool_len, h.crate_name_offset)
}

impl UnitMatchIndex {
    /// Builds a precomputed `UnitMatchIndex` from the workspace readers and metadata.
    pub fn build<B: AsRef<[u8]>>(readers: &[IndexReader<B>], unit_keys: &[Option<String>], reader_extern_units: &[HashMap<String, String>]) -> Self {
        Self::build_with_deps(readers, unit_keys, reader_extern_units, &[])
    }

    /// Builds a precomputed `UnitMatchIndex` including 6-hop dependency reachability bitsets.
    pub fn build_with_deps<B: AsRef<[u8]>>(
        readers: &[IndexReader<B>],
        unit_keys: &[Option<String>],
        reader_extern_units: &[HashMap<String, String>],
        reader_dependencies: &[Vec<String>],
    ) -> Self {
        let mut reader_fingerprints = Vec::with_capacity(readers.len());
        let mut versions_by_crate: HashMap<String, HashSet<String>> = HashMap::new();
        let mut crate_defined_units: HashMap<String, HashSet<String>> = HashMap::new();
        let mut crate_all_units: HashMap<String, HashSet<String>> = HashMap::new();
        let mut unit_to_readers: HashMap<String, Vec<usize>> = HashMap::new();
        let mut crate_to_readers: HashMap<String, Vec<usize>> = HashMap::new();
        let mut file_to_readers: HashMap<String, Vec<(usize, u32)>> = HashMap::with_capacity(readers.len() * 2);
        let mut reader_has_special_files: Vec<bool> = Vec::with_capacity(readers.len());
        let mut defs_by_pos: Vec<Vec<(u32, u32, u32)>> = Vec::with_capacity(readers.len());
        let mut assoc_ty_defs: Vec<Vec<u32>> = Vec::with_capacity(readers.len());
        let mut resolved_uks: Vec<Option<String>> = Vec::with_capacity(readers.len());

        for (j, r) in readers.iter().enumerate() {
            reader_fingerprints.push(reader_fingerprint(r));
            let crate_name = r.crate_name();
            crate_to_readers.entry(crate_name.to_string()).or_default().push(j);
            let ver = r.crate_version();
            if !ver.is_empty() {
                versions_by_crate.entry(crate_name.to_string()).or_default().insert(ver.to_string());
            }
            let uk_opt = unit_keys.get(j).and_then(|u| u.as_deref()).filter(|s| !s.is_empty()).or_else(|| Some(r.unit_key()).filter(|s| !s.is_empty()));
            if let Some(uk) = uk_opt {
                crate_defined_units.entry(crate_name.to_string()).or_default().insert(uk.to_string());
                crate_all_units.entry(crate_name.to_string()).or_default().insert(uk.to_string());
                unit_to_readers.entry(uk.to_string()).or_default().push(j);
            }
            resolved_uks.push(uk_opt.map(str::to_string));
            let mut has_special = false;
            for fid in 0..r.files_count() {
                let f = r.get_file(fid);
                if f.contains("..") || f.contains("//") || f.contains("./") || f.contains("/.") || f.contains('\\') || f.ends_with('/') {
                    has_special = true;
                }
                let clean = f.strip_prefix("./").unwrap_or(f);
                let vec = file_to_readers.entry(clean.to_string()).or_default();
                if vec.last().is_none_or(|&(last_r, _)| last_r != j) {
                    vec.push((j, fid));
                }
            }
            reader_has_special_files.push(has_special);
            let defs = r.definitions();
            let mut pos_entries: Vec<(u32, u32, u32)> = Vec::with_capacity(defs.len());
            let mut atys: Vec<u32> = Vec::new();
            for (idx, d) in defs.iter().enumerate() {
                let s = d.span();
                pos_entries.push((s.file_id(), s.start_line(), idx as u32));
                if d.kind == DefKind::AssocTy as u8 && !d.is_macro_expansion() {
                    atys.push(idx as u32);
                }
            }
            pos_entries.sort_by_key(|&(fid, sl, _)| (fid, sl));
            defs_by_pos.push(pos_entries);
            assoc_ty_defs.push(atys);
        }
        for m in reader_extern_units {
            for (crate_name, uk) in m {
                crate_all_units.entry(crate_name.clone()).or_default().insert(uk.clone());
            }
        }
        let crate_version_counts = versions_by_crate.into_iter().map(|(k, v)| (k, v.len())).collect();
        let n = readers.len();
        let words = n.div_ceil(64);
        let mut reachable_reader_bits = Vec::with_capacity(n);
        let mut dep_graph_counts = Vec::with_capacity(n);
        for sr in 0..n {
            dep_graph_counts.push((reader_extern_units.get(sr).map_or(0, HashMap::len), reader_dependencies.get(sr).map_or(0, Vec::len)));
            let mut visited = vec![false; n];
            visited[sr] = true;
            let mut frontier = vec![sr];
            let mut reachable_uks: HashSet<&str> = HashSet::new();
            for _ in 0..6 {
                if frontier.is_empty() {
                    break;
                }
                let mut next_frontier = Vec::new();
                for cur in frontier {
                    for uk in reader_extern_units
                        .get(cur)
                        .into_iter()
                        .flat_map(|m| m.values().map(String::as_str))
                        .chain(reader_dependencies.get(cur).into_iter().flat_map(|d| d.iter().map(String::as_str)))
                    {
                        reachable_uks.insert(uk);
                        for &mid_idx in unit_to_readers.get(uk).map(Vec::as_slice).unwrap_or(&[]) {
                            if !visited[mid_idx] {
                                visited[mid_idx] = true;
                                next_frontier.push(mid_idx);
                            }
                        }
                    }
                }
                frontier = next_frontier;
            }
            let mut bits = vec![0u64; words];
            bits[sr >> 6] |= 1u64 << (sr & 63);
            for (t, uk_opt) in resolved_uks.iter().enumerate() {
                if uk_opt.as_deref().is_some_and(|uk| reachable_uks.contains(uk)) {
                    bits[t >> 6] |= 1u64 << (t & 63);
                }
            }
            reachable_reader_bits.push(bits);
        }
        Self {
            reader_fingerprints,
            crate_version_counts,
            crate_defined_units,
            crate_all_units,
            unit_to_readers,
            crate_to_readers,
            file_to_readers,
            reader_has_special_files,
            defs_by_pos,
            assoc_ty_defs,
            reachable_reader_bits,
            dep_graph_counts,
        }
    }

    #[inline]
    pub fn assoc_ty_defs(&self, reader_idx: usize) -> &[u32] {
        self.assoc_ty_defs.get(reader_idx).map(Vec::as_slice).unwrap_or(&[])
    }

    #[inline]
    pub fn is_valid_for_readers<B: AsRef<[u8]>>(&self, readers: &[IndexReader<B>]) -> bool {
        self.reader_fingerprints.len() == readers.len() && self.reader_fingerprints.iter().zip(readers.iter()).all(|(&fp, r)| fp == reader_fingerprint(r))
    }

    #[inline]
    pub fn reader_has_special(&self, reader_idx: usize) -> bool {
        self.reader_has_special_files.get(reader_idx).copied().unwrap_or(true)
    }

    #[inline]
    pub fn readers_for_clean_file(&self, file_path: &str) -> Option<&[(usize, u32)]> {
        let clean = file_path.strip_prefix("./").unwrap_or(file_path);
        if clean.contains("//") || clean.contains("/.") || clean.contains('\\') || clean.ends_with('/') {
            return None;
        }
        Some(self.file_to_readers.get(clean).map(Vec::as_slice).unwrap_or(&[]))
    }

    #[inline]
    pub fn find_file_id_fast<B: AsRef<[u8]>>(&self, reader_idx: usize, reader: &IndexReader<B>, file_path: &str) -> Option<Option<u32>> {
        if self.reader_fingerprints.get(reader_idx).copied() != Some(reader_fingerprint(reader)) || *self.reader_has_special_files.get(reader_idx)? {
            return None;
        }
        let clean = file_path.strip_prefix("./").unwrap_or(file_path);
        if clean.contains("//") || clean.contains("/.") || clean.contains('\\') || clean.ends_with('/') {
            return None;
        }
        let Some(list) = self.file_to_readers.get(clean) else {
            return Some(None);
        };
        Some(list.binary_search_by_key(&reader_idx, |&(r, _)| r).ok().map(|pos| list[pos].1))
    }

    #[inline]
    pub fn matches_unit(&self, reader_idx: usize, ref_target_unit: &str, target_unit: &str, target_crate: &str, reader_extern_units: &[HashMap<String, String>]) -> bool {
        if ref_target_unit == target_unit {
            return true;
        }
        if !self.crate_all_units.get(target_crate).is_some_and(|units| units.contains(target_unit)) || *self.crate_version_counts.get(target_crate).unwrap_or(&0) > 1 {
            return false;
        }
        self.crate_defined_units.get(target_crate).is_some_and(|units| units.contains(ref_target_unit))
            || reader_extern_units.get(reader_idx).and_then(|m| m.get(target_crate)).map(String::as_str) == Some(ref_target_unit)
    }

    #[inline]
    pub fn crate_versions_count(&self, crate_name: &str) -> usize {
        *self.crate_version_counts.get(crate_name).unwrap_or(&0)
    }

    #[inline]
    pub fn is_unit_in_crate(&self, crate_name: &str, unit_key: &str) -> bool {
        self.crate_all_units.get(crate_name).is_some_and(|units| units.contains(unit_key))
    }

    #[inline]
    pub fn is_crate_defined_unit(&self, crate_name: &str, unit_key: &str) -> bool {
        self.crate_defined_units.get(crate_name).is_some_and(|units| units.contains(unit_key))
    }

    #[inline]
    pub fn readers_for_unit(&self, unit_key: &str) -> Option<&[usize]> {
        self.unit_to_readers.get(unit_key).map(Vec::as_slice)
    }

    #[inline]
    pub fn readers_for_crate(&self, crate_name: &str) -> Option<&[usize]> {
        self.crate_to_readers.get(crate_name).map(Vec::as_slice)
    }

    #[inline]
    pub fn defs_in_file_up_to(&self, reader_idx: usize, file_id: u32, max_line: u32) -> &[(u32, u32, u32)] {
        let Some(table) = self.defs_by_pos.get(reader_idx) else {
            return &[];
        };
        let lo = table.partition_point(|&(fid, _, _)| fid < file_id);
        let hi = table.partition_point(|&(fid, sl, _)| (fid, sl) <= (file_id, max_line));
        table.get(lo..hi).unwrap_or(&[])
    }
}

impl super::WorkspaceIndex {
    /// Looks up `file_path` in `self.readers[reader_idx]` using the O(1) precomputed table when valid.
    #[inline]
    pub(crate) fn reader_find_file_id(&self, reader_idx: usize, file_path: &str) -> Option<u32> {
        let reader = self.readers.get(reader_idx)?;
        if let Some(res) = self.unit_match.find_file_id_fast(reader_idx, reader, file_path) {
            return res;
        }
        reader.find_file_id(file_path)
    }

    /// Returns reader indices matching `crate_name` in ascending order using O(1) lookup when valid.
    #[inline]
    pub(crate) fn readers_for_crate_fast(&self, crate_name: &str) -> std::borrow::Cow<'_, [usize]> {
        if self.unit_match.is_valid_for_readers(&self.readers) {
            return std::borrow::Cow::Borrowed(self.unit_match.readers_for_crate(crate_name).unwrap_or(&[]));
        }
        std::borrow::Cow::Owned(self.readers.iter().enumerate().filter(|(_, r)| r.crate_name() == crate_name).map(|(i, _)| i).collect())
    }

    /// Returns reader indices matching `unit_key` in ascending order using O(1) lookup when valid.
    #[inline]
    pub(crate) fn readers_for_unit_fast(&self, unit_key: &str) -> std::borrow::Cow<'_, [usize]> {
        if self.unit_match.is_valid_for_readers(&self.readers) && self.unit_keys.len() == self.readers.len() {
            if let Some(indices) = self.unit_match.readers_for_unit(unit_key) {
                if indices.iter().all(|&j| self.unit_keys.get(j).and_then(|u| u.as_deref()) == Some(unit_key)) {
                    return std::borrow::Cow::Borrowed(indices);
                }
            } else {
                return std::borrow::Cow::Borrowed(&[]);
            }
        }
        std::borrow::Cow::Owned(self.unit_keys.iter().enumerate().filter(|(_, u)| u.as_deref() == Some(unit_key)).map(|(i, _)| i).collect())
    }
    /// Returns true if `target_reader_idx` is directly or transitively (up to 6 hops)
    /// referenced in `extern_units` or `dependencies` of any reader in `source_readers`.
    #[inline]
    pub fn is_reader_in_dep_graph(&self, source_readers: &[usize], target_reader_idx: usize) -> bool {
        if source_readers.contains(&target_reader_idx) {
            return true;
        }
        if self.unit_match.dep_graph_counts.len() == self.readers.len()
            && source_readers.iter().all(|&sr| {
                self.unit_match.dep_graph_counts.get(sr).copied()
                    == Some((self.reader_extern_units.get(sr).map_or(0, HashMap::len), self.reader_dependencies.get(sr).map_or(0, Vec::len)))
            })
        {
            return source_readers.iter().any(|&sr| {
                self.unit_match.reachable_reader_bits.get(sr).and_then(|bits| bits.get(target_reader_idx >> 6)).is_some_and(|&w| (w & (1u64 << (target_reader_idx & 63))) != 0)
            });
        }
        let target_uk = self
            .unit_keys
            .get(target_reader_idx)
            .and_then(|u| u.as_deref())
            .filter(|s| !s.is_empty())
            .or_else(|| self.readers.get(target_reader_idx).map(|r| r.unit_key()).filter(|s| !s.is_empty()));
        let Some(target_uk) = target_uk else {
            return false;
        };
        let reader_has_dep = |r_idx: usize, uk: &str| -> bool {
            self.reader_extern_units.get(r_idx).is_some_and(|m| m.values().any(|v| v == uk)) || self.reader_dependencies.get(r_idx).is_some_and(|d| d.iter().any(|v| v == uk))
        };
        let mut visited: HashSet<usize> = source_readers.iter().copied().collect();
        let mut frontier: Vec<usize> = source_readers.to_vec();
        for _ in 0..6 {
            if frontier.is_empty() {
                break;
            }
            let mut next_frontier = Vec::new();
            for sr in frontier {
                if reader_has_dep(sr, target_uk) {
                    return true;
                }
                for mid_uk in self
                    .reader_extern_units
                    .get(sr)
                    .into_iter()
                    .flat_map(|m| m.values().map(String::as_str))
                    .chain(self.reader_dependencies.get(sr).into_iter().flat_map(|d| d.iter().map(String::as_str)))
                {
                    for &mid_idx in self.unit_match.readers_for_unit(mid_uk).unwrap_or(&[]) {
                        if visited.insert(mid_idx) {
                            next_frontier.push(mid_idx);
                        }
                    }
                }
            }
            frontier = next_frontier;
        }
        false
    }

    /// Finds the nearest preceding `Impl`, `AssocFn`, `AssocTy`, `AssocConst`, or `Trait`
    /// definition in `(reader_idx, file_id)` at or before `line` via binary search on `defs_by_pos`.
    pub fn find_enclosing_impl_or_trait_def(&self, reader_idx: usize, file_id: u32, line: u32) -> Option<&DefEntry> {
        let reader = self.readers.get(reader_idx)?;
        let defs = reader.definitions();
        let slice = self.unit_match.defs_in_file_up_to(reader_idx, file_id, line);
        for &(_, _, def_idx) in slice.iter().rev() {
            if let Some(d) = defs.get(def_idx as usize) {
                let k = DefKind::from(d.kind);
                if matches!(k, DefKind::Impl | DefKind::AssocFn | DefKind::AssocTy | DefKind::AssocConst | DefKind::Trait) {
                    return Some(d);
                }
            }
        }
        None
    }

    /// Finds a generic type parameter (`DefKind::TyParam`) in `(reader_idx, file_id)` matching `name`
    /// whose enclosing item span contains `(line, col)` via binary search on `defs_by_pos`.
    pub fn find_type_param_in_reader<'a>(&'a self, reader_idx: usize, file_id: u32, name: &str, line: u32, col: u32) -> Option<ResolvedDefinition<'a>> {
        let reader = self.readers.get(reader_idx)?;
        let defs = reader.definitions();
        let slice = self.unit_match.defs_in_file_up_to(reader_idx, file_id, u32::MAX);
        let mut best: Option<(&DefEntry, u64)> = None;
        for &(_, _, def_idx) in slice {
            let Some(def) = defs.get(def_idx as usize) else {
                continue;
            };
            if def.kind != DefKind::TyParam as u8 || reader.get_str(def.name_offset()) != name {
                continue;
            }
            if let Some(size) = reader.check_type_param_scope(file_id, def_idx, def, line, col) {
                if best.is_none_or(|(_, best_size)| size < best_size) {
                    best = Some((def, size));
                }
            }
        }
        best.map(|(def, _)| self.make_resolved_definition(reader_idx, def))
    }
}
