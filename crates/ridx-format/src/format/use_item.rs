/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! On-disk `UseOwner`, `UsePrimTy`, flags, and 28-byte `UseEntry` record for `.ridx` format v5 (1.2-G7).

use super::field_item::ItemVis;
use super::ty_prim::TyDefId;

/// Scope owning a `UseEntry` (`CrateRoot` = `0`, `Def(def_index)` = `def_index + 1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UseOwner {
    /// Crate root module (`lib.rs` / `main.rs`, or `core` / `alloc` / `std` root).
    CrateRoot,
    /// Enclosing `DefEntry` index (`DefKind::Mod` for submodules, or item/fn scope for local `use`).
    Def(u32),
}

impl UseOwner {
    #[inline]
    pub const fn to_raw(self) -> u32 {
        match self {
            Self::CrateRoot => 0,
            Self::Def(idx) => idx.saturating_add(1),
        }
    }

    #[inline]
    pub const fn from_raw(raw: u32) -> Self {
        if raw == 0 {
            Self::CrateRoot
        } else {
            Self::Def(raw - 1)
        }
    }
}

/// Built-in primitive target for `use core::primitive::*` / `std::primitive::*` imports.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum UsePrimTy {
    #[default]
    None = 0,
    Bool = 1,
    Char = 2,
    Str = 3,
    I8 = 4,
    I16 = 5,
    I32 = 6,
    I64 = 7,
    I128 = 8,
    Isize = 9,
    U8 = 10,
    U16 = 11,
    U32 = 12,
    U64 = 13,
    U128 = 14,
    Usize = 15,
    F16 = 16,
    F32 = 17,
    F64 = 18,
    F128 = 19,
}

impl UsePrimTy {
    #[inline]
    pub const fn from_u8(v: u8) -> Self {
        if v <= 19 {
            // SAFETY: `UsePrimTy` is `#[repr(u8)]` with contiguous variants `0..=19`.
            unsafe { std::mem::transmute::<u8, Self>(v) }
        } else {
            Self::None
        }
    }

    #[inline]
    pub const fn as_str(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Bool => Some("bool"),
            Self::Char => Some("char"),
            Self::Str => Some("str"),
            Self::I8 => Some("i8"),
            Self::I16 => Some("i16"),
            Self::I32 => Some("i32"),
            Self::I64 => Some("i64"),
            Self::I128 => Some("i128"),
            Self::Isize => Some("isize"),
            Self::U8 => Some("u8"),
            Self::U16 => Some("u16"),
            Self::U32 => Some("u32"),
            Self::U64 => Some("u64"),
            Self::U128 => Some("u128"),
            Self::Usize => Some("usize"),
            Self::F16 => Some("f16"),
            Self::F32 => Some("f32"),
            Self::F64 => Some("f64"),
            Self::F128 => Some("f128"),
        }
    }
}

/// Glob import or glob re-export (`use path::*;`, `name == "*"`, target `Mod`/`Enum` in `type_def`).
pub const USE_FLAG_GLOB: u8 = 1 << 0;
/// Exported / re-exported binding (`pub use`, `pub(crate) use`, `ModChild` re-export, or `#[macro_export]`).
pub const USE_FLAG_REEXPORT: u8 = 1 << 1;
/// `extern crate foo [as bar];` binding (`type_def` points to target crate root `Mod`).
pub const USE_FLAG_EXTERN_CRATE: u8 = 1 << 2;
/// Anonymous trait import (`use Trait as _;`, `name == "_"`).
pub const USE_FLAG_UNDERSCORE: u8 = 1 << 3;
/// `prim_tag` carries a non-zero `UsePrimTy` in the type namespace.
pub const USE_FLAG_PRIM_TY: u8 = 1 << 4;
/// Compiler-injected `#[prelude_import]` crate-wide prelude glob (`std::prelude::rust_2021::*`, etc.).
pub const USE_FLAG_PRELUDE: u8 = 1 << 5;

/// Fixed-size 28-byte module `use` / glob / re-export / `extern crate` row in `uses`,
/// sorted by `(owner_raw, name_str, ...)` for O(log N) lookup by owner and identifier.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct UseEntry {
    pub owner_raw: u32,
    pub def_index: u32,
    pub name_offset: u32,
    pub type_def: u32,
    pub value_def: u32,
    pub macro_def: u32,
    pub vis: u8,
    pub flags: u8,
    pub prim_tag: u8,
    pub _pad: u8,
}

impl UseEntry {
    #[inline]
    pub const fn to_le(self) -> Self {
        Self {
            owner_raw: self.owner_raw.to_le(),
            def_index: self.def_index.to_le(),
            name_offset: self.name_offset.to_le(),
            type_def: self.type_def.to_le(),
            value_def: self.value_def.to_le(),
            macro_def: self.macro_def.to_le(),
            vis: self.vis,
            flags: self.flags,
            prim_tag: self.prim_tag,
            _pad: 0,
        }
    }

    #[inline]
    pub const fn from_le(self) -> Self {
        Self {
            owner_raw: u32::from_le(self.owner_raw),
            def_index: u32::from_le(self.def_index),
            name_offset: u32::from_le(self.name_offset),
            type_def: u32::from_le(self.type_def),
            value_def: u32::from_le(self.value_def),
            macro_def: u32::from_le(self.macro_def),
            vis: self.vis,
            flags: self.flags,
            prim_tag: self.prim_tag,
            _pad: 0,
        }
    }

    #[inline]
    pub const fn owner_raw(&self) -> u32 {
        u32::from_le(self.owner_raw)
    }

    #[inline]
    pub const fn owner(&self) -> UseOwner {
        UseOwner::from_raw(self.owner_raw())
    }

    #[inline]
    pub const fn def_index(&self) -> Option<u32> {
        let v = u32::from_le(self.def_index);
        if v == u32::MAX {
            None
        } else {
            Some(v)
        }
    }

    #[inline]
    pub const fn name_offset(&self) -> u32 {
        u32::from_le(self.name_offset)
    }

    #[inline]
    pub const fn type_def(&self) -> Option<TyDefId> {
        let v = u32::from_le(self.type_def);
        if v == u32::MAX {
            None
        } else {
            Some(TyDefId(v))
        }
    }

    #[inline]
    pub const fn value_def(&self) -> Option<TyDefId> {
        let v = u32::from_le(self.value_def);
        if v == u32::MAX {
            None
        } else {
            Some(TyDefId(v))
        }
    }

    #[inline]
    pub const fn macro_def(&self) -> Option<TyDefId> {
        let v = u32::from_le(self.macro_def);
        if v == u32::MAX {
            None
        } else {
            Some(TyDefId(v))
        }
    }

    #[inline]
    pub const fn visibility(&self) -> ItemVis {
        ItemVis::from_u8(self.vis)
    }

    #[inline]
    pub const fn prim_ty(&self) -> UsePrimTy {
        UsePrimTy::from_u8(self.prim_tag)
    }
}
