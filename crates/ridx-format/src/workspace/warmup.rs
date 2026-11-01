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
}

impl WorkspaceIndex {
    /// Spawns a background thread to warm the reachability bitset cache and workspace assoc-item indices.
    pub fn warm_background_caches(&self) {
        let reach_cell = Arc::clone(&self.unit_match.reachable_reader_bits);
        let assoc_cells = Arc::clone(&self.unit_match.assoc_ty_defs);
        let n = self.unit_match.resolved_uks.len();
        let need_reach = reach_cell.get().is_none();
        let ws_paths: Vec<(usize, PathBuf)> = self
            .paths
            .iter()
            .enumerate()
            .filter(|&(i, p)| !self.is_reader_external(i) && !p.as_os_str().is_empty() && assoc_cells.get(i).is_some_and(|c| c.get().is_none()))
            .map(|(i, p)| (i, p.clone()))
            .collect();
        if !need_reach && ws_paths.is_empty() {
            return;
        }
        let unit_to_readers = if need_reach { self.unit_match.unit_to_readers.clone() } else { HashMap::new() };
        let extern_units = if need_reach { self.reader_extern_units.clone() } else { Vec::new() };
        let deps = if need_reach { self.reader_dependencies.clone() } else { Vec::new() };
        let _ = std::thread::Builder::new().name("ridx-warmup".into()).spawn(move || {
            if need_reach && reach_cell.get().is_none() {
                let _ = reach_cell.get_or_init(|| UnitMatchIndex::compute_reachable_bits(n, &unit_to_readers, &extern_units, &deps));
            }
            for (idx, path) in ws_paths {
                if let Some(cell) = assoc_cells.get(idx) {
                    if cell.get().is_none() {
                        if let Ok(r) = IndexReader::open(&path) {
                            let _ = cell.get_or_init(|| UnitMatchIndex::compute_assoc_ty_defs(&r));
                        }
                    }
                }
            }
        });
    }
}
