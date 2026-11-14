/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Whether a reference's target unit is the unit a definition was compiled in, for cross-crate lookups.

use super::*;

impl WorkspaceIndex {
    /// Checks whether `ref_target_unit` matches `target_unit` for `target_crate`.
    /// Handles exact unit match, as well as single-version workspaces where Cargo
    /// assigned different unit hashes (e.g. lib vs test vs dep compilation).
    #[inline]
    pub(crate) fn matches_unit(&self, reader_idx: usize, ref_target_unit: &str, target_unit: &str, target_crate: &str) -> bool {
        if !self.unit_match.matches_unit(reader_idx, ref_target_unit, target_unit, target_crate, &self.reader_extern_units) {
            return false;
        }
        if ref_target_unit != target_unit {
            let ws_sr = |uk: &str| {
                self.readers_for_unit_fast(uk)
                    .iter()
                    .copied()
                    .find(|&r| !self.reader_is_dependency.get(r).copied().unwrap_or(false))
                    .and_then(|r| self.source_roots.get(r).and_then(|s| s.as_deref()))
            };
            if let (Some(sr1), Some(sr2)) = (ws_sr(target_unit), ws_sr(ref_target_unit)) {
                if sr1 != sr2 {
                    return false;
                }
            }
        }
        true
    }

    #[inline]
    pub(super) fn matches_unit_memoized<'a>(
        &self,
        match_memo: &mut std::collections::HashMap<(usize, &'a str), bool>,
        reader_idx: usize,
        ref_target_unit: &'a str,
        target_unit: &str,
        target_crate: &str,
    ) -> bool {
        if ref_target_unit == target_unit {
            true
        } else if let Some(&cached) = match_memo.get(&(reader_idx, ref_target_unit)) {
            cached
        } else {
            let res = self.matches_unit(reader_idx, ref_target_unit, target_unit, target_crate);
            match_memo.insert((reader_idx, ref_target_unit), res);
            res
        }
    }
}
