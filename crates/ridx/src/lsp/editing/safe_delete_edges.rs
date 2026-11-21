/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Every edge the compiler recorded to a definition, read from the shards as stored: a macro's expansion, an impl
//! header, whatever navigation leaves out (review of 4009c2c4: `call!()` expanding to `helper()` kept `helper`).

use ridx_format::WorkspaceIndex;
use std::path::{Path, PathBuf};

/// Where a use is: its file and the compiler's 1-based lines and character columns.
pub(crate) struct Edge {
    pub path: PathBuf,
    pub start: (u32, u32),
    pub end: (u32, u32),
}

/// The edges of every shard to the definition at the compiler's `line` and `col` of `file` (canonical): by that place,
/// or by `target_path` where the reference records no target file of its own (a recorded one elsewhere is another
/// definition of the same path, another version or unit: review of b0b16e39). `Err` when a matching edge's use cannot
/// be placed: a known use is no absence of uses.
pub(crate) fn raw_edges(ws: &WorkspaceIndex, file: &Path, line: u32, col: u32, target_path: &str) -> Result<Vec<Edge>, ()> {
    let mut out = Vec::new();
    for (idx, r) in ws.readers().iter().enumerate() {
        for e in r.references() {
            let target_file = r.get_str(e.target_file_offset());
            let target_unit = r.get_str(e.target_unit_offset());
            // A relative target file is the target unit's, not the referrer's (review of 6b91c9eb: two crates' src/lib.rs);
            // with no unit recorded, any reader's root that places it at this file does; a unit recorded but not loaded
            // is another unit's, never one of these (review of 4d025d17).
            let unit_reader = ws.readers().iter().position(|t| !target_unit.is_empty() && t.unit_key() == target_unit);
            let target_readers: Vec<usize> = match (unit_reader, target_unit.is_empty()) {
                (Some(t), _) => vec![t],
                (None, true) => (0..ws.readers().len()).collect(),
                (None, false) => Vec::new(),
            };
            let matches = if target_file.is_empty() {
                // No file: the path names it, unless the recorded unit's own definition of that id is known to be in
                // another file (review of 6b91c9eb); which reader a file routes to is no evidence of who defines in it
                // (review of 4d025d17).
                // The definition is looked up in the unit the reference names, or in the referring one when it names
                // none: by its id, or by its path where no id is recorded (a foreign reference: review of 61e7c131).
                let path = r.get_str(e.target_path_offset());
                let owner = unit_reader.or(target_unit.is_empty().then_some(idx));
                let elsewhere = owner.is_some_and(|t| {
                    let tr = &ws.readers()[t];
                    let def = if e.target_def_id() == u32::MAX {
                        let mut by_path = tr.definitions().iter().filter(|d| tr.get_str(d.path_offset()) == path);
                        by_path.next().filter(|_| by_path.next().is_none())
                    } else {
                        tr.definitions().iter().find(|d| d.def_id() == e.target_def_id())
                    };
                    def.is_some_and(|d| {
                        super::safe_delete_fresh::resolve(ws, t, tr.get_file(d.span().file_id())).is_some_and(|p| ridx_format::canonicalize_best_effort(&p) != file)
                    })
                });
                path == target_path && !elsewhere
            } else {
                e.target_line() == line
                    && e.target_col() == col
                    && target_readers.iter().any(|&t| super::safe_delete_fresh::resolve(ws, t, target_file).is_some_and(|p| ridx_format::canonicalize_best_effort(&p) == file))
            };
            if !matches {
                continue;
            }
            let span = e.span();
            let path = super::safe_delete_fresh::resolve(ws, idx, r.get_file(span.file_id())).ok_or(())?;
            out.push(Edge { path, start: (span.start_line(), span.start_col()), end: (span.end_line(), span.end_col()) });
        }
    }
    Ok(out)
}
