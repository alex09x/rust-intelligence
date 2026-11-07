/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Binary layout for active per-crate `cfg` predicates (`#[cfg(...)]` and `--cfg feature="..."`).

/// Sentinel `value_offset` indicating a bare atom predicate (`#[cfg(unix)]`, `#[cfg(test)]`)
/// as opposed to a key-value predicate (`#[cfg(feature = "std")]`, including empty `""`).
pub const NO_CFG_VALUE: u32 = u32::MAX;

/// Fixed-size 8-byte `cfg` table entry (`align 4`, zero padding).
///
/// Entries in the `cfgs` section are sorted lexicographically by
/// `(key_str, has_value_bool, value_str)` so bare atoms `cfg(key)` precede
/// `cfg(key = "...")` and `cfg_values(key)` forms a contiguous range for `O(log N)` lookup.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CfgEntry {
    pub key_offset: u32,
    pub value_offset: u32,
}

impl CfgEntry {
    #[inline]
    pub const fn new_flag(key_offset: u32) -> Self {
        Self { key_offset, value_offset: NO_CFG_VALUE }
    }

    #[inline]
    pub const fn new_key_value(key_offset: u32, value_offset: u32) -> Self {
        Self { key_offset, value_offset }
    }

    #[inline]
    pub const fn has_value(self) -> bool {
        self.value_offset != NO_CFG_VALUE
    }

    #[inline]
    pub const fn key_offset(self) -> u32 {
        u32::from_le(self.key_offset)
    }

    #[inline]
    pub const fn value_offset_opt(self) -> Option<u32> {
        let v = u32::from_le(self.value_offset);
        if v == NO_CFG_VALUE {
            None
        } else {
            Some(v)
        }
    }

    #[inline]
    pub const fn to_le(self) -> Self {
        Self { key_offset: self.key_offset.to_le(), value_offset: self.value_offset.to_le() }
    }

    #[inline]
    pub const fn from_le(self) -> Self {
        Self { key_offset: u32::from_le(self.key_offset), value_offset: u32::from_le(self.value_offset) }
    }
}
