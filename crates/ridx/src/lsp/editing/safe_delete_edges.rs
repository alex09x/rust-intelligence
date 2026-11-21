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
            let matches = if target_file.is_empty() {
                r.get_str(e.target_path_offset()) == target_path
            } else {
                e.target_line() == line
                    && e.target_col() == col
                    && super::safe_delete_fresh::resolve(ws, idx, target_file).is_some_and(|p| ridx_format::canonicalize_best_effort(&p) == file)
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
