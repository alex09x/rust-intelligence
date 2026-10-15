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

use crate::format::{DefEntry, DefKind, RefEntry};
use crate::reader::{IndexReader, ResolvedDefinition};
use std::collections::{HashMap, HashSet};

/// Precomputed lookup index for workspace unit matching and file-scoped definition positions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnitMatchIndex {
    /// Distinct non-empty `crate_version` count per `crate_name`.
    crate_version_counts: HashMap<String, usize>,
    /// Units defined directly by workspace readers for `crate_name`.
    crate_defined_units: HashMap<String, HashSet<String>>,
    /// All units (defined or referenced via extern_units) associated with `crate_name`.
    crate_all_units: HashMap<String, HashSet<String>>,
    /// Map from `unit_key` to reader indices defining that unit.
    unit_to_readers: HashMap<String, Vec<usize>>,
    /// Per-reader definitions sorted stably by `(file_id, start_line)` as `(file_id, start_line, def_idx)`.
    defs_by_pos: Vec<Vec<(u32, u32, u32)>>,
}

impl UnitMatchIndex {
    /// Builds a precomputed `UnitMatchIndex` from the workspace readers and metadata.
    pub fn build<B: AsRef<[u8]>>(
        readers: &[IndexReader<B>],
        unit_keys: &[Option<String>],
        reader_extern_units: &[HashMap<String, String>],
    ) -> Self {
        let mut versions_by_crate: HashMap<String, HashSet<String>> = HashMap::new();
        let mut crate_defined_units: HashMap<String, HashSet<String>> = HashMap::new();
        let mut crate_all_units: HashMap<String, HashSet<String>> = HashMap::new();
        let mut unit_to_readers: HashMap<String, Vec<usize>> = HashMap::new();
        let mut defs_by_pos: Vec<Vec<(u32, u32, u32)>> = Vec::with_capacity(readers.len());

        for (j, r) in readers.iter().enumerate() {
            let crate_name = r.crate_name();
            let ver = r.crate_version();
            if !ver.is_empty() {
                versions_by_crate
                    .entry(crate_name.to_string())
                    .or_default()
                    .insert(ver.to_string());
            }
            let uk_opt = unit_keys
                .get(j)
                .and_then(|u| u.as_deref())
                .filter(|s| !s.is_empty())
                .or_else(|| Some(r.unit_key()).filter(|s| !s.is_empty()));
            if let Some(uk) = uk_opt {
                crate_defined_units
                    .entry(crate_name.to_string())
                    .or_default()
                    .insert(uk.to_string());
                crate_all_units
                    .entry(crate_name.to_string())
                    .or_default()
                    .insert(uk.to_string());
                unit_to_readers.entry(uk.to_string()).or_default().push(j);
            }
            let defs = r.definitions();
            let mut pos_entries: Vec<(u32, u32, u32)> = defs
                .iter()
                .enumerate()
                .map(|(idx, d)| {
                    let s = d.span();
                    (s.file_id(), s.start_line(), idx as u32)
                })
                .collect();
            pos_entries.sort_by_key(|&(fid, sl, _)| (fid, sl));
            defs_by_pos.push(pos_entries);
        }

        for m in reader_extern_units {
            for (crate_name, uk) in m {
                crate_all_units
                    .entry(crate_name.clone())
                    .or_default()
                    .insert(uk.clone());
            }
        }

        let crate_version_counts = versions_by_crate
            .into_iter()
            .map(|(k, v)| (k, v.len()))
            .collect();

        Self {
            crate_version_counts,
            crate_defined_units,
            crate_all_units,
            unit_to_readers,
            defs_by_pos,
        }
    }

    /// Checks whether `ref_target_unit` matches `target_unit` for `target_crate`.
    ///
    /// Handles exact unit match, as well as single-version workspaces where Cargo
    /// assigned different unit hashes (e.g. lib vs test vs dep compilation).
    ///
    /// Evaluates in O(1) time without allocations.
    #[inline]
    pub fn matches_unit(
        &self,
        reader_idx: usize,
        ref_target_unit: &str,
        target_unit: &str,
        target_crate: &str,
        reader_extern_units: &[HashMap<String, String>],
    ) -> bool {
        if ref_target_unit == target_unit {
            return true;
        }

        let target_unit_in_crate = self
            .crate_all_units
            .get(target_crate)
            .map(|units| units.contains(target_unit))
            .unwrap_or(false);
        if !target_unit_in_crate {
            return false;
        }

        let target_versions_count = *self.crate_version_counts.get(target_crate).unwrap_or(&0);
        if target_versions_count > 1 {
            return false;
        }

        let in_defined_units = self
            .crate_defined_units
            .get(target_crate)
            .map(|units| units.contains(ref_target_unit))
            .unwrap_or(false);
        if in_defined_units {
            return true;
        }

        reader_extern_units
            .get(reader_idx)
            .and_then(|m| m.get(target_crate))
            .map(|u| u.as_str())
            == Some(ref_target_unit)
    }

    /// Number of distinct non-empty versions for `crate_name`.
    #[inline]
    pub fn crate_versions_count(&self, crate_name: &str) -> usize {
        *self.crate_version_counts.get(crate_name).unwrap_or(&0)
    }

    /// Whether `unit_key` is associated with `crate_name` (defined or extern).
    #[inline]
    pub fn is_unit_in_crate(&self, crate_name: &str, unit_key: &str) -> bool {
        self.crate_all_units
            .get(crate_name)
            .map(|units| units.contains(unit_key))
            .unwrap_or(false)
    }

    /// Whether `unit_key` is directly defined by a workspace reader for `crate_name`.
    #[inline]
    pub fn is_crate_defined_unit(&self, crate_name: &str, unit_key: &str) -> bool {
        self.crate_defined_units
            .get(crate_name)
            .map(|units| units.contains(unit_key))
            .unwrap_or(false)
    }

    /// Reader indices defining `unit_key`.
    #[inline]
    pub fn readers_for_unit(&self, unit_key: &str) -> Option<&[usize]> {
        self.unit_to_readers.get(unit_key).map(Vec::as_slice)
    }

    /// Position-sorted definition entries `(file_id, start_line, def_idx)` for `reader_idx` in `file_id` up to `max_line`.
    #[inline]
    pub fn defs_in_file_up_to(
        &self,
        reader_idx: usize,
        file_id: u32,
        max_line: u32,
    ) -> &[(u32, u32, u32)] {
        let Some(table) = self.defs_by_pos.get(reader_idx) else {
            return &[];
        };
        let lo = table.partition_point(|&(fid, _, _)| fid < file_id);
        let hi = table.partition_point(|&(fid, sl, _)| (fid, sl) <= (file_id, max_line));
        if lo <= hi {
            &table[lo..hi]
        } else {
            &[]
        }
    }
}

impl super::WorkspaceIndex {
    /// Returns true if `target_reader_idx` is directly or transitively (up to 2 hops)
    /// referenced in `extern_units` or `dependencies` of any reader in `source_readers`.
    pub fn is_reader_in_dep_graph(
        &self,
        source_readers: &[usize],
        target_reader_idx: usize,
    ) -> bool {
        if source_readers.contains(&target_reader_idx) {
            return true;
        }
        let target_uk = self
            .unit_keys
            .get(target_reader_idx)
            .and_then(|u| u.as_deref())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                self.readers
                    .get(target_reader_idx)
                    .map(|r| r.unit_key())
                    .filter(|s| !s.is_empty())
            });
        let Some(target_uk) = target_uk else {
            return false;
        };
        let reader_has_dep = |r_idx: usize, uk: &str| -> bool {
            self.reader_extern_units
                .get(r_idx)
                .is_some_and(|m| m.values().any(|v| v == uk))
                || self
                    .reader_dependencies
                    .get(r_idx)
                    .is_some_and(|d| d.iter().any(|v| v == uk))
        };
        for &sr in source_readers {
            if reader_has_dep(sr, target_uk) {
                return true;
            }
            let mid_uks = self
                .reader_extern_units
                .get(sr)
                .into_iter()
                .flat_map(|m| m.values().map(String::as_str))
                .chain(
                    self.reader_dependencies
                        .get(sr)
                        .into_iter()
                        .flat_map(|d| d.iter().map(String::as_str)),
                );
            for mid_uk in mid_uks {
                if let Some(mid_indices) = self.unit_match.readers_for_unit(mid_uk) {
                    if mid_indices
                        .iter()
                        .any(|&mid_idx| reader_has_dep(mid_idx, target_uk))
                    {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Finds the nearest preceding `Impl`, `AssocFn`, `AssocTy`, `AssocConst`, or `Trait`
    /// definition in `(reader_idx, file_id)` at or before `line` via binary search on `defs_by_pos`.
    pub fn find_enclosing_impl_or_trait_def(
        &self,
        reader_idx: usize,
        file_id: u32,
        line: u32,
    ) -> Option<&DefEntry> {
        let reader = self.readers.get(reader_idx)?;
        let defs = reader.definitions();
        let slice = self
            .unit_match
            .defs_in_file_up_to(reader_idx, file_id, line);
        for &(_, _, def_idx) in slice.iter().rev() {
            if let Some(d) = defs.get(def_idx as usize) {
                let k = DefKind::from(d.kind);
                if matches!(
                    k,
                    DefKind::Impl
                        | DefKind::AssocFn
                        | DefKind::AssocTy
                        | DefKind::AssocConst
                        | DefKind::Trait
                ) {
                    return Some(d);
                }
            }
        }
        None
    }

    /// Finds a generic type parameter (`DefKind::TyParam`) in `(reader_idx, file_id)` matching `name`
    /// whose enclosing item span contains `(line, col)` via binary search on `defs_by_pos`.
    pub fn find_type_param_in_reader<'a>(
        &'a self,
        reader_idx: usize,
        file_id: u32,
        name: &str,
        line: u32,
        col: u32,
    ) -> Option<ResolvedDefinition<'a>> {
        let reader = self.readers.get(reader_idx)?;
        let defs = reader.definitions();
        let slice = self
            .unit_match
            .defs_in_file_up_to(reader_idx, file_id, u32::MAX);
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

    /// Resolves a definition at `(file_id, line, col)` in `reader_idx`, reusing a pre-looked-up
    /// `inner_ref` if present, or binary-searching `defs_by_pos` for `file_id`.
    pub fn find_def_at_in_reader<'a>(
        &'a self,
        reader_idx: usize,
        file_id: u32,
        line: u32,
        col: u32,
        inner_ref: Option<&'a RefEntry>,
    ) -> Option<ResolvedDefinition<'a>> {
        let reader = self.readers.get(reader_idx)?;
        if let Some(r) = inner_ref {
            return Some(reader.resolve_ref_definition(r));
        }
        let defs = reader.definitions();
        let slice = self
            .unit_match
            .defs_in_file_up_to(reader_idx, file_id, line);
        let mut best_def: Option<(&DefEntry, u64)> = None;
        for &(_, _, def_idx) in slice {
            if let Some(def) = defs.get(def_idx as usize) {
                let def_span = def.span();
                if def_span.contains(file_id, line, col) {
                    let size = def_span.approximate_size();
                    if best_def.is_none_or(|(_, best_size)| size < best_size) {
                        best_def = Some((def, size));
                    }
                }
            }
        }
        best_def.map(|(def, _)| reader.def_to_resolved(def))
    }
}
