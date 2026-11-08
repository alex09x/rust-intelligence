/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Core inference-time type identifiers, variable domains, summary flags, and [`InferTyKind`].

use crate::name_res::{GlobalDefId, PrimTy};

/// Dense `u32` index into [`super::arena::InferTyArena`]. Builtin types (`0..21`) are constants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InferTyId(pub u32);

impl InferTyId {
    pub const NONE: Self = Self(u32::MAX);
    pub const UNKNOWN: Self = Self(0);
    pub const NEVER: Self = Self(1);
    pub const UNIT: Self = Self(2);
    pub const I8: Self = Self(3);
    pub const I16: Self = Self(4);
    pub const I32: Self = Self(5);
    pub const I64: Self = Self(6);
    pub const I128: Self = Self(7);
    pub const ISIZE: Self = Self(8);
    pub const U8: Self = Self(9);
    pub const U16: Self = Self(10);
    pub const U32: Self = Self(11);
    pub const U64: Self = Self(12);
    pub const U128: Self = Self(13);
    pub const USIZE: Self = Self(14);
    pub const F32: Self = Self(15);
    pub const F64: Self = Self(16);
    pub const BOOL: Self = Self(17);
    pub const CHAR: Self = Self(18);
    pub const STR: Self = Self(19);
    /// Shared `&str` (`Ref { is_mut: false, inner: Self::STR }`), pre-interned at slot `20`.
    pub const STR_REF: Self = Self(20);

    pub const BUILTIN_COUNT: usize = 21;

    #[inline]
    pub const fn is_some(self) -> bool {
        self.0 != u32::MAX
    }

    #[inline]
    pub const fn from_prim(p: PrimTy) -> Self {
        Self((p as u32) + 2)
    }
}

impl Default for InferTyId {
    #[inline]
    fn default() -> Self {
        Self::UNKNOWN
    }
}

/// Dense `u32` index into [`super::uf_table::UnionFindTable`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TyVarId(pub u32);

impl TyVarId {
    pub const NONE: Self = Self(u32::MAX);

    #[inline]
    pub const fn is_some(self) -> bool {
        self.0 != u32::MAX
    }
}

/// Constraint domain of an unresolved inference variable.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TyVarKind {
    /// Unconstrained type variable (`_`, generic arg, unannotated `let`); falls back to `{unknown}`.
    General = 0,
    /// Unsuffixed integer literal (`{integer}`); unifies with integer primitives, falls back to `i32`.
    IntVar = 1,
    /// Unsuffixed float literal (`{float}`); unifies with `f32`/`f64`, falls back to `f64`.
    FloatVar = 2,
}

/// Contiguous slice `[start .. start + len)` inside [`super::arena::InferTyArena`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TySliceRange {
    pub start: u32,
    pub len: u16,
}

impl TySliceRange {
    pub const EMPTY: Self = Self { start: 0, len: 0 };

    #[inline]
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }
}

/// Summary bitmask propagated bottom-up when interning types for `O(1)` traversal pruning.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TyFlags(pub u8);

impl TyFlags {
    pub const EMPTY: Self = Self(0);
    pub const HAS_INFER: Self = Self(1 << 0);
    pub const HAS_PARAM: Self = Self(1 << 1);
    pub const HAS_UNKNOWN: Self = Self(1 << 2);
    pub const HAS_ALIAS: Self = Self(1 << 3);

    #[inline]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    #[inline]
    pub const fn has_infer(self) -> bool {
        (self.0 & Self::HAS_INFER.0) != 0
    }

    #[inline]
    pub const fn has_param(self) -> bool {
        (self.0 & Self::HAS_PARAM.0) != 0
    }

    #[inline]
    pub const fn has_unknown(self) -> bool {
        (self.0 & Self::HAS_UNKNOWN.0) != 0
    }

    #[inline]
    pub const fn has_alias(self) -> bool {
        (self.0 & Self::HAS_ALIAS.0) != 0
    }
}

/// Structural kind of an inference-time type stored in [`super::arena::InferTyArena`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InferTyKind {
    Unknown,
    Never,
    Prim(PrimTy),
    Tuple(TySliceRange),
    Adt {
        def: GlobalDefId,
        args: TySliceRange,
    },
    Ref {
        is_mut: bool,
        inner: InferTyId,
    },
    Ptr {
        is_mut: bool,
        inner: InferTyId,
    },
    Slice(InferTyId),
    Array {
        elem: InferTyId,
        len: u64,
    },
    /// Function pointer `fn(T0, .., Tn-1) -> Ret`; `sig` stores `[T0, .., Tn-1, Ret]` (`len >= 1`).
    FnPtr {
        sig: TySliceRange,
    },
    FnDef {
        def: GlobalDefId,
        args: TySliceRange,
    },
    /// Local closure with unique body-local `id` and signature `[inputs..., output]`.
    Closure {
        id: u32,
        sig: TySliceRange,
    },
    Param {
        idx: u16,
        def: GlobalDefId,
    },
    Alias {
        trait_def: GlobalDefId,
        assoc_def: GlobalDefId,
        self_ty: InferTyId,
        args: TySliceRange,
    },
    Dyn {
        principal: GlobalDefId,
        args: TySliceRange,
    },
    Opaque {
        def: GlobalDefId,
        args: TySliceRange,
    },
    Infer(TyVarId),
}

#[inline]
pub const fn is_int_prim(p: PrimTy) -> bool {
    matches!(
        p,
        PrimTy::I8 | PrimTy::I16 | PrimTy::I32 | PrimTy::I64 | PrimTy::I128 | PrimTy::Isize | PrimTy::U8 | PrimTy::U16 | PrimTy::U32 | PrimTy::U64 | PrimTy::U128 | PrimTy::Usize
    )
}

#[inline]
pub const fn is_float_prim(p: PrimTy) -> bool {
    matches!(p, PrimTy::F32 | PrimTy::F64)
}
