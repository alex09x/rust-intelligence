/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Binary layout for declarative macro definitions (`macro_rules!` and Rust 2.0 `macro`).

/// Macro was defined with `macro_rules!` (`true`) vs declarative `macro` 2.0 (`false`).
pub const MACRO_FLAG_MACRO_RULES: u8 = 1 << 0;
/// Macro carries `#[macro_export]`.
pub const MACRO_FLAG_EXPORTED: u8 = 1 << 1;
/// Macro carries `#[macro_export(local_inner_macros)]`.
pub const MACRO_FLAG_LOCAL_INNER: u8 = 1 << 2;
/// Macro carries `#[rustc_builtin_macro]` (compiler built-in expansion stub in `core`/`std`).
pub const MACRO_FLAG_BUILTIN: u8 = 1 << 3;

/// Outer delimiter of the macro definition body (`macro_rules! foo { ... }` / `( ... )` / `[ ... ]`).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MacroDelim {
    #[default]
    Brace = 0,
    Paren = 1,
    Bracket = 2,
}

impl MacroDelim {
    #[inline]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Paren,
            2 => Self::Bracket,
            _ => Self::Brace,
        }
    }
}

/// Hygiene edition of the crate that defined the declarative macro.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MacroEdition {
    Edition2015 = 0,
    Edition2018 = 1,
    #[default]
    Edition2021 = 2,
    Edition2024 = 3,
}

impl MacroEdition {
    #[inline]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Edition2015,
            1 => Self::Edition2018,
            2 => Self::Edition2021,
            _ => Self::Edition2024,
        }
    }
}

/// Fixed-size 12-byte declarative macro table entry (`align 4`, zero wasted alignment gaps).
///
/// Entries in the `macro_defs` section are sorted strictly by `def_index` for `O(log N)` lookup.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MacroDefEntry {
    pub def_index: u32,
    pub body_offset: u32,
    pub flags: u8,
    pub delim: u8,
    pub edition: u8,
    pub _pad: u8,
}

impl MacroDefEntry {
    #[inline]
    pub const fn new(def_index: u32, body_offset: u32, flags: u8, delim: MacroDelim, edition: MacroEdition) -> Self {
        Self { def_index, body_offset, flags, delim: delim as u8, edition: edition as u8, _pad: 0 }
    }

    #[inline]
    pub const fn def_index(self) -> u32 {
        u32::from_le(self.def_index)
    }

    #[inline]
    pub const fn body_offset(self) -> u32 {
        u32::from_le(self.body_offset)
    }

    #[inline]
    pub const fn is_macro_rules(self) -> bool {
        (self.flags & MACRO_FLAG_MACRO_RULES) != 0
    }

    #[inline]
    pub const fn is_exported(self) -> bool {
        (self.flags & MACRO_FLAG_EXPORTED) != 0
    }

    #[inline]
    pub const fn has_local_inner_macros(self) -> bool {
        (self.flags & MACRO_FLAG_LOCAL_INNER) != 0
    }

    #[inline]
    pub const fn is_builtin(self) -> bool {
        (self.flags & MACRO_FLAG_BUILTIN) != 0
    }

    #[inline]
    pub const fn delim_kind(self) -> MacroDelim {
        MacroDelim::from_u8(self.delim)
    }

    #[inline]
    pub const fn edition_kind(self) -> MacroEdition {
        MacroEdition::from_u8(self.edition)
    }

    #[inline]
    pub const fn to_le(self) -> Self {
        Self { def_index: self.def_index.to_le(), body_offset: self.body_offset.to_le(), flags: self.flags, delim: self.delim, edition: self.edition, _pad: 0 }
    }

    #[inline]
    pub const fn from_le(self) -> Self {
        Self { def_index: u32::from_le(self.def_index), body_offset: u32::from_le(self.body_offset), flags: self.flags, delim: self.delim, edition: self.edition, _pad: 0 }
    }
}
