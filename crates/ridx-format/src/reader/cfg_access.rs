/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Zero-copy `O(log N)` reader accessors for `CfgEntry`, enabled features, and target triple.

use super::IndexReader;
use crate::format::{CfgEntry, NO_CFG_VALUE};

impl<B: AsRef<[u8]>> IndexReader<B> {
    /// Effective target triple recorded for this shard (or `""` if none).
    #[inline]
    pub fn target_triple(&self) -> &str {
        let off = self.header.target_offset;
        if off == 0 {
            ""
        } else {
            self.get_str(off)
        }
    }

    /// Raw slice of `CfgEntry` rows sorted by `(key_str, Option<value_str>)`.
    #[inline]
    pub fn cfgs(&self) -> &[CfgEntry] {
        let h = self.header();
        let count = h.cfgs_count as usize;
        if count == 0 {
            return &[];
        }
        let start = h.cfgs_offset as usize;
        let len = count * std::mem::size_of::<CfgEntry>();
        let bytes = self.bytes();
        if start + len > bytes.len() {
            return &[];
        }
        unsafe { std::slice::from_raw_parts(bytes[start..].as_ptr() as *const CfgEntry, count) }
    }

    /// Iterates over all `(key, Option<value>)` `cfg` predicates in sorted order.
    pub fn cfg_iter(&self) -> impl ExactSizeIterator<Item = (&str, Option<&str>)> + '_ {
        self.cfgs().iter().map(|raw| {
            let e = raw.from_le();
            let k = self.get_str(e.key_offset);
            let v = if e.value_offset == NO_CFG_VALUE { None } else { Some(self.get_str(e.value_offset)) };
            (k, v)
        })
    }

    /// `O(log N)` lookup for a bare `cfg` flag atom (`#[cfg(unix)]`, `#[cfg(test)]`).
    pub fn has_cfg_flag(&self, key: &str) -> bool {
        let entries = self.cfgs();
        entries
            .binary_search_by(|raw| {
                let e = raw.from_le();
                let ek = self.get_str(e.key_offset);
                let ev: Option<&str> = if e.value_offset == NO_CFG_VALUE { None } else { Some(self.get_str(e.value_offset)) };
                (ek, ev).cmp(&(key, None))
            })
            .is_ok()
    }

    /// `O(log N)` lookup for a `cfg(key = "value")` predicate (`#[cfg(feature = "std")]`).
    pub fn has_cfg_key_value(&self, key: &str, value: &str) -> bool {
        let entries = self.cfgs();
        entries
            .binary_search_by(|raw| {
                let e = raw.from_le();
                let ek = self.get_str(e.key_offset);
                let ev: Option<&str> = if e.value_offset == NO_CFG_VALUE { None } else { Some(self.get_str(e.value_offset)) };
                (ek, ev).cmp(&(key, Some(value)))
            })
            .is_ok()
    }

    /// `O(log N)` zero-allocation iterator over all values `v` of `cfg(key = "v")` in sorted order.
    pub fn cfg_values<'a>(&'a self, key: &str) -> impl ExactSizeIterator<Item = &'a str> + 'a {
        let entries = self.cfgs();
        let start = entries.partition_point(|raw| {
            let e = raw.from_le();
            let ek = self.get_str(e.key_offset);
            match ek.cmp(key) {
                std::cmp::Ordering::Less => true,
                std::cmp::Ordering::Equal => e.value_offset == NO_CFG_VALUE,
                std::cmp::Ordering::Greater => false,
            }
        });
        let end = entries[start..].partition_point(|raw| {
            let e = raw.from_le();
            self.get_str(e.key_offset) == key
        }) + start;
        entries[start..end].iter().map(move |raw| self.get_str(raw.from_le().value_offset))
    }

    /// `O(log N)` zero-allocation iterator over all enabled Cargo features (`feature = "..."`).
    #[inline]
    pub fn enabled_features(&self) -> impl ExactSizeIterator<Item = &str> + '_ {
        self.cfg_values("feature")
    }
}
