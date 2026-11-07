/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Zero-copy `O(log N)` reader accessors for declarative macro definitions (`MacroDefEntry`).

use super::IndexReader;
use crate::format::{MacroDefEntry, MacroDelim, MacroEdition, MACRO_FLAG_BUILTIN, MACRO_FLAG_EXPORTED, MACRO_FLAG_LOCAL_INNER, MACRO_FLAG_MACRO_RULES};

/// Decoded zero-copy view of a `MacroDefEntry` row and its interned token body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MacroDefView<'a> {
    pub def_index: u32,
    pub body: &'a str,
    pub flags: u8,
    pub delim: MacroDelim,
    pub edition: MacroEdition,
}

impl<'a> MacroDefView<'a> {
    #[inline]
    pub const fn is_macro_rules(&self) -> bool {
        (self.flags & MACRO_FLAG_MACRO_RULES) != 0
    }

    #[inline]
    pub const fn is_exported(&self) -> bool {
        (self.flags & MACRO_FLAG_EXPORTED) != 0
    }

    #[inline]
    pub const fn has_local_inner_macros(&self) -> bool {
        (self.flags & MACRO_FLAG_LOCAL_INNER) != 0
    }

    #[inline]
    pub const fn is_builtin(&self) -> bool {
        (self.flags & MACRO_FLAG_BUILTIN) != 0
    }
}

impl<B: AsRef<[u8]>> IndexReader<B> {
    /// Raw slice of `MacroDefEntry` rows sorted by `def_index`.
    #[inline]
    pub fn macro_defs(&self) -> &[MacroDefEntry] {
        self.typed_slice(self.header.macro_defs_offset, self.header.macro_defs_count)
    }

    #[inline]
    fn decode_macro_def(&self, raw: &MacroDefEntry) -> MacroDefView<'_> {
        let e = raw.from_le();
        MacroDefView { def_index: e.def_index, body: self.get_str(e.body_offset), flags: e.flags, delim: e.delim_kind(), edition: e.edition_kind() }
    }

    /// Iterates over all decoded `MacroDefView` entries in `def_index` order.
    pub fn macro_defs_iter(&self) -> impl ExactSizeIterator<Item = MacroDefView<'_>> + '_ {
        self.macro_defs().iter().map(|raw| self.decode_macro_def(raw))
    }

    /// `O(log N)` lookup for the declarative macro definition owned by `def_index`.
    pub fn macro_def_for_def(&self, def_index: u32) -> Option<MacroDefView<'_>> {
        let entries = self.macro_defs();
        let idx = entries.binary_search_by_key(&def_index, |raw| raw.def_index()).ok()?;
        Some(self.decode_macro_def(&entries[idx]))
    }
}
