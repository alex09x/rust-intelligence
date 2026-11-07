/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `IndexWriter` builders and deterministic sorting for `MacroDefEntry`.

use super::IndexWriter;
use crate::format::{MacroDefEntry, MacroDelim, MacroEdition};

/// High-level declarative macro definition input for [`IndexWriter::add_macro_def`].
#[derive(Debug, Clone, Copy)]
pub struct RawMacroDef<'a> {
    pub def_index: u32,
    pub body: &'a str,
    pub flags: u8,
    pub delim: MacroDelim,
    pub edition: MacroEdition,
}

impl IndexWriter {
    /// Records a declarative macro (`macro_rules!` or Rust 2.0 `macro`) body for `raw.def_index`.
    pub fn add_macro_def(&mut self, raw: RawMacroDef<'_>) {
        let body_offset = self.intern_string(raw.body);
        self.macro_defs.push(MacroDefEntry::new(raw.def_index, body_offset, raw.flags, raw.delim, raw.edition));
    }

    /// Total count of raw `MacroDefEntry` rows queued before deduplication.
    pub fn macro_def_count(&self) -> usize {
        self.macro_defs.len()
    }

    pub(super) fn prepare_sorted_macro_defs(&self) -> Vec<MacroDefEntry> {
        let mut list = self.macro_defs.clone();
        list.sort_by_key(|m| m.def_index);
        list.dedup_by_key(|m| m.def_index);
        list
    }
}
