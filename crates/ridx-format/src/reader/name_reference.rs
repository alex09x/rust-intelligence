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
//! (`find_innermost_ref_at_file_id`, `find_references`) apply to the reference table.

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
        if r.target_def_id() == u32::MAX {
            return true;
        }
        let path = self.get_str(r.target_path_offset());
        let Some(def) = self.find_definition_by_id_under_path(r.target_def_id(), path) else {
            return true;
        };
        let (span, def_span) = (r.span(), def.span());
        let (start, end) = (
            (span.start_line(), span.start_col()),
            (span.end_line(), span.end_col()),
        );
        let def_start = (def_span.start_line(), def_span.start_col());
        let def_end = (def_span.end_line(), def_span.end_col());
        let def_end_valid = def_end >= def_start;
        let encloses = span.file_id() == def_span.file_id()
            && start <= def_start
            && end >= def_end
            && (start, end) != (def_start, def_end);
        !(def_end_valid && encloses)
    }
}
