/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! On-disk `#[repr(C)]` layout for structured ADT/variant fields (`FieldEntry`) and
//! definition value/alias/constructor types (`DefTyEntry`) in `.ridx` format version 5.

use super::TyId;

/// `FieldEntry.flags`: positional tuple field (`.0`, `.1`, ...).
pub const FIELD_FLAG_TUPLE: u8 = 1 << 0;
/// `FieldEntry.flags`: `unsafe` field (`FieldDef.safety.is_unsafe()`).
pub const FIELD_FLAG_UNSAFE: u8 = 1 << 1;

/// `DefTyEntry.flags`: `#[non_exhaustive]` on `Enum` variant list or `Struct`/`Variant` field list.
pub const DEF_TY_FLAG_NON_EXHAUSTIVE: u8 = 1 << 0;
/// `DefTyEntry.flags`: `static mut` (`Mutability::Mut`).
pub const DEF_TY_FLAG_MUT_STATIC: u8 = 1 << 1;
/// `DefTyEntry.flags`: `unsafe` extern static (`Safety::Unsafe`).
pub const DEF_TY_FLAG_UNSAFE_STATIC: u8 = 1 << 2;
/// `DefTyEntry.flags`: `#[fundamental]` ADT (`Box<T>`, `Pin<P>`).
pub const DEF_TY_FLAG_FUNDAMENTAL: u8 = 1 << 3;

/// Visibility level matching `DefEntry.visibility` (`0 = Public, 1 = Crate, 2 = Restricted, 3 = Private`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[repr(u8)]
pub enum ItemVis {
    /// `pub`: visible everywhere including downstream crates.
    #[default]
    Public = 0,
    /// `pub(crate)`: visible throughout the defining crate.
    Crate = 1,
    /// `pub(super)` / `pub(in ancestor)`: visible in an ancestor module within the defining crate.
    Restricted = 2,
    /// Private to the enclosing module.
    Private = 3,
}

impl ItemVis {
    #[inline]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Public,
            1 => Self::Crate,
            2 => Self::Restricted,
            _ => Self::Private,
        }
    }

    #[inline]
    pub const fn is_public(self) -> bool {
        matches!(self, Self::Public)
    }

    #[inline]
    pub const fn is_crate_visible(self) -> bool {
        matches!(self, Self::Public | Self::Crate)
    }
}

/// Syntactic/constructor shape of a `Struct`, `Union`, or `Enum` `Variant`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[repr(u8)]
pub enum CtorShape {
    /// Not a value/record constructor (`TyAlias`, `Const`, `Static`, `AssocConst`, `Enum`).
    #[default]
    None = 0,
    /// Braced record (`struct Point { x: i32 }`, `Enum::Struct { a: u8 }`, `union Raw { ... }`).
    Record = 1,
    /// Tuple constructor (`struct Token(u64);`, `Option::Some(T)`); also indexed in `fn_sigs`.
    Tuple = 2,
    /// Unit constructor (`struct Marker;`, `Option::None`).
    Unit = 3,
}

impl CtorShape {
    #[inline]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Record,
            2 => Self::Tuple,
            3 => Self::Unit,
            _ => Self::None,
        }
    }
}

/// Structured field row for a `Struct`, `Union`, or `Enum` `Variant` (`20` bytes, `4`-byte aligned).
///
/// Sorted on disk by `(owner_def_index, field_idx)` so all fields of an ADT or enum variant
/// form a contiguous slice discoverable via `O(log N)` binary search.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FieldEntry {
    /// Local `DefEntry` index of the owning `Struct`, `Union`, or `Variant`.
    pub owner_def_index: u32,
    /// Local `DefEntry` index of this `Field` (`u32::MAX` when no `DefEntry` was emitted).
    pub def_index: u32,
    /// Field identifier offset in `string_pool` (`"x"`, `"0"`, `"1"`, ...).
    pub name_offset: u32,
    /// Field `TyId` in `ty_entries` (in the owning ADT's generic parameter namespace).
    pub ty: u32,
    /// `0`-based declaration position inside the struct/union/variant.
    pub field_idx: u16,
    /// `ItemVis` (`0 = Public, 1 = Crate, 2 = Restricted, 3 = Private`).
    pub vis: u8,
    /// `FIELD_FLAG_*` bitmask (`FIELD_FLAG_TUPLE`, `FIELD_FLAG_UNSAFE`).
    pub flags: u8,
}

impl FieldEntry {
    #[inline]
    pub const fn to_le(self) -> Self {
        Self {
            owner_def_index: self.owner_def_index.to_le(),
            def_index: self.def_index.to_le(),
            name_offset: self.name_offset.to_le(),
            ty: self.ty.to_le(),
            field_idx: self.field_idx.to_le(),
            vis: self.vis,
            flags: self.flags,
        }
    }

    #[inline]
    pub const fn from_le(self) -> Self {
        Self {
            owner_def_index: u32::from_le(self.owner_def_index),
            def_index: u32::from_le(self.def_index),
            name_offset: u32::from_le(self.name_offset),
            ty: u32::from_le(self.ty),
            field_idx: u16::from_le(self.field_idx),
            vis: self.vis,
            flags: self.flags,
        }
    }

    #[inline]
    pub const fn owner_def_index(&self) -> u32 {
        u32::from_le(self.owner_def_index)
    }

    #[inline]
    pub const fn def_index_opt(&self) -> Option<u32> {
        let idx = u32::from_le(self.def_index);
        if idx == u32::MAX {
            None
        } else {
            Some(idx)
        }
    }

    #[inline]
    pub const fn ty_id(&self) -> TyId {
        TyId(u32::from_le(self.ty))
    }

    #[inline]
    pub const fn field_idx(&self) -> u16 {
        u16::from_le(self.field_idx)
    }

    #[inline]
    pub const fn visibility(&self) -> ItemVis {
        ItemVis::from_u8(self.vis)
    }

    #[inline]
    pub const fn is_tuple_field(&self) -> bool {
        (self.flags & FIELD_FLAG_TUPLE) != 0
    }

    #[inline]
    pub const fn is_unsafe(&self) -> bool {
        (self.flags & FIELD_FLAG_UNSAFE) != 0
    }
}

/// Structured type/constructor descriptor keyed by `def_index` (`16` bytes, `4`-byte aligned).
///
/// Covers `TyAlias` (RHS target `TyId`), `Const`/`Static`/`AssocConst` (value `TyId`),
/// `Struct`/`Union`/`Enum` (identity ADT `TyId` + `CtorShape`), and `Variant` (owning `Enum`
/// identity `TyId`, `parent_def_index`, `variant_idx`, and `CtorShape`).
/// Sorted on disk by `def_index` for `O(log N)` binary search.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DefTyEntry {
    /// Local `DefEntry` index of this definition (sorted primary key).
    pub def_index: u32,
    /// `TyId` in `ty_entries` (RHS for `TyAlias`, value type for constants/statics, ADT type for ADTs/variants).
    pub ty: u32,
    /// Owning `Enum` `DefEntry` index for `Variant` (`u32::MAX` for non-variants).
    pub parent_def_index: u32,
    /// `0`-based `VariantIdx` (`0..N` for `Variant`, `0` for `Struct`/`Union`, `u16::MAX` otherwise).
    pub variant_idx: u16,
    /// `CtorShape` (`None = 0`, `Record = 1`, `Tuple = 2`, `Unit = 3`).
    pub ctor_shape: u8,
    /// `DEF_TY_FLAG_*` bitmask.
    pub flags: u8,
}

impl DefTyEntry {
    #[inline]
    pub const fn to_le(self) -> Self {
        Self {
            def_index: self.def_index.to_le(),
            ty: self.ty.to_le(),
            parent_def_index: self.parent_def_index.to_le(),
            variant_idx: self.variant_idx.to_le(),
            ctor_shape: self.ctor_shape,
            flags: self.flags,
        }
    }

    #[inline]
    pub const fn from_le(self) -> Self {
        Self {
            def_index: u32::from_le(self.def_index),
            ty: u32::from_le(self.ty),
            parent_def_index: u32::from_le(self.parent_def_index),
            variant_idx: u16::from_le(self.variant_idx),
            ctor_shape: self.ctor_shape,
            flags: self.flags,
        }
    }

    #[inline]
    pub const fn def_index(&self) -> u32 {
        u32::from_le(self.def_index)
    }

    #[inline]
    pub const fn ty_id(&self) -> TyId {
        TyId(u32::from_le(self.ty))
    }

    #[inline]
    pub const fn parent_def_index_opt(&self) -> Option<u32> {
        let p = u32::from_le(self.parent_def_index);
        if p == u32::MAX {
            None
        } else {
            Some(p)
        }
    }

    #[inline]
    pub const fn variant_idx_opt(&self) -> Option<u16> {
        let v = u16::from_le(self.variant_idx);
        if v == u16::MAX {
            None
        } else {
            Some(v)
        }
    }

    #[inline]
    pub const fn ctor_shape(&self) -> CtorShape {
        CtorShape::from_u8(self.ctor_shape)
    }
}
