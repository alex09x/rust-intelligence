/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Trait-item declaration lookup for `textDocument/declaration`.
//!
//! When the cursor is on an associated item (`AssocFn`, `AssocConst`, `AssocTy`) inside a
//! trait implementation (`impl Trait for Type`), `textDocument/declaration` navigates to the
//! trait's declaration of that associated item rather than the `impl` item itself.

use super::impl_parse::parse_impl_def_path;
use super::qualified_path::{extract_impl_trait, extract_root_crate};
use super::trait_impl_refs::{path_matches_trait, strip_generics};
use super::*;
use std::collections::HashSet;

impl WorkspaceIndex {
    /// Resolves the trait declaration for an associated item defined inside a trait `impl`.
    /// Returns `None` if `def` is not a trait-impl associated item or the trait declaration is not indexed.
    pub fn find_trait_item_declaration<'a>(&'a self, def: &ResolvedDefinition<'a>) -> Option<ResolvedDefinition<'a>> {
        if !matches!(def.kind, DefKind::AssocFn | DefKind::AssocConst | DefKind::AssocTy) || def.name.is_empty() {
            return None;
        }
        let trait_str = extract_impl_trait(def.path).or_else(|| self.find_enclosing_impl_trait_path(def))?;
        let raw_trait = strip_generics(trait_str);
        if raw_trait.is_empty() {
            return None;
        }

        let mut cands = std_map::std_path_candidates(raw_trait);
        if let Some(rest) = raw_trait.strip_prefix("core::") {
            cands.push(format!("std::{rest}"));
            cands.push(format!("alloc::{rest}"));
        } else if let Some(rest) = raw_trait.strip_prefix("alloc::") {
            cands.push(format!("std::{rest}"));
            cands.push(format!("core::{rest}"));
        }
        if let Some(r_idx) = def.reader_idx {
            let def_crate = self.readers[r_idx].crate_name();
            if !def_crate.is_empty() && !raw_trait.starts_with(&format!("{def_crate}::")) {
                cands.push(format!("{def_crate}::{raw_trait}"));
            }
        }

        let mut checked = HashSet::new();
        if let Some(pref) = def.reader_idx {
            checked.insert(pref);
            if let Some(res) = self.search_trait_item_decl_in_reader(pref, def, raw_trait, &cands) {
                return Some(res);
            }
        }
        for tp in &cands {
            let root_crate = extract_root_crate(tp);
            for &idx in self.readers_for_crate_fast(root_crate).iter() {
                if checked.insert(idx) {
                    if let Some(res) = self.search_trait_item_decl_in_reader(idx, def, raw_trait, &cands) {
                        return Some(res);
                    }
                }
            }
        }
        for idx in 0..self.readers.len() {
            if checked.insert(idx) {
                if let Some(res) = self.search_trait_item_decl_in_reader(idx, def, raw_trait, &cands) {
                    return Some(res);
                }
            }
        }
        None
    }

    fn find_enclosing_impl_trait_path<'a>(&'a self, def: &ResolvedDefinition<'a>) -> Option<&'a str> {
        let r_idx = def.reader_idx?;
        let r = self.readers.get(r_idx)?;
        let file_id = r.find_file_id(def.file)?;
        for d in r.definitions() {
            if DefKind::from(d.kind) == DefKind::Impl && d.span().file_id() == file_id && d.span().start_line() <= def.start_line && def.start_line <= d.span().end_line() {
                if let (Some(tr), _) = parse_impl_def_path(r.get_str(d.path_offset())) {
                    return Some(tr);
                }
            }
        }
        None
    }

    fn search_trait_item_decl_in_reader<'a>(&'a self, reader_idx: usize, def: &ResolvedDefinition<'a>, raw_trait: &str, cands: &[String]) -> Option<ResolvedDefinition<'a>> {
        let r = &self.readers[reader_idx];
        let cname = r.crate_name();
        let is_valid_item = |d: &DefEntry| -> bool {
            DefKind::from(d.kind) == def.kind
                && !d.is_reexport()
                && r.get_str(d.name_offset()) == def.name
                && r.get_file(d.span().file_id()) != "external"
                && extract_impl_trait(r.get_str(d.path_offset())).is_none()
        };

        let mut matches: Vec<&DefEntry> = Vec::new();
        for tp in cands {
            let local_tp = tp.strip_prefix(&format!("{cname}::")).unwrap_or(tp);
            let global_tp = format!("{cname}::{local_tp}");
            for owner_q in [local_tp, global_tp.as_str(), tp.as_str()] {
                for d in r.find_definitions_by_owner(owner_q) {
                    if is_valid_item(d) && !matches.iter().any(|existing| std::ptr::eq(*existing, d)) {
                        matches.push(d);
                    }
                }
            }
            for path_q in [format!("{local_tp}::{}", def.name), format!("{global_tp}::{}", def.name)] {
                if let Some(d) = r.find_definition_by_path(&path_q) {
                    if is_valid_item(d) && !matches.iter().any(|existing| std::ptr::eq(*existing, d)) {
                        matches.push(d);
                    }
                }
            }
        }

        if matches.is_empty() && (Some(reader_idx) == def.reader_idx || cands.iter().any(|tp| extract_root_crate(tp) == cname)) {
            let local_raw = raw_trait.strip_prefix(&format!("{cname}::")).unwrap_or(raw_trait);
            let global_raw = format!("{cname}::{local_raw}");
            for td in r.definitions() {
                if DefKind::from(td.kind) != DefKind::Trait {
                    continue;
                }
                let dp = r.get_str(td.path_offset());
                if path_matches_trait(dp, raw_trait, local_raw, &global_raw, cands) {
                    let dp_local = dp.strip_prefix(&format!("{cname}::")).unwrap_or(dp);
                    for owner_q in [dp, dp_local] {
                        for d in r.find_definitions_by_owner(owner_q) {
                            if is_valid_item(d) && !matches.iter().any(|existing| std::ptr::eq(*existing, d)) {
                                matches.push(d);
                            }
                        }
                    }
                }
            }
        }

        let best = if Some(reader_idx) == def.reader_idx && matches.len() > 1 {
            matches
                .iter()
                .copied()
                .filter(|d| r.get_file(d.span().file_id()) == def.file && d.span().start_line() <= def.start_line)
                .max_by_key(|d| d.span().start_line())
                .or_else(|| matches.first().copied())?
        } else {
            matches.first().copied()?
        };
        Some(self.make_resolved_definition(reader_idx, best))
    }
}
