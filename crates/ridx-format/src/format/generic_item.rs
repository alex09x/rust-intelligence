/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! On-disk `#[repr(C)]` layout for structured generic parameter declarations (`GenericParamEntry`)
//! and `where` / bound / supertrait / projection predicates (`WherePredEntry`) in `.ridx` v5.

use super::ty_prim::{TyDefId, TyId};

/// High-bit tag in `owner_key` distinguishing `ImplEntry` owners (`0x8000_0000 | impl_index`)
/// from `DefEntry` owners (`0..0x7fff_ffff`).
pub const GENERIC_OWNER_IMPL_BIT: u32 = 1 << 31;

/// Owner of a `GenericParamEntry` or `WherePredEntry` row (`DefEntry` or `ImplEntry`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GenericOwner {
    /// Local `DefEntry` index (`0..2^31`).
    Def(u32),
    /// Shard `ImplEntry` index (`0..impls().len()`), including metadata-only sysroot impls.
    Impl(u32),
}

impl GenericOwner {
    #[inline]
    pub const fn to_raw(self) -> u32 {
        match self {
            Self::Def(idx) => idx & !GENERIC_OWNER_IMPL_BIT,
            Self::Impl(idx) => GENERIC_OWNER_IMPL_BIT | (idx & !GENERIC_OWNER_IMPL_BIT),
        }
    }

    #[inline]
    pub const fn from_raw(raw: u32) -> Self {
        if (raw & GENERIC_OWNER_IMPL_BIT) != 0 {
            Self::Impl(raw & !GENERIC_OWNER_IMPL_BIT)
        } else {
            Self::Def(raw)
        }
    }
}

/// Discriminator of a generic parameter (`1` byte).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GenericParamKind {
    #[default]
    Lifetime = 0,
    Type = 1,
    Const = 2,
}

impl GenericParamKind {
    #[inline]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Lifetime,
            1 => Self::Type,
            2 => Self::Const,
            _ => Self::Type,
        }
    }
}

/// `GenericParamEntry.flags`: parameter declares a default (`type T = U` or `const N: usize = 4`).
pub const GENERIC_PARAM_FLAG_HAS_DEFAULT: u8 = 1 << 0;
/// `GenericParamEntry.flags`: synthetic `impl Trait` in argument position (APIT).
pub const GENERIC_PARAM_FLAG_SYNTHETIC: u8 = 1 << 1;
/// `GenericParamEntry.flags`: implicit `Self` parameter (`index == 0`) on a `Trait` or `TraitAlias`.
pub const GENERIC_PARAM_FLAG_IS_SELF: u8 = 1 << 2;
/// `GenericParamEntry.flags`: `Type` parameter has `Sized` bound (unset when `?Sized`).
pub const GENERIC_PARAM_FLAG_SIZED: u8 = 1 << 3;
/// `GenericParamEntry.flags`: compiler host/const effect parameter.
pub const GENERIC_PARAM_FLAG_HOST_EFFECT: u8 = 1 << 4;

/// Structured generic parameter declaration (`20` bytes, `4`-byte aligned).
///
/// Sorted on disk by `(owner_key, param_idx)` for `O(log N)` binary search per `DefEntry` or `ImplEntry`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GenericParamEntry {
    /// Encoded `GenericOwner` (`Def(def_idx)` or `Impl(impl_idx)`).
    pub owner_key: u32,
    /// Local `DefEntry` index of this parameter if present in `defs`, or `u32::MAX`.
    pub def_index: u32,
    /// Parameter name offset in `string_pool` (e.g. `"T"`, `"'a"`, `"N"`, `"Self"`).
    pub name_offset: u32,
    /// For `Type` with `HAS_DEFAULT`: default `TyId` (`u32::MAX` if no default).
    /// For `Const`: value `TyId` of the const parameter (e.g. `usize`).
    /// For `Lifetime`: `u32::MAX`.
    pub ty_or_default: u32,
    /// Exact `rustc` `GenericParamDef.index` matching `TyKindView::Param { index, .. }`.
    pub param_idx: u16,
    /// `GenericParamKind` (`0 = Lifetime, 1 = Type, 2 = Const`).
    pub kind: u8,
    /// `GENERIC_PARAM_FLAG_*` bitmask.
    pub flags: u8,
}

impl GenericParamEntry {
    #[inline]
    pub const fn to_le(self) -> Self {
        Self {
            owner_key: self.owner_key.to_le(),
            def_index: self.def_index.to_le(),
            name_offset: self.name_offset.to_le(),
            ty_or_default: self.ty_or_default.to_le(),
            param_idx: self.param_idx.to_le(),
            kind: self.kind,
            flags: self.flags,
        }
    }

    #[inline]
    pub const fn from_le(self) -> Self {
        Self {
            owner_key: u32::from_le(self.owner_key),
            def_index: u32::from_le(self.def_index),
            name_offset: u32::from_le(self.name_offset),
            ty_or_default: u32::from_le(self.ty_or_default),
            param_idx: u16::from_le(self.param_idx),
            kind: self.kind,
            flags: self.flags,
        }
    }

    #[inline]
    pub const fn owner_key(&self) -> u32 {
        u32::from_le(self.owner_key)
    }

    #[inline]
    pub const fn param_idx(&self) -> u16 {
        u16::from_le(self.param_idx)
    }
}

/// Discriminator of a `WherePredEntry` clause (`1` byte).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WherePredKind {
    #[default]
    TraitBound = 0,
    ProjectionEq = 1,
    TypeOutlives = 2,
    RegionOutlives = 3,
}

impl WherePredKind {
    #[inline]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::TraitBound,
            1 => Self::ProjectionEq,
            2 => Self::TypeOutlives,
            3 => Self::RegionOutlives,
            _ => Self::TraitBound,
        }
    }
}

/// `WherePredEntry.flags`: higher-ranked binder `for<'a>` wraps this predicate.
pub const WHERE_FLAG_HRTB: u8 = 1 << 0;
/// `WherePredEntry.flags`: negative polarity (`T: !Trait`).
pub const WHERE_FLAG_NEGATIVE: u8 = 1 << 1;
/// `WherePredEntry.flags`: `const` trait bound (`T: ~const Trait` / `const Trait`).
pub const WHERE_FLAG_CONST: u8 = 1 << 2;

/// Structured `where` / inline bound / supertrait / assoc-item bound predicate (`28` bytes, `4`-byte aligned).
///
/// Sorted on disk by `owner_key` (`GenericOwner::to_raw()`), preserving declaration order per owner.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WherePredEntry {
    /// Encoded `GenericOwner` (`Def(def_idx)` or `Impl(impl_idx)`).
    pub owner_key: u32,
    /// Constrained `TyId` (`TraitBound`, `ProjectionEq` `self_ty`, `TypeOutlives`), or lhs region (`RegionOutlives`).
    pub lhs_ty: u32,
    /// Target trait `TyDefId` (`TraitBound` / `ProjectionEq`), or `u32::MAX` for outlives clauses.
    pub trait_def: u32,
    /// Start index in `ty_args` for `trait_def`'s generic args excluding `Self` (`0` if empty).
    pub trait_args_start: u32,
    /// For `ProjectionEq`: RHS `TyId` (`u32::MAX` if non-type const term).
    /// For `TypeOutlives` / `RegionOutlives`: target lifetime (`LIFETIME_STATIC`, `LIFETIME_ELIDED`, or region index).
    /// For `TraitBound`: `u32::MAX`.
    pub rhs_or_region: u32,
    /// Associated item name offset in `string_pool` for `ProjectionEq` (e.g. `"Item"`, `"Output"`), else `u32::MAX`.
    pub assoc_name_offset: u32,
    /// Number of `GenericArgWord`s starting at `trait_args_start`.
    pub trait_args_len: u16,
    /// `WherePredKind` (`0..=3`).
    pub kind: u8,
    /// `WHERE_FLAG_*` bitmask.
    pub flags: u8,
}

impl WherePredEntry {
    #[inline]
    pub const fn to_le(self) -> Self {
        Self {
            owner_key: self.owner_key.to_le(),
            lhs_ty: self.lhs_ty.to_le(),
            trait_def: self.trait_def.to_le(),
            trait_args_start: self.trait_args_start.to_le(),
            rhs_or_region: self.rhs_or_region.to_le(),
            assoc_name_offset: self.assoc_name_offset.to_le(),
            trait_args_len: self.trait_args_len.to_le(),
            kind: self.kind,
            flags: self.flags,
        }
    }

    #[inline]
    pub const fn from_le(self) -> Self {
        Self {
            owner_key: u32::from_le(self.owner_key),
            lhs_ty: u32::from_le(self.lhs_ty),
            trait_def: u32::from_le(self.trait_def),
            trait_args_start: u32::from_le(self.trait_args_start),
            rhs_or_region: u32::from_le(self.rhs_or_region),
            assoc_name_offset: u32::from_le(self.assoc_name_offset),
            trait_args_len: u16::from_le(self.trait_args_len),
            kind: self.kind,
            flags: self.flags,
        }
    }

    #[inline]
    pub const fn owner_key(&self) -> u32 {
        u32::from_le(self.owner_key)
    }

    #[inline]
    pub const fn trait_def_opt(&self) -> Option<TyDefId> {
        let d = u32::from_le(self.trait_def);
        if d == u32::MAX {
            None
        } else {
            Some(TyDefId(d))
        }
    }

    #[inline]
    pub const fn lhs_ty_id(&self) -> TyId {
        TyId(u32::from_le(self.lhs_ty))
    }
}

const _: () = {
    assert!(std::mem::size_of::<GenericParamEntry>() == 20);
    assert!(std::mem::align_of::<GenericParamEntry>() == 4);
    assert!(std::mem::size_of::<WherePredEntry>() == 28);
    assert!(std::mem::align_of::<WherePredEntry>() == 4);
};
