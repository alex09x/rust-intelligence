/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use std::io;

use super::IndexReader;
use crate::format::*;

impl<B: AsRef<[u8]>> IndexReader<B> {
    /// Look up all direct Deref or supertrait targets for a canonical type path.
    pub fn find_deref_targets(&self, source: &str) -> Vec<&str> {
        let mut targets = Vec::new();
        let entries = self.deref_entries();
        if entries.is_empty() || source.is_empty() {
            return targets;
        }
        let hash = fnv1a_64(source.as_bytes());
        if let Ok(idx) = entries.binary_search_by_key(&hash, |e| e.source_type_hash()) {
            let mut start = idx;
            while start > 0 && entries[start - 1].source_type_hash() == hash {
                start -= 1;
            }
            let mut i = start;
            while i < entries.len() && entries[i].source_type_hash() == hash {
                let entry = &entries[i];
                if self.get_str(entry.source_type_offset()) == source {
                    targets.push(self.get_str(entry.target_type_offset()));
                }
                i += 1;
            }
        }
        targets
    }

    /// Look up the first direct Deref target for a canonical type path.
    pub fn find_deref_target(&self, source: &str) -> Option<&str> {
        self.find_deref_targets(source).into_iter().next()
    }

    /// Resolves the autoderef/supertrait chain starting from `source` up to `max_depth` steps.
    ///
    /// Returns the sequence of target type canonical paths (excluding `source`).
    /// Handles multiple supertraits and cycles gracefully via breadth-first search and visited tracking.
    pub fn resolve_deref_chain(&self, source: &str, max_depth: usize) -> Vec<&str> {
        let mut chain = Vec::new();
        let mut queue = std::collections::VecDeque::new();
        let mut visited = std::collections::HashSet::new();
        visited.insert(source);
        queue.push_back((source, 0));

        while let Some((current, depth)) = queue.pop_front() {
            if depth >= max_depth {
                continue;
            }
            for target in self.find_deref_targets(current) {
                if !target.is_empty() && visited.insert(target) {
                    chain.push(target);
                    queue.push_back((target, depth + 1));
                }
            }
        }

        chain
    }

    /// Fast symbol search across definitions matching a query string.
    ///
    /// For ASCII queries, this avoids per-entry heap allocations by lowercasing the query once
    /// and performing case-insensitive byte matching directly over string pool slices.
    pub fn search_symbols(&self, query: &str) -> Vec<&DefEntry> {
        if query.is_ascii() {
            let q_lower = query.to_ascii_lowercase();
            self.search_symbols_ascii(q_lower.as_bytes())
        } else {
            let mut matches = Vec::new();
            let q = query.to_lowercase();
            for def in self.definitions() {
                if def.kind == DefKind::TyParam as u8 {
                    continue;
                }
                let name = self.get_str(def.name_offset());
                let path = self.get_str(def.path_offset());
                if name.to_lowercase().contains(&q) || path.to_lowercase().contains(&q) {
                    matches.push(def);
                }
            }
            matches
        }
    }

    /// Fast ASCII case-insensitive symbol search across definitions using a pre-lowercased query slice.
    pub fn search_symbols_ascii(&self, needle_lower: &[u8]) -> Vec<&DefEntry> {
        let mut matches = Vec::new();
        for def in self.definitions() {
            if def.kind == DefKind::TyParam as u8 {
                continue;
            }
            let name = self.get_str(def.name_offset());
            let path = self.get_str(def.path_offset());
            if contains_ignore_case_ascii(name, needle_lower) || contains_ignore_case_ascii(path, needle_lower) {
                matches.push(def);
            }
        }
        matches
    }

    /// Iterates over matching definitions without heap allocation, stopping when `cb` returns false
    /// or when `budget` is exhausted.
    pub fn for_each_symbol_ascii<F>(&self, needle_lower: &[u8], budget: &mut usize, cb: F) -> bool
    where
        F: FnMut(&DefEntry) -> bool,
    {
        let mut record_budget = usize::MAX;
        self.for_each_symbol_ascii_budgeted(needle_lower, budget, &mut record_budget, cb)
    }

    /// Iterates over matching definitions without heap allocation, stopping when `cb` returns false,
    /// when match `budget` is exhausted, or when `record_budget` of inspected definitions is reached.
    pub fn for_each_symbol_ascii_budgeted<F>(&self, needle_lower: &[u8], match_budget: &mut usize, record_budget: &mut usize, mut cb: F) -> bool
    where
        F: FnMut(&DefEntry) -> bool,
    {
        for def in self.definitions() {
            if *record_budget == 0 {
                return false;
            }
            *record_budget -= 1;
            if def.kind == DefKind::TyParam as u8 {
                continue;
            }
            let name = self.get_str(def.name_offset());
            let path = self.get_str(def.path_offset());
            if contains_ignore_case_ascii(name, needle_lower) || contains_ignore_case_ascii(path, needle_lower) {
                if *match_budget == 0 {
                    return false;
                }
                *match_budget -= 1;
                if !cb(def) {
                    return false;
                }
            }
        }
        true
    }
}

/// Case-insensitive ASCII substring search without heap allocations.
/// Returns true if `haystack` contains `needle_lower` case-insensitively.
/// `needle_lower` MUST already be in ASCII lowercase.
#[inline]
pub(crate) fn contains_ignore_case_ascii(haystack: &str, needle_lower: &[u8]) -> bool {
    if needle_lower.is_empty() {
        return true;
    }
    let h = haystack.as_bytes();
    let n = needle_lower.len();
    if h.len() < n {
        return false;
    }
    let q0 = needle_lower[0];
    let q0_upper = q0.to_ascii_uppercase();
    let rest = &needle_lower[1..];
    for i in 0..=(h.len() - n) {
        let b = h[i];
        if (b == q0 || b == q0_upper) && h[i + 1..i + n].iter().zip(rest).all(|(&hb, &qb)| hb.to_ascii_lowercase() == qb) {
            return true;
        }
    }
    false
}

pub(super) fn validate_sections(buf_len: usize, header: &Header) -> io::Result<()> {
    // 1. String pool alignment and bounds
    if (header.string_pool_offset as usize) % 8 != 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "String pool offset is not 8-byte aligned"));
    }
    let pool_start = header.string_pool_offset as usize;
    let pool_len = header.string_pool_len as usize;
    let pool_end = pool_start.checked_add(pool_len).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "String pool length overflow"))?;
    if pool_end > buf_len {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "String pool extends beyond file bounds"));
    }

    if (header.crate_name_offset as usize) >= pool_len || (header.crate_version_offset as usize) >= pool_len || (header.target_offset as usize) >= pool_len {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Crate metadata string offsets exceed string pool bounds"));
    }

    // 2. Files table
    check_section(buf_len, header.files_offset, header.files_count, std::mem::size_of::<FileEntry>(), std::mem::align_of::<FileEntry>(), "Files table")?;

    // 3. Definitions table
    check_section(buf_len, header.defs_offset, header.defs_count, std::mem::size_of::<DefEntry>(), 4, "Definitions table")?;

    // 4. Definitions hash index
    check_section(buf_len, header.defs_by_hash_offset, header.defs_by_hash_count, std::mem::size_of::<HashIndexEntry>(), 8, "Definitions hash index")?;

    // 5. References table
    check_section(buf_len, header.refs_offset, header.refs_count, std::mem::size_of::<RefEntry>(), 4, "References table")?;

    // 6. References target hash index
    check_section(buf_len, header.refs_by_target_offset, header.refs_by_target_count, std::mem::size_of::<HashIndexEntry>(), 8, "References target hash index")?;

    // 7. Types table
    check_section(buf_len, header.types_offset, header.types_count, std::mem::size_of::<TypeEntry>(), 4, "Types table")?;

    // 8. Definitions by owner table
    check_section(buf_len, header.defs_by_owner_offset, header.defs_by_owner_count, std::mem::size_of::<OwnerIndexEntry>(), 8, "Definitions by owner table")?;

    // 9. Derefs table
    check_section(buf_len, header.derefs_offset, header.derefs_count, std::mem::size_of::<DerefEntry>(), 8, "Derefs table")?;

    // 10. Notes table (format version 4)
    check_section(buf_len, header.notes_offset, header.notes_count, std::mem::size_of::<NoteEntry>(), 4, "Notes table")?;

    // 11. TyArena, FnSig, Impl, and AssocTy sections (format version 5)
    check_section(buf_len, header.ty_defs_offset, header.ty_defs_count, std::mem::size_of::<TyDefEntry>(), 8, "TyArena defs table")?;
    check_section(buf_len, header.ty_entries_offset, header.ty_entries_count, std::mem::size_of::<TyEntry>(), 4, "TyArena entries table")?;
    check_section(buf_len, header.ty_bounds_offset, header.ty_bounds_count, std::mem::size_of::<TyBoundEntry>(), 4, "TyArena bounds table")?;
    check_section(buf_len, header.ty_args_offset, header.ty_args_count, std::mem::size_of::<u32>(), 4, "TyArena args table")?;
    check_section(buf_len, header.fn_sigs_offset, header.fn_sigs_count, std::mem::size_of::<FnSigEntry>(), 4, "FnSigs table")?;
    check_section(buf_len, header.impls_offset, header.impls_count, std::mem::size_of::<ImplEntry>(), 4, "Impls table")?;
    check_section(buf_len, header.assoc_tys_offset, header.assoc_tys_count, std::mem::size_of::<AssocTyEntry>(), 4, "AssocTys table")?;
    check_section(buf_len, header.fields_offset, header.fields_count, std::mem::size_of::<FieldEntry>(), 4, "Fields table")?;
    check_section(buf_len, header.def_tys_offset, header.def_tys_count, std::mem::size_of::<DefTyEntry>(), 4, "DefTys table")?;
    check_section(buf_len, header.generic_params_offset, header.generic_params_count, std::mem::size_of::<GenericParamEntry>(), 4, "GenericParams table")?;
    check_section(buf_len, header.where_preds_offset, header.where_preds_count, std::mem::size_of::<WherePredEntry>(), 4, "WherePreds table")?;
    check_section(buf_len, header.lang_items_offset, header.lang_items_count, std::mem::size_of::<LangItemEntry>(), 4, "LangItems table")?;
    check_section(buf_len, header.uses_offset, header.uses_count, std::mem::size_of::<UseEntry>(), 4, "Uses table")?;
    check_section(buf_len, header.cfgs_offset, header.cfgs_count, std::mem::size_of::<CfgEntry>(), 4, "Cfgs table")?;

    Ok(())
}

pub(super) fn check_section(buf_len: usize, offset: u64, count: u32, elem_size: usize, required_align: usize, section_name: &str) -> io::Result<()> {
    let off = offset as usize;
    if off % required_align != 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("{section_name} offset {offset} is not aligned to {required_align} bytes")));
    }
    if count == 0 {
        let max_empty_offset = buf_len.div_ceil(8) * 8;
        if off > max_empty_offset {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("{section_name} offset {offset} extends beyond file bounds ({max_empty_offset})")));
        }
        return Ok(());
    }
    let total_bytes = (count as usize).checked_mul(elem_size).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("{section_name} size calculation overflow")))?;
    let end = off.checked_add(total_bytes).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("{section_name} end offset overflow")))?;
    if end > buf_len {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("{section_name} extends beyond file bounds ({end} > {buf_len})")));
    }
    Ok(())
}
