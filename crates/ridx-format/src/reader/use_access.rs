/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Zero-copy `IndexReader` accessors for `UseEntry` module import/re-export tables (v5+, 1.2-G7).

use crate::format::{UseEntry, UseOwner};
use crate::ty_arena::{UseIter, UseView};
use crate::writer::typed_generics::use_kind_bucket;

use super::IndexReader;

impl<B: AsRef<[u8]>> IndexReader<B> {
    /// Raw slice of `UseEntry` records sorted by `(owner_raw, kind_bucket, name_bytes, ...)` (empty for v3/v4 shards).
    #[inline]
    pub fn uses(&self) -> &[UseEntry] {
        let h = self.header();
        self.typed_slice(h.uses_offset, h.uses_count)
    }

    /// Zero-allocation iterator over all `UseEntry` records in this shard.
    #[inline]
    pub fn uses_iter(&self) -> UseIter<'_> {
        UseIter::new(self.ty_arena(), self.uses())
    }

    #[inline]
    fn raw_uses_for_owner(&self, owner: UseOwner) -> &[UseEntry] {
        let all = self.uses();
        let target = owner.to_raw();
        let start = all.partition_point(|e| e.owner_raw() < target);
        let end = start + all[start..].partition_point(|e| e.owner_raw() == target);
        &all[start..end]
    }

    /// Binary-searches all `UseEntry` rows (named, glob, and `_` trait imports) owned by `owner`.
    #[inline]
    pub fn uses_for_owner(&self, owner: UseOwner) -> UseIter<'_> {
        UseIter::new(self.ty_arena(), self.raw_uses_for_owner(owner))
    }

    /// Binary-searches all `UseEntry` rows owned by the crate root (`UseOwner::CrateRoot`).
    #[inline]
    pub fn uses_for_crate_root(&self) -> UseIter<'_> {
        self.uses_for_owner(UseOwner::CrateRoot)
    }

    /// Binary-searches all `UseEntry` rows owned by `DefEntry` `def_index` (`UseOwner::Def(def_index)`).
    #[inline]
    pub fn uses_for_def(&self, def_index: u32) -> UseIter<'_> {
        self.uses_for_owner(UseOwner::Def(def_index))
    }

    /// Binary-searches a named `UseEntry` (`!is_glob && !is_underscore`) in `owner` by identifier `name` in `O(log N)`.
    #[inline]
    pub fn use_by_name(&self, owner: UseOwner, name: &str) -> Option<UseView<'_>> {
        let owner_rows = self.raw_uses_for_owner(owner);
        let named_end = owner_rows.partition_point(|e| use_kind_bucket(e.flags) == 0);
        let arena = self.ty_arena();
        let idx = owner_rows[..named_end].binary_search_by(|e| arena.str_at(e.name_offset()).as_bytes().cmp(name.as_bytes())).ok()?;
        Some(UseView::from_raw(arena, owner_rows[idx]))
    }

    /// Binary-searches all glob imports/re-exports (`use path::*;`) owned by `owner`.
    #[inline]
    pub fn glob_uses_for_owner(&self, owner: UseOwner) -> UseIter<'_> {
        let owner_rows = self.raw_uses_for_owner(owner);
        let start = owner_rows.partition_point(|e| use_kind_bucket(e.flags) < 1);
        let end = start + owner_rows[start..].partition_point(|e| use_kind_bucket(e.flags) == 1);
        UseIter::new(self.ty_arena(), &owner_rows[start..end])
    }

    /// Binary-searches all anonymous trait imports (`use Trait as _;`) owned by `owner`.
    #[inline]
    pub fn underscore_uses_for_owner(&self, owner: UseOwner) -> UseIter<'_> {
        let owner_rows = self.raw_uses_for_owner(owner);
        let start = owner_rows.partition_point(|e| use_kind_bucket(e.flags) < 2);
        let end = start + owner_rows[start..].partition_point(|e| use_kind_bucket(e.flags) == 2);
        UseIter::new(self.ty_arena(), &owner_rows[start..end])
    }

    /// Returns the crate's active `#[prelude_import]` glob `UseView` (if present).
    #[inline]
    pub fn prelude_use(&self) -> Option<UseView<'_>> {
        self.glob_uses_for_owner(UseOwner::CrateRoot).find(UseView::is_prelude)
    }
}
