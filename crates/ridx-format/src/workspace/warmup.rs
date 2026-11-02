/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Background cache warming for workspace dependency reachability bitsets and associated-item indices.

use super::unit_match::{ReachableCache, UnitMatchIndex};
use super::WorkspaceIndex;
use crate::format::DefKind;
use crate::reader::IndexReader;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

impl UnitMatchIndex {
    pub(crate) fn compute_reachable_bits(
        n: usize,
        unit_to_readers: &HashMap<String, Vec<usize>>,
        reader_extern_units: &[HashMap<String, String>],
        reader_dependencies: &[Vec<String>],
    ) -> ReachableCache {
        let words = n.div_ceil(64);
        let mut counts = Vec::with_capacity(n);
        let mut adj: Vec<Vec<usize>> = Vec::with_capacity(n);
        for cur in 0..n {
            counts.push((reader_extern_units.get(cur).map_or(0, HashMap::len), reader_dependencies.get(cur).map_or(0, Vec::len)));
            let mut neighbors: Vec<usize> = reader_extern_units
                .get(cur)
                .into_iter()
                .flat_map(|m| m.values().map(String::as_str))
                .chain(reader_dependencies.get(cur).into_iter().flat_map(|d| d.iter().map(String::as_str)))
                .filter_map(|uk| unit_to_readers.get(uk))
                .flatten()
                .copied()
                .collect();
            neighbors.sort_unstable();
            neighbors.dedup();
            adj.push(neighbors);
        }
        let mut bits_vec = Vec::with_capacity(n);
        let (mut frontier, mut next_frontier) = (Vec::new(), Vec::new());
        for sr in 0..n {
            let mut bits = vec![0u64; words];
            bits[sr >> 6] |= 1u64 << (sr & 63);
            frontier.clear();
            frontier.push(sr);
            for _ in 0..6 {
                if frontier.is_empty() {
                    break;
                }
                next_frontier.clear();
                for &cur in &frontier {
                    for &mid_idx in &adj[cur] {
                        let (word, mask) = (&mut bits[mid_idx >> 6], 1u64 << (mid_idx & 63));
                        if (*word & mask) == 0 {
                            *word |= mask;
                            next_frontier.push(mid_idx);
                        }
                    }
                }
                std::mem::swap(&mut frontier, &mut next_frontier);
            }
            bits_vec.push(bits);
        }
        (counts, bits_vec)
    }

    pub(crate) fn compute_assoc_ty_defs<B: AsRef<[u8]>>(reader: &IndexReader<B>) -> Vec<u32> {
        reader
            .definitions()
            .iter()
            .enumerate()
            .filter_map(|(idx, d)| {
                let p = reader.get_str(d.path_offset());
                (matches!(DefKind::from(d.kind), DefKind::AssocTy | DefKind::AssocFn | DefKind::AssocConst)
                    && (d.kind == DefKind::AssocTy as u8 || p.starts_with('<') || p.contains("<impl ")))
                .then_some(idx as u32)
            })
            .collect()
    }

    pub(crate) fn compute_field_variant_decl_refs<B: AsRef<[u8]>>(reader: &IndexReader<B>) -> Vec<(u32, u32, u32, u32)> {
        let mut file_cache: HashMap<u32, Option<u32>> = HashMap::new();
        let mut out = Vec::new();
        for (idx, r) in reader.references().iter().enumerate() {
            if !matches!(DefKind::from(r.kind), DefKind::Field | DefKind::Variant | DefKind::VariantCtor) {
                continue;
            }
            let tf_off = r.target_file_offset();
            if tf_off == 0 {
                continue;
            }
            let fid_opt = *file_cache.entry(tf_off).or_insert_with(|| {
                let s = reader.get_str(tf_off);
                (!s.is_empty()).then(|| reader.find_file_id(s)).flatten()
            });
            if let Some(fid) = fid_opt {
                out.push((fid, r.target_line(), r.target_col(), idx as u32));
            }
        }
        out.sort_unstable();
        out.dedup_by_key(|e| (e.0, e.1, e.2));
        out
    }

    #[inline]
    pub(crate) fn field_variant_decl_refs<B: AsRef<[u8]>>(&self, reader_idx: usize, reader: &IndexReader<B>) -> &[(u32, u32, u32, u32)] {
        let Some(cell) = self.field_variant_decl_refs.get(reader_idx) else {
            return &[];
        };
        cell.get_or_init(|| Self::compute_field_variant_decl_refs(reader))
    }
}

impl WorkspaceIndex {
    pub(crate) fn find_ref_targeting_position_in_reader<'a>(&'a self, reader_idx: usize, file_id: u32, line: u32, col: u32) -> Option<crate::reader::ResolvedDefinition<'a>> {
        let reader = self.readers.get(reader_idx)?;
        if !self.unit_match.is_valid_for_readers(&self.readers) {
            return reader.find_ref_targeting_position(file_id, line, col);
        }
        let slice = self.unit_match.field_variant_decl_refs(reader_idx, reader);
        let lo = slice.partition_point(|&(f, l, _, _)| (f, l) < (file_id, line));
        let hi = slice.partition_point(|&(f, l, _, _)| (f, l) <= (file_id, line));
        let refs = reader.references();
        for &(_, _, tcol, r_idx) in &slice[lo..hi] {
            let Some(r) = refs.get(r_idx as usize) else { continue };
            let tp = reader.get_str(r.target_path_offset());
            let name = tp.rsplit("::").next().unwrap_or(tp);
            let end_col = tcol + name.chars().count() as u32;
            if col >= tcol && col <= end_col {
                let mut res = reader.resolve_ref_definition(r);
                res.is_macro_expansion = false;
                return Some(res);
            }
        }
        None
    }

    /// Spawns a background thread to warm the reachability bitset cache and workspace assoc-item indices.
    pub fn warm_background_caches(&self) {
        let reach_cell = Arc::clone(&self.unit_match.reachable_reader_bits);
        let assoc_cells = Arc::clone(&self.unit_match.assoc_ty_defs);
        let fv_cells = Arc::clone(&self.unit_match.field_variant_decl_refs);
        let manifest_opt = super::cargo_targets::find_manifest_path(self);
        let n = self.unit_match.resolved_uks.len();
        let need_reach = reach_cell.get().is_none();
        let ws_paths: Vec<(usize, PathBuf)> = self
            .paths
            .iter()
            .enumerate()
            .filter(|&(i, p)| {
                !self.is_reader_external(i)
                    && !p.as_os_str().is_empty()
                    && (assoc_cells.get(i).is_some_and(|c| c.get().is_none()) || fv_cells.get(i).is_some_and(|c| c.get().is_none()))
            })
            .map(|(i, p)| (i, p.clone()))
            .collect();
        if !need_reach && ws_paths.is_empty() && manifest_opt.is_none() {
            return;
        }
        let unit_to_readers = if need_reach { self.unit_match.unit_to_readers.clone() } else { HashMap::new() };
        let extern_units = if need_reach { self.reader_extern_units.clone() } else { Vec::new() };
        let deps = if need_reach { self.reader_dependencies.clone() } else { Vec::new() };
        let _ = std::thread::Builder::new().name("ridx-warmup".into()).spawn(move || {
            if let Some(mp) = manifest_opt {
                let _ = super::cargo_targets::cached_cargo_metadata(&mp);
            }
            if need_reach && reach_cell.get().is_none() {
                let _ = reach_cell.get_or_init(|| UnitMatchIndex::compute_reachable_bits(n, &unit_to_readers, &extern_units, &deps));
            }
            for (idx, path) in ws_paths {
                let need_assoc = assoc_cells.get(idx).is_some_and(|c| c.get().is_none());
                let need_fv = fv_cells.get(idx).is_some_and(|c| c.get().is_none());
                if need_assoc || need_fv {
                    if let Ok(r) = IndexReader::open(&path) {
                        if let Some(cell) = assoc_cells.get(idx).filter(|c| c.get().is_none()) {
                            let _ = cell.get_or_init(|| UnitMatchIndex::compute_assoc_ty_defs(&r));
                        }
                        if let Some(cell) = fv_cells.get(idx).filter(|c| c.get().is_none()) {
                            let _ = cell.get_or_init(|| UnitMatchIndex::compute_field_variant_decl_refs(&r));
                        }
                    }
                }
            }
        });
    }
}
