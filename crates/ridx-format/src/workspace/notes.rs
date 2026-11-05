/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Notes (format version 4) looked up across the workspace's shards.

use super::WorkspaceIndex;
use crate::reader::ResolvedDefinition;

impl WorkspaceIndex {
    /// The `kind` note whose span starts at `(line, col)` of `file` (an absolute or workspace path).
    pub fn note_at(&self, file: &str, line: u32, col: u32, kind: u32) -> Option<&str> {
        self.resolved_reader_file_ids(file).into_iter().find_map(|(idx, file_id)| self.readers.get(idx)?.note_at(file_id, line, col, kind))
    }

    /// The `kind` note at the name of definition `d`, read from the shard that defines it.
    pub fn note_at_definition(&self, d: &ResolvedDefinition<'_>, kind: u32) -> Option<&str> {
        let reader = self.readers.get(d.reader_idx?)?;
        let file_id = reader.find_file_id(d.file)?;
        reader.note_at(file_id, d.start_line, d.start_col, kind)
    }
}
