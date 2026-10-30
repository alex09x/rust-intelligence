/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The definition at a cursor within one reader of the workspace, found through the
//! position-sorted `defs_by_pos` table of `UnitMatchIndex`, and its precedence over a reference
//! enclosing the cursor.

use super::WorkspaceIndex;
use crate::format::{DefEntry, DefKind, RefEntry};
use crate::reader::{is_synthetic_def_name, span_strictly_inside, ResolvedDefinition};

impl WorkspaceIndex {
    /// Resolves a definition at `(file_id, line, col)` in `reader_idx`. A pre-looked-up
    /// `inner_ref` answers unless a definition at the cursor lies inside its span (an item written
    /// inside a macro invocation or a generated call whose reference spans it); without a
    /// reference the smallest definition containing the cursor answers.
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
            if let Some(def) =
                self.definition_inside_reference_at(reader_idx, file_id, line, col, r)
            {
                return Some(reader.def_to_resolved(def));
            }
            return Some(reader.resolve_ref_definition(r));
        }
        self.smallest_def_at(reader_idx, file_id, line, col, None)
            .map(|def| reader.def_to_resolved(def))
    }

    /// The smallest definition of `reader_idx` at `(file_id, line, col)` whose span lies inside the
    /// span of the reference `r`, when `r` is wide enough to enclose one. Such a definition takes
    /// precedence over the reference at the cursor.
    pub fn definition_inside_reference_at(
        &self,
        reader_idx: usize,
        file_id: u32,
        line: u32,
        col: u32,
        r: &RefEntry,
    ) -> Option<&DefEntry> {
        let reader = self.readers.get(reader_idx)?;
        if !reader.may_enclose_definition(r) {
            return None;
        }
        self.smallest_def_at(reader_idx, file_id, line, col, Some(r))
    }

    /// The smallest named definition of `reader_idx` containing `(file_id, line, col)`, restricted
    /// to definitions whose span lies inside the span of `inside` when given. The nameless
    /// crate-root module spanning the whole file is never an answer.
    pub(crate) fn smallest_def_at(
        &self,
        reader_idx: usize,
        file_id: u32,
        line: u32,
        col: u32,
        inside: Option<&RefEntry>,
    ) -> Option<&DefEntry> {
        let reader = self.readers.get(reader_idx)?;
        let defs = reader.definitions();
        let slice = self
            .unit_match
            .defs_in_file_up_to(reader_idx, file_id, line);
        let mut best: Option<(&DefEntry, u64)> = None;
        for &(_, _, def_idx) in slice {
            let Some(def) = defs.get(def_idx as usize) else {
                continue;
            };
            let def_span = def.span();
            // Span checks first; the name is read only for a definition that contains the cursor.
            if !def_span.contains(file_id, line, col)
                || !inside.is_none_or(|r| span_strictly_inside(&def_span, &r.span()))
                || is_synthetic_def_name(DefKind::from(def.kind), reader.get_str(def.name_offset()))
            {
                continue;
            }
            let size = def_span.approximate_size();
            if best.is_none_or(|(_, best_size)| size < best_size) {
                best = Some((def, size));
            }
        }
        best.map(|(def, _)| def)
    }

    pub(crate) fn find_indexed_definition_at<'a>(
        &'a self,
        file: &str,
        line: u32,
        col: u32,
    ) -> Option<ResolvedDefinition<'a>> {
        let (exact, wide, fmod) = self.find_indexed_definition_candidates(file, line, col);
        exact.or(wide).or(fmod)
    }

    #[allow(clippy::type_complexity)]
    pub(crate) fn find_indexed_definition_candidates<'a>(
        &'a self,
        file: &str,
        line: u32,
        col: u32,
    ) -> (
        Option<ResolvedDefinition<'a>>,
        Option<ResolvedDefinition<'a>>,
        Option<ResolvedDefinition<'a>>,
    ) {
        let mut fallback_mod = None;
        let mut fallback_wide_ref: Option<(usize, ResolvedDefinition<'a>)> = None;
        let mut cursor_ident_cache: Option<Option<(String, u32, u32)>> = None;
        let mut direct: Vec<(usize, std::borrow::Cow<'_, str>)> = self
            .reader_indices_for_file(file)
            .into_iter()
            .filter_map(|i| self.target_file_for_reader(i, file).map(|tf| (i, tf)))
            .collect();
        direct.sort_by_key(|&(idx, _)| {
            usize::from(
                !self
                    .target_kinds
                    .get(idx)
                    .copied()
                    .flatten()
                    .is_some_and(|k| k.is_lib()),
            )
        });
        let pairs = if !direct.is_empty() {
            direct
        } else {
            self.candidate_reader_indices_for_file(file)
                .into_iter()
                .filter_map(|i| self.target_file_for_reader(i, file).map(|tf| (i, tf)))
                .collect()
        };

        for (i, target_file) in pairs {
            let reader = &self.readers[i];
            let Some(fid) = reader.find_file_id(target_file.as_ref()) else {
                continue;
            };
            let inner_ref = reader.find_innermost_ref_at_file_id(fid, line, col);
            let def_inside =
                inner_ref.and_then(|r| self.definition_inside_reference_at(i, fid, line, col, r));
            let res = if let Some(def) = def_inside {
                reader.def_to_resolved(def)
            } else if let Some(r) = inner_ref {
                reader.resolve_ref_definition(r)
            } else if let Some(def) = self.smallest_def_at(i, fid, line, col, None) {
                reader.def_to_resolved(def)
            } else {
                continue;
            };

            if res.kind == DefKind::Mod && (res.start_line != line || col < res.start_col) {
                if fallback_mod.is_none() {
                    fallback_mod = Some(res);
                }
                continue;
            }

            let is_wide_enclosing_ref =
                def_inside.is_none() && inner_ref.is_some_and(|r| reader.may_enclose_definition(r));
            if is_wide_enclosing_ref {
                let ident_opt =
                    cursor_ident_cache.get_or_insert_with(|| self.cursor_ident_at(file, line, col));
                if let Some((ref cid, _, _)) = ident_opt {
                    if cid != res.name && !res.path.ends_with(&format!("::{cid}")) {
                        if fallback_wide_ref.is_none() {
                            fallback_wide_ref = Some((i, res));
                        }
                        continue;
                    }
                }
            }
            let resolved = self.finalize_reader_resolution(i, res);
            let wide_resolved =
                fallback_wide_ref.map(|(wi, wres)| self.finalize_reader_resolution(wi, wres));
            return (Some(resolved), wide_resolved, fallback_mod);
        }
        let wide_resolved =
            fallback_wide_ref.map(|(wi, wres)| self.finalize_reader_resolution(wi, wres));
        (None, wide_resolved, fallback_mod)
    }

    /// Finds a definition at a cursor location, seamlessly resolving cross-crate references.
    pub fn find_definition_at<'a>(
        &'a self,
        file: &str,
        line: u32,
        col: u32,
    ) -> Option<ResolvedDefinition<'a>> {
        let (exact, fallback_wide_ref, fallback_mod) =
            self.find_indexed_definition_candidates(file, line, col);
        if let Some(resolved) = exact {
            return Some(resolved);
        }
        if let Some(cursor_def) = self.resolve_cursor_path_at(file, line, col) {
            return Some(cursor_def);
        }
        fallback_wide_ref.or(fallback_mod)
    }
}
