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

/// The edges of every shard to the definition at the compiler's `line` and `col` of `file` (canonical), or named by
/// `target_path`.
pub(crate) fn raw_edges(ws: &WorkspaceIndex, file: &Path, line: u32, col: u32, target_path: &str) -> Vec<Edge> {
    let mut out = Vec::new();
    for (idx, r) in ws.readers().iter().enumerate() {
        for e in r.references() {
            let by_place = e.target_line() == line
                && e.target_col() == col
                && super::safe_delete_fresh::resolve(ws, idx, r.get_str(e.target_file_offset())).is_some_and(|p| ridx_format::canonicalize_best_effort(&p) == file);
            if !by_place && r.get_str(e.target_path_offset()) != target_path {
                continue;
            }
            let span = e.span();
            let Some(path) = super::safe_delete_fresh::resolve(ws, idx, r.get_file(span.file_id())) else { continue };
            out.push(Edge { path, start: (span.start_line(), span.start_col()), end: (span.end_line(), span.end_col()) });
        }
    }
    out
}
