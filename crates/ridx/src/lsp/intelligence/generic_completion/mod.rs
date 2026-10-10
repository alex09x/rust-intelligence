/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Generic parameter bounds resolution for `T::` completion.
//!
//! Resolves generic parameters in scope (from enclosing functions, impl blocks, or traits)
//! to candidate owners based on their declared trait bounds and associated types.

pub mod parse;
pub mod scan;
pub mod types;

pub use parse::*;
pub use scan::*;
pub use types::*;

use crate::lsp::intelligence::self_completion::collect_trait_and_supertraits;
use ridx_format::{DefKind, WorkspaceIndex};
use std::collections::HashSet;

/// Resolves owner candidates for `T::` (or `T::Assoc::`) where `T` is a generic parameter in scope.
pub fn resolve_generic_param_owners(
    ws: &WorkspaceIndex,
    file: &str,
    line_1based: u32,
    line_slice: &str,
    overlay_text: Option<&str>,
    is_dirty: bool,
    qual: &str,
) -> Option<Vec<String>> {
    let mut segs = qual.split("::");
    let first_seg = segs.next()?;

    // Fast path: try resolving generic parameter from index driver data (no disk I/O, no lexical parse)
    let param_bounds_opt: Option<Vec<String>> = if !is_dirty {
        let col = (line_slice.chars().count() + 1) as u32;
        if let Some(def) = ws.find_type_param_in_scope(file, first_seg, line_1based, col) {
            let mut bounds = Vec::new();
            if let Some(sig) = def.signature {
                for b in sig.split('+') {
                    let trimmed = b.trim();
                    if !trimmed.is_empty() && !bounds.contains(&trimmed.to_string()) {
                        bounds.push(trimmed.to_string());
                    }
                }
            }
            if bounds.is_empty() {
                for owner in ws.find_owners_of_definition(&def) {
                    if !bounds.contains(&owner.to_string()) {
                        bounds.push(owner.to_string());
                    }
                }
            }
            Some(bounds)
        } else {
            None
        }
    } else {
        None
    };

    let bounds = if let Some(bounds) = param_bounds_opt {
        bounds
    } else {
        // Fallback: lexical scope parser (used for active overlay edits or unindexed files)
        let scopes =
            crate::lsp::intelligence::scope_cache::get_or_parse_file_scopes(file, overlay_text)?;
        let cursor_byte = scopes.cursor_byte(line_1based, line_slice);
        let generics = scopes.find_generic_params_in_scope(cursor_byte);
        if generics.is_empty() {
            return None;
        }
        let param = generics.iter().find(|p| p.name == first_seg)?;
        param.bounds.clone()
    };

    if bounds.is_empty() {
        return None;
    }

    let mut owners = Vec::new();
    let mut seen = HashSet::new();

    let remaining_segs: Vec<&str> = segs.collect();
    if remaining_segs.is_empty() {
        // Simple case: `T::` -> candidates are the trait bounds on `T`
        for bound in &bounds {
            collect_trait_and_supertraits(ws, file, bound, &mut owners, &mut seen, 0);
        }
    } else {
        // Multi-segment case: `T::Assoc::...`
        let mut cur_trait_names = bounds;
        for seg in remaining_segs {
            let mut next_traits = Vec::new();
            for tname in &cur_trait_names {
                let def_opt = ws.resolve_qualifier(file, tname);
                let target_path = def_opt.as_ref().map(|d| d.path).unwrap_or(tname.as_str());
                for def in ws.find_definitions_by_owner_for_file(file, target_path) {
                    if def.name == seg && def.kind == DefKind::AssocTy {
                        // Inspect associated type bound: driver data signature/owners first,
                        // fallback to scope_cache parser only if missing
                        let mut assoc_bounds = Vec::new();
                        if let Some(sig) = def.signature {
                            for b in sig.split('+') {
                                let trimmed = b.trim();
                                if !trimmed.is_empty()
                                    && !assoc_bounds.contains(&trimmed.to_string())
                                {
                                    assoc_bounds.push(trimmed.to_string());
                                }
                            }
                        }
                        if assoc_bounds.is_empty() {
                            for owner in ws.find_owners_of_definition(&def) {
                                if owner != target_path
                                    && !assoc_bounds.contains(&owner.to_string())
                                {
                                    assoc_bounds.push(owner.to_string());
                                }
                            }
                        }
                        if assoc_bounds.is_empty() {
                            assoc_bounds =
                                crate::lsp::intelligence::scope_cache::get_or_resolve_assoc_bounds(
                                    def.file,
                                    def.start_line,
                                );
                        }
                        for clean_bound in assoc_bounds {
                            if !next_traits.contains(&clean_bound) {
                                next_traits.push(clean_bound);
                            }
                        }
                    }
                }
            }
            cur_trait_names = next_traits;
            if cur_trait_names.is_empty() {
                break;
            }
        }

        for tname in &cur_trait_names {
            collect_trait_and_supertraits(ws, file, tname, &mut owners, &mut seen, 0);
        }
    }

    if owners.is_empty() {
        None
    } else {
        Some(owners)
    }
}

#[cfg(test)]
mod tests;
