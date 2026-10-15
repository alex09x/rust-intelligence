/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::{IndexReader, ResolvedDefinition};
use crate::format::*;

impl<B: AsRef<[u8]>> IndexReader<B> {
    /// Find a definition by its canonical path (e.g. `stdx::panic_context::enter`).
    pub fn find_definition_by_path(&self, path: &str) -> Option<&DefEntry> {
        let hash = fnv1a_64(path.as_bytes());
        let defs_hash = self.defs_by_hash();
        let defs = self.definitions();

        let partition = defs_hash.binary_search_by_key(&hash, |e| e.hash());
        if let Ok(idx) = partition {
            // Check matching hash range
            let mut i = idx;
            while i > 0 && defs_hash[i - 1].hash() == hash {
                i -= 1;
            }
            while i < defs_hash.len() && defs_hash[i].hash() == hash {
                let def = &defs[defs_hash[i].index() as usize];
                if self.get_str(def.path_offset()) == path {
                    return Some(def);
                }
                i += 1;
            }
        }
        None
    }

    /// Go to definition at a given cursor position `(file, line, col)`.
    pub fn find_definition_at(
        &self,
        file: &str,
        line: u32,
        col: u32,
    ) -> Option<ResolvedDefinition<'_>> {
        let file_id = self.find_file_id(file)?;

        // 1. Check references: is the cursor on a symbol reference? Pick innermost span.
        let mut best_ref: Option<(&RefEntry, u64)> = None;
        for r in self.references() {
            let r_span = r.span();
            if r_span.file_id() == file_id && r_span.contains(file_id, line, col) {
                let size = r_span.approximate_size();
                match best_ref {
                    None => best_ref = Some((r, size)),
                    Some((_, best_size)) if size < best_size => best_ref = Some((r, size)),
                    _ => {}
                }
            }
        }

        if let Some((r, _)) = best_ref {
            // If it resolves to a local def_id:
            if r.target_def_id() != u32::MAX {
                let defs = self.definitions();
                if let Some(def) = defs.iter().find(|d| d.def_id() == r.target_def_id()) {
                    let def_span = def.span();
                    return Some(ResolvedDefinition {
                        name: self.get_str(def.name_offset()),
                        path: self.get_str(def.path_offset()),
                        kind: DefKind::from(def.kind),
                        signature: if def.sig_offset() != 0 {
                            Some(self.get_str(def.sig_offset()))
                        } else {
                            None
                        },
                        file: self.get_file(def_span.file_id()),
                        start_line: def_span.start_line(),
                        start_col: def_span.start_col(),
                        end_line: def_span.end_line(),
                        end_col: def_span.end_col(),
                        is_macro_expansion: def.is_macro_expansion(),
                        is_reexport: def.is_reexport(),
                        source_root: None,
                        unit_key: None,
                        reader_idx: None,
                    });
                }
            }
            // Otherwise return target path and location
            let target_path = self.get_str(r.target_path_offset());
            let target_file = self.get_str(r.target_file_offset());
            let target_unit = self.get_str(r.target_unit_offset());
            let target_name = target_path.rsplit("::").next().unwrap_or(target_path);

            return Some(ResolvedDefinition {
                name: target_name,
                path: target_path,
                kind: DefKind::from(r.kind),
                signature: None,
                file: if !target_file.is_empty() {
                    target_file
                } else {
                    "external"
                },
                start_line: r.target_line(),
                start_col: r.target_col(),
                end_line: r.target_line(),
                end_col: r.target_col() + (target_name.len() as u32),
                is_macro_expansion: r.is_macro_expansion(),
                is_reexport: false,
                source_root: None,
                unit_key: if !target_unit.is_empty() {
                    Some(target_unit)
                } else {
                    None
                },
                reader_idx: None,
            });
        }

        // 2. Check if cursor is directly on a definition span (pick innermost span, e.g. struct inside mod)
        let mut best_def: Option<(&DefEntry, u64)> = None;
        for def in self.definitions() {
            let def_span = def.span();
            if def_span.file_id() == file_id && def_span.contains(file_id, line, col) {
                let size = def_span.approximate_size();
                match best_def {
                    None => best_def = Some((def, size)),
                    Some((_, best_size)) if size < best_size => best_def = Some((def, size)),
                    _ => {}
                }
            }
        }

        if let Some((def, _)) = best_def {
            let def_span = def.span();
            return Some(ResolvedDefinition {
                name: self.get_str(def.name_offset()),
                path: self.get_str(def.path_offset()),
                kind: DefKind::from(def.kind),
                signature: if def.sig_offset() != 0 {
                    Some(self.get_str(def.sig_offset()))
                } else {
                    None
                },
                file: self.get_file(def_span.file_id()),
                start_line: def_span.start_line(),
                start_col: def_span.start_col(),
                end_line: def_span.end_line(),
                end_col: def_span.end_col(),
                is_macro_expansion: def.is_macro_expansion(),
                is_reexport: def.is_reexport(),
                source_root: None,
                unit_key: None,
                reader_idx: None,
            });
        }

        None
    }

    /// Find all references to a given symbol path (e.g. `thread::Builder::allow_leak`).
    pub fn find_references(&self, target_path: &str) -> Vec<&RefEntry> {
        let hash = fnv1a_64(target_path.as_bytes());
        let ref_hashes = self.refs_by_target();
        let refs = self.references();
        let mut results = Vec::new();

        if let Ok(idx) = ref_hashes.binary_search_by_key(&hash, |e| e.hash()) {
            let mut i = idx;
            while i > 0 && ref_hashes[i - 1].hash() == hash {
                i -= 1;
            }
            while i < ref_hashes.len() && ref_hashes[i].hash() == hash {
                let r = &refs[ref_hashes[i].index() as usize];
                if self.get_str(r.target_path_offset()) == target_path {
                    results.push(r);
                }
                i += 1;
            }
        }
        results
    }

    /// Finds the innermost TypeEntry at a cursor position `(file, line, col)`.
    pub fn find_innermost_type_entry(&self, file: &str, line: u32, col: u32) -> Option<&TypeEntry> {
        let file_id = self.find_file_id(file)?;
        let types = self.types();
        let key = (file_id, line, col);
        let hi = types.partition_point(|e| {
            let s = e.span();
            (s.file_id(), s.start_line(), s.start_col()) <= key
        });

        let mut best: Option<(&TypeEntry, u64)> = None;
        for entry in types[..hi].iter().rev() {
            let span = entry.span();
            if span.file_id() != file_id {
                break;
            }
            if let Some((_, best_size)) = best {
                let min_size = if line > span.start_line() {
                    (line - span.start_line()) as u64 * 80
                } else {
                    (col.saturating_sub(span.start_col()) as u64).min(80)
                };
                if min_size > best_size {
                    break;
                }
            }
            if span.contains(file_id, line, col) {
                let size = span.approximate_size();
                match best {
                    None => best = Some((entry, size)),
                    Some((_, best_size)) if size <= best_size => best = Some((entry, size)),
                    _ => {}
                }
            }
        }

        best.map(|(e, _)| e)
    }

    /// Retrieves the inferred expression type at a cursor position `(file, line, col)`.
    /// When nested expressions overlap, selects the tightest (innermost) span.
    pub fn type_at(&self, file: &str, line: u32, col: u32) -> Option<&str> {
        self.find_innermost_type_entry(file, line, col)
            .map(|e| self.get_str(e.type_offset()))
    }

    /// Retrieves the inferred expression type and optional canonical owner path at `(file, line, col)`.
    /// Returns `(type_name, canonical_owner_path)`.
    pub fn type_entry_at(&self, file: &str, line: u32, col: u32) -> Option<(&str, Option<&str>)> {
        let entry = self.find_innermost_type_entry(file, line, col)?;
        let ty_name = self.get_str(entry.type_offset());
        let owner_offset = entry.owner_path_offset();
        let owner = if owner_offset != 0 {
            let s = self.get_str(owner_offset);
            if s.is_empty() {
                None
            } else {
                Some(s)
            }
        } else {
            None
        };
        Some((ty_name, owner))
    }

    /// Find all definitions belonging to an owner (canonical type path or module path).
    ///
    /// Performs binary search on the 64-bit FNV-1a hash of `owner`, with full collision
    /// verification against the interned canonical owner string.
    pub fn find_definitions_by_owner(&self, owner: &str) -> Vec<&DefEntry> {
        let owner_entries = self.defs_by_owner();
        if owner_entries.is_empty() || owner.is_empty() || owner.starts_with("scope:") {
            return Vec::new();
        }
        let hash = fnv1a_64(owner.as_bytes());
        let defs = self.definitions();
        let mut results = Vec::new();

        if let Ok(idx) = owner_entries.binary_search_by_key(&hash, |e| e.hash()) {
            let mut start = idx;
            while start > 0 && owner_entries[start - 1].hash() == hash {
                start -= 1;
            }
            let mut i = start;
            while i < owner_entries.len() && owner_entries[i].hash() == hash {
                let entry = &owner_entries[i];
                if self.get_str(entry.owner_path_offset()) == owner {
                    let def_idx = entry.def_index() as usize;
                    if def_idx < defs.len() {
                        results.push(&defs[def_idx]);
                    }
                }
                i += 1;
            }
        }

        results
    }

    /// Find all canonical owners associated with a definition index.
    pub fn find_owners_by_def_index(&self, def_index: u32) -> Vec<&str> {
        let mut owners = Vec::new();
        for entry in self.defs_by_owner() {
            if entry.def_index() == def_index {
                let s = self.get_str(entry.owner_path_offset());
                if !s.is_empty() && !s.starts_with("scope:") && !owners.contains(&s) {
                    owners.push(s);
                }
            }
        }
        owners
    }

    /// Returns the lexical enclosing scope coordinates `(start_line, start_col, end_line, end_col)`
    /// for a definition index if recorded.
    pub fn type_param_scope(&self, def_index: u32) -> Option<(u32, u32, u32, u32)> {
        for entry in self.defs_by_owner() {
            if entry.def_index() == def_index {
                let s = self.get_str(entry.owner_path_offset());
                if let Some(rest) = s.strip_prefix("scope:") {
                    let mut parts = rest.split(':');
                    if let (Some(sl), Some(sc), Some(el), Some(ec)) = (
                        parts.next().and_then(|p| p.parse().ok()),
                        parts.next().and_then(|p| p.parse().ok()),
                        parts.next().and_then(|p| p.parse().ok()),
                        parts.next().and_then(|p| p.parse().ok()),
                    ) {
                        return Some((sl, sc, el, ec));
                    }
                }
            }
        }
        None
    }

    /// Finds a generic type parameter (`DefKind::TyParam`) in the specified file matching `name`
    /// whose enclosing item span contains `(line, col)`.
    pub fn find_type_param_in_scope(
        &self,
        file: &str,
        name: &str,
        line: u32,
        col: u32,
    ) -> Option<(&DefEntry, u32)> {
        let file_id = self.find_file_id(file)?;
        let mut best: Option<(&DefEntry, u32, u64)> = None;

        for (idx, def) in self.definitions().iter().enumerate() {
            if def.kind != DefKind::TyParam as u8 {
                continue;
            }
            if self.get_str(def.name_offset()) != name {
                continue;
            }
            let span = def.span();
            if span.file_id() != file_id {
                continue;
            }

            // 1. Check explicit enclosing scope recorded in owners
            let mut matched_scope = None;
            if let Some((sl, sc, el, ec)) = self.type_param_scope(idx as u32) {
                let in_range = (line > sl || (line == sl && col >= sc))
                    && (line < el || (line == el && col <= ec));
                if in_range {
                    let size = ((el.saturating_sub(sl)) as u64) * 10_000
                        + ((ec.saturating_sub(sc)) as u64);
                    matched_scope = Some(size);
                }
            }

            // 2. Fallback: check parent definition by path or def span
            let (in_scope, size) = if let Some(scope_size) = matched_scope {
                (true, scope_size)
            } else {
                let path = self.get_str(def.path_offset());
                if let Some((parent_path, _)) = path.rsplit_once("::") {
                    if let Some(parent_def) = self.find_definition_by_path(parent_path) {
                        let p_span = parent_def.span();
                        let in_range = p_span.file_id() == file_id
                            && (line > p_span.start_line()
                                || (line == p_span.start_line() && col >= p_span.start_col()))
                            && (line < p_span.end_line()
                                || (line == p_span.end_line() && col <= p_span.end_col()));
                        (in_range, p_span.approximate_size())
                    } else {
                        let in_range = line >= span.start_line() && line <= span.end_line();
                        (in_range, span.approximate_size())
                    }
                } else {
                    let in_range = line >= span.start_line() && line <= span.end_line();
                    (in_range, span.approximate_size())
                }
            };

            if in_scope {
                match best {
                    None => best = Some((def, idx as u32, size)),
                    Some((_, _, best_size)) if size < best_size => {
                        best = Some((def, idx as u32, size))
                    }
                    _ => {}
                }
            }
        }

        best.map(|(d, idx, _)| (d, idx))
    }
}
