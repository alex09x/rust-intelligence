/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Which stored references count as uses of a name: the single filter the reader's lookups
//! (`find_innermost_ref_at_file_id`, `find_references`) apply to the reference table, and the
//! span test behind the precedence of a definition at the cursor over a reference enclosing it.

use super::IndexReader;
use crate::format::*;

impl<B: AsRef<[u8]>> IndexReader<B> {
    /// The definition `def_id` among the definitions stored under `path`, if any.
    fn find_definition_by_id_under_path(&self, def_id: u32, path: &str) -> Option<&DefEntry> {
        let hash = fnv1a_64(path.as_bytes());
        let defs_hash = self.defs_by_hash();
        let defs = self.definitions();
        let mut i = defs_hash.binary_search_by_key(&hash, |e| e.hash()).ok()?;
        while i > 0 && defs_hash[i - 1].hash() == hash {
            i -= 1;
        }
        while i < defs_hash.len() && defs_hash[i].hash() == hash {
            let def = &defs[defs_hash[i].index() as usize];
            if def.def_id() == def_id {
                return Some(def);
            }
            i += 1;
        }
        None
    }

    /// True when `r` is a use of its target's name. A reference whose span properly encloses the
    /// span of its own target's definition is not one: it is the span of the item or file that
    /// defines the target. The driver writes two such references per `#[test]` function: a plain
    /// `Fn` reference spanning the whole function (the call the test harness generates) and a
    /// macro `Const` reference to the generated test constant spanning the whole file or module
    /// (the constant's definition sits on the `#[test]` attribute, so the span begins where the
    /// definition begins and reaches far past it); derive expansions add references spanning a
    /// whole variant or field. A definition whose end lies before its start (an index without end
    /// spans) is never enclosed. The reader's lookups (`find_innermost_ref_at_file_id`,
    /// `find_references`) skip references that fail this test; `IndexReader::references` still
    /// returns every record.
    pub fn is_name_reference(&self, r: &RefEntry) -> bool {
        let path = self.get_str(r.target_path_offset());
        if DefKind::from(r.kind) == DefKind::Local && path.rsplit("::").next().unwrap_or(path).starts_with("__") {
            return false;
        }
        if r.target_def_id() == u32::MAX {
            return true;
        }
        let Some(def) = self.find_definition_by_id_under_path(r.target_def_id(), path) else {
            return true;
        };
        let (span, def_span) = (r.span(), def.span());
        let def_end_valid = (def_span.end_line(), def_span.end_col()) >= (def_span.start_line(), def_span.start_col());
        !(def_end_valid && span_strictly_inside(&def_span, &span))
    }

    /// True when the span of `r` can contain a definition's name span: a span over more than one
    /// line, or a single-line span wider than its target's name. A reference as wide as the name
    /// it uses contains no definition, so `find_definition_at` answers it without looking for one;
    /// a span whose end lies before its start contains nothing.
    pub fn may_enclose_definition(&self, r: &RefEntry) -> bool {
        let span = r.span();
        let (start, end) = ((span.start_line(), span.start_col()), (span.end_line(), span.end_col()));
        if end < start {
            return false;
        }
        if span.end_line() != span.start_line() {
            return true;
        }
        let target_path = self.get_str(r.target_path_offset());
        let name = target_path.rsplit("::").next().unwrap_or(target_path);
        (span.end_col() - span.start_col()) as usize > name.chars().count()
    }
}

/// True when `inner` lies within `outer` in the same file and the two spans differ.
pub(crate) fn span_strictly_inside(inner: &Span, outer: &Span) -> bool {
    let (inner_start, inner_end) = ((inner.start_line(), inner.start_col()), (inner.end_line(), inner.end_col()));
    let (outer_start, outer_end) = ((outer.start_line(), outer.start_col()), (outer.end_line(), outer.end_col()));
    inner.file_id() == outer.file_id() && outer_start <= inner_start && inner_end <= outer_end && (inner_start, inner_end) != (outer_start, outer_end)
}

#[cfg(test)]
mod tests {
    use super::super::cursor_precedence_tests::write_fixture;
    use super::super::IndexReader;

    #[test]
    fn only_a_span_wider_than_its_target_name_may_enclose_a_definition() {
        let dir = std::env::temp_dir().join(format!("ridx_may_enclose_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let reader = IndexReader::open(write_fixture(&dir)).unwrap();
        // 7:5-7:8 is as wide as `Arc`; 8:5-8:30 is wider; 1:1-30:2 spans thirty lines.
        let name_wide = reader.find_innermost_ref_at("src/lib.rs", 7, 6).unwrap();
        let wider = reader.find_innermost_ref_at("src/lib.rs", 8, 6).unwrap();
        let invocation = reader.find_innermost_ref_at("src/lib.rs", 20, 1).unwrap();
        assert_eq!((reader.may_enclose_definition(name_wide), reader.may_enclose_definition(wider), reader.may_enclose_definition(invocation),), (false, true, true));
        let _ = std::fs::remove_dir_all(dir);
    }
}
