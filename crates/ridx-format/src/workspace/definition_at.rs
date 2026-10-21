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
use crate::format::{DefEntry, RefEntry};
use crate::reader::{span_strictly_inside, ResolvedDefinition};

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
    fn smallest_def_at(
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
            if reader.get_str(def.name_offset()).is_empty() {
                continue;
            }
            let def_span = def.span();
            if !def_span.contains(file_id, line, col)
                || !inside.is_none_or(|r| span_strictly_inside(&def_span, &r.span()))
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
}
