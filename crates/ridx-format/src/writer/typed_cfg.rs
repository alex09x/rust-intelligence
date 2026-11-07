/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `IndexWriter` builders and deterministic sorting for `CfgEntry` and target triple metadata.

use super::IndexWriter;
use crate::format::{CfgEntry, NO_CFG_VALUE};

impl IndexWriter {
    /// Sets the compilation target triple (e.g. `x86_64-unknown-linux-gnu`) for this shard.
    pub fn set_target_triple(&mut self, triple: impl Into<String>) {
        let s = triple.into();
        if s.is_empty() {
            self.target_triple = None;
        } else {
            self.target_triple = Some(s);
        }
    }

    /// Adds a bare `cfg` flag atom (e.g. `unix`, `test`, `debug_assertions`).
    pub fn add_cfg_flag(&mut self, key: &str) {
        self.add_cfg(key, None);
    }

    /// Adds a `cfg(key = "value")` predicate (e.g. `("feature", "serde")`, `("target_os", "linux")`).
    pub fn add_cfg_key_value(&mut self, key: &str, value: &str) {
        self.add_cfg(key, Some(value));
    }

    /// Adds an active `cfg` predicate (`None` for bare atom, `Some(val)` for `key = "val"`).
    pub fn add_cfg(&mut self, key: &str, value: Option<&str>) {
        if key.is_empty() {
            return;
        }
        let key_offset = self.intern_string(key);
        let value_offset = match value {
            Some(v) => self.intern_string(v),
            None => NO_CFG_VALUE,
        };
        self.cfgs.push(CfgEntry { key_offset, value_offset });
    }

    /// Total count of raw `CfgEntry` rows queued before deduplication.
    pub fn cfg_count(&self) -> usize {
        self.cfgs.len()
    }

    pub(super) fn prepare_sorted_cfgs(&self) -> Vec<CfgEntry> {
        let mut list = self.cfgs.clone();
        list.sort_by(|a, b| {
            let ka = self.get_interned_str(a.key_offset);
            let kb = self.get_interned_str(b.key_offset);
            let va = if a.value_offset == NO_CFG_VALUE { None } else { Some(self.get_interned_str(a.value_offset)) };
            let vb = if b.value_offset == NO_CFG_VALUE { None } else { Some(self.get_interned_str(b.value_offset)) };
            (ka, va).cmp(&(kb, vb))
        });
        list.dedup_by(|a, b| a.key_offset == b.key_offset && a.value_offset == b.value_offset);
        list
    }
}
