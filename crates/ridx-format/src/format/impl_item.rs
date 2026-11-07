/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! On-disk `#[repr(C)]` layout for structured `impl` headers (`ImplEntry`) and
//! associated type bindings (`AssocTyEntry`) in `.ridx` format version 5.

use super::ty::{TyEntry, TyTag};
use super::ty_prim::TyMutability;

/// `ImplEntry.flags`: `impl Trait for Self` (unset for inherent `impl Self`).
pub const IMPL_FLAG_TRAIT: u8 = 1 << 0;
/// `ImplEntry.flags`: negative polarity (`impl !Send for Self`).
pub const IMPL_FLAG_NEGATIVE: u8 = 1 << 1;
/// `ImplEntry.flags`: `unsafe impl Trait for Self`.
pub const IMPL_FLAG_UNSAFE: u8 = 1 << 2;
/// `ImplEntry.flags`: compiler reservation impl (`#[rustc_reservation_impl]`).
pub const IMPL_FLAG_RESERVATION: u8 = 1 << 3;
/// `ImplEntry.flags`: `impl const Trait for Self`.
pub const IMPL_FLAG_CONST: u8 = 1 << 4;

/// Coarse head-constructor discriminator for `ImplEntry.self_ty` (`1` byte).
///
/// Crucially preserves exact type constructors without `peel_refs`:
/// `&T` (`RefShared`), `&mut T` (`RefMut`), blanket `T` (`Param`), tuples, and `fn` pointers
/// each occupy their own distinct bucket so `IndexReader` can binary-search impls in `O(log N)`.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SimplifiedSelfKind {
    Adt = 0,
    Bool = 1,
    Char = 2,
    Int = 3,
    Uint = 4,
    Float = 5,
    Str = 6,
    Slice = 7,
    Array = 8,
    Tuple = 9,
    RefShared = 10,
    RefMut = 11,
    PtrConst = 12,
    PtrMut = 13,
    FnPtr = 14,
    Dynamic = 15,
    Never = 16,
    Param = 17,
    Unit = 18,
    Other = 19,
}

impl SimplifiedSelfKind {
    #[inline]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Adt,
            1 => Self::Bool,
            2 => Self::Char,
            3 => Self::Int,
            4 => Self::Uint,
            5 => Self::Float,
            6 => Self::Str,
            7 => Self::Slice,
            8 => Self::Array,
            9 => Self::Tuple,
            10 => Self::RefShared,
            11 => Self::RefMut,
            12 => Self::PtrConst,
            13 => Self::PtrMut,
            14 => Self::FnPtr,
            15 => Self::Dynamic,
            16 => Self::Never,
            17 => Self::Param,
            18 => Self::Unit,
            _ => Self::Other,
        }
    }

    #[inline]
    pub const fn from_ty_entry(entry: TyEntry) -> Self {
        match TyTag::from_u8(entry.tag) {
            TyTag::Adt => Self::Adt,
            TyTag::Bool => Self::Bool,
            TyTag::Char => Self::Char,
            TyTag::Int => Self::Int,
            TyTag::Uint => Self::Uint,
            TyTag::Float => Self::Float,
            TyTag::Str => Self::Str,
            TyTag::Slice => Self::Slice,
            TyTag::Array => Self::Array,
            TyTag::Tuple => Self::Tuple,
            TyTag::Ref => match TyMutability::from_u8(entry.flags) {
                TyMutability::Shared => Self::RefShared,
                TyMutability::Mut => Self::RefMut,
            },
            TyTag::Ptr => match TyMutability::from_u8(entry.flags) {
                TyMutability::Shared => Self::PtrConst,
                TyMutability::Mut => Self::PtrMut,
            },
            TyTag::FnPtr => Self::FnPtr,
            TyTag::Dyn => Self::Dynamic,
            TyTag::Never => Self::Never,
            TyTag::Param => Self::Param,
            TyTag::Unit => Self::Unit,
            TyTag::FnDef | TyTag::Assoc | TyTag::ImplTrait | TyTag::Unknown => Self::Other,
        }
    }
}

/// Structured inherent or trait `impl` header (32 bytes, 4-byte aligned).
///
/// Sorted on disk by `(simplified_self, is_trait_u8, self_ty, trait_def, def_index)`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ImplEntry {
    /// Local `DefEntry` index of this `impl` block, or `u32::MAX` if metadata-only (e.g. sysroot).
    pub def_index: u32,
    /// Exact `Self` type (`TyId` in `TyArena`, never `peel_refs`-collapsed).
    pub self_ty: u32,
    /// Implemented trait (`TyDefId` in `TyArena`), or `u32::MAX` (`TyDefId::NONE`) for inherent impls.
    pub trait_def: u32,
    /// Start index in `ty_args` for generic arguments applied to `trait_def` (excluding `Self`).
    pub trait_args_start: u32,
    /// Start index in `assoc_tys` for `AssocTyEntry` bindings belonging to this `impl`.
    pub assoc_tys_start: u32,
    /// Start index in `ty_args` for local `DefEntry` indices (`AssocFn` / `AssocConst`) of this `impl`.
    pub assoc_defs_start: u32,
    /// Number of encoded `GenericArgWord`s starting at `trait_args_start`.
    pub trait_args_len: u16,
    /// Number of `AssocTyEntry` rows starting at `assoc_tys_start`.
    pub assoc_tys_len: u16,
    /// Number of `u32` `DefEntry` indices starting at `assoc_defs_start`.
    pub assoc_defs_len: u16,
    /// Bitflags (`IMPL_FLAG_*`).
    pub flags: u8,
    /// `SimplifiedSelfKind` discriminator (`u8`).
    pub simplified_self: u8,
}

impl ImplEntry {
    #[inline]
    pub const fn to_le(self) -> Self {
        Self {
            def_index: self.def_index.to_le(),
            self_ty: self.self_ty.to_le(),
            trait_def: self.trait_def.to_le(),
            trait_args_start: self.trait_args_start.to_le(),
            assoc_tys_start: self.assoc_tys_start.to_le(),
            assoc_defs_start: self.assoc_defs_start.to_le(),
            trait_args_len: self.trait_args_len.to_le(),
            assoc_tys_len: self.assoc_tys_len.to_le(),
            assoc_defs_len: self.assoc_defs_len.to_le(),
            flags: self.flags,
            simplified_self: self.simplified_self,
        }
    }

    #[inline]
    pub const fn from_le(self) -> Self {
        Self {
            def_index: u32::from_le(self.def_index),
            self_ty: u32::from_le(self.self_ty),
            trait_def: u32::from_le(self.trait_def),
            trait_args_start: u32::from_le(self.trait_args_start),
            assoc_tys_start: u32::from_le(self.assoc_tys_start),
            assoc_defs_start: u32::from_le(self.assoc_defs_start),
            trait_args_len: u16::from_le(self.trait_args_len),
            assoc_tys_len: u16::from_le(self.assoc_tys_len),
            assoc_defs_len: u16::from_le(self.assoc_defs_len),
            flags: self.flags,
            simplified_self: self.simplified_self,
        }
    }

    #[inline]
    pub const fn simplified_self_kind(&self) -> SimplifiedSelfKind {
        SimplifiedSelfKind::from_u8(self.simplified_self)
    }

    #[inline]
    pub const fn is_trait(&self) -> bool {
        (self.flags & IMPL_FLAG_TRAIT) != 0
    }
}

/// Associated type binding inside an `ImplEntry` (`type Item = T;`, 16 bytes, 4-byte aligned).
///
/// Stored contiguously per `ImplEntry` (`assoc_tys_start..assoc_tys_start + assoc_tys_len`).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AssocTyEntry {
    /// Index of the owning `ImplEntry` in `impls`.
    pub impl_index: u32,
    /// Local `DefEntry` index in `definitions`, or `u32::MAX` if metadata-only.
    pub def_index: u32,
    /// Offset of the associated type identifier in `string_pool`.
    pub name_offset: u32,
    /// Lowered RHS `TyId` in `TyArena`.
    pub ty: u32,
}

impl AssocTyEntry {
    #[inline]
    pub const fn to_le(self) -> Self {
        Self { impl_index: self.impl_index.to_le(), def_index: self.def_index.to_le(), name_offset: self.name_offset.to_le(), ty: self.ty.to_le() }
    }

    #[inline]
    pub const fn from_le(self) -> Self {
        Self { impl_index: u32::from_le(self.impl_index), def_index: u32::from_le(self.def_index), name_offset: u32::from_le(self.name_offset), ty: u32::from_le(self.ty) }
    }
}
