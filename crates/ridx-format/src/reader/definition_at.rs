/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The definition at a cursor: the innermost name reference there, the definition at the cursor
//! when its span lies inside that reference's span, or the smallest definition containing the
//! cursor when no reference does.

use super::name_reference::span_strictly_inside;
use super::{IndexReader, ResolvedDefinition};
use crate::format::*;

/// True when a definition's name is a synthetic compiler placeholder rather than a user-written
/// source identifier (`""` crate root, `"{use#N}"`, `"{impl#N}"`, `"{closure#N}"`, or a numeric
/// tuple-struct field `"0"`, `"1"`, ... whose span covers the field's type).
pub(crate) fn is_synthetic_def_name(kind: DefKind, name: &str) -> bool {
    kind == DefKind::Impl
        || name.is_empty()
        || name.starts_with('{')
        || (kind == DefKind::Field && name.bytes().all(|b| b.is_ascii_digit()))
        || (kind == DefKind::Local && name.starts_with("__"))
}

impl<B: AsRef<[u8]>> IndexReader<B> {
    /// Go to definition at a given cursor position `(file, line, col)`.
    ///
    /// The innermost name reference at the cursor answers, unless a definition at the cursor lies
    /// inside that reference's span: a reference spanning a macro invocation or a generated call
    /// encloses every item written inside it, and on such an item's name the item is the answer.
    /// Equal spans (a self-reference) keep the reference. A reference as wide as its target's name
    /// cannot enclose a definition, so no definition is looked for behind it. Without a reference
    /// the smallest named definition containing the cursor answers; the nameless crate-root module
    /// the driver records with the span of the whole file never does, so a position on no name
    /// has no definition.
    pub fn find_definition_at(&self, file: &str, line: u32, col: u32) -> Option<ResolvedDefinition<'_>> {
        let file_id = self.find_file_id(file)?;
        let inner = self.find_innermost_ref_at_file_id(file_id, line, col);
        if let Some(r) = inner {
            if !self.may_enclose_definition(r) {
                return Some(self.resolve_ref_definition(r));
            }
        }

        let mut best_def: Option<(&DefEntry, u64)> = None;
        for def in self.definitions() {
            let def_span = def.span();
            // Span checks first: they are integer comparisons on every definition; the name is
            // read only for a definition that contains the cursor.
            if def_span.contains(file_id, line, col)
                && inner.is_none_or(|r| span_strictly_inside(&def_span, &r.span()))
                && !is_synthetic_def_name(DefKind::from(def.kind), self.get_str(def.name_offset()))
            {
                let size = def_span.approximate_size();
                if best_def.is_none_or(|(_, best_size)| size < best_size) {
                    best_def = Some((def, size));
                }
            }
        }
        if let Some((def, _)) = best_def {
            return Some(self.def_to_resolved(def));
        }
        inner.map(|r| self.resolve_ref_definition(r))
    }

    /// Finds the innermost `RefEntry` at `(file, line, col)` via binary search on span-sorted references.
    pub fn find_innermost_ref_at(&self, file: &str, line: u32, col: u32) -> Option<&RefEntry> {
        let file_id = self.find_file_id(file)?;
        self.find_innermost_ref_at_file_id(file_id, line, col)
    }

    /// Finds the innermost `RefEntry` at `(file_id, line, col)` via binary search on span-sorted references.
    /// References that are not uses of a name (`is_name_reference`) are skipped.
    pub fn find_innermost_ref_at_file_id(&self, file_id: u32, line: u32, col: u32) -> Option<&RefEntry> {
        let refs = self.references();
        let key = (file_id, line, col);
        let hi = refs.partition_point(|r| {
            let s = r.span();
            (s.file_id(), s.start_line(), s.start_col()) <= key
        });

        let mut best: Option<(&RefEntry, u64)> = None;
        for r in refs[..hi].iter().rev() {
            let span = r.span();
            if span.file_id() != file_id {
                break;
            }
            if let Some((_, best_size)) = best {
                let min_size = if line > span.start_line() { (line - span.start_line()) as u64 * 80 } else { (col.saturating_sub(span.start_col()) as u64).min(80) };
                if min_size > best_size {
                    break;
                }
            }
            if !span.contains(file_id, line, col) {
                continue;
            }
            if !self.is_name_reference(r) {
                continue;
            }
            let size = span.approximate_size();
            // A shorthand (`Point { x }`) names the local and the field on one span: the local comes first, as
            // rust-analyzer's first definition of a field shorthand.
            let yields_to_local = |b: &RefEntry| DefKind::from(b.kind) == DefKind::Local && DefKind::from(r.kind) == DefKind::Field;
            if best.is_none_or(|(b, best_size)| size < best_size || (size == best_size && !yields_to_local(b))) {
                best = Some((r, size));
            }
        }
        best.map(|(r, _)| r)
    }
}
