/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Zero-dependency trait boundary ([`TyDefDb`]), `#[repr(u16)]`-aligned [`TyLangItem`] catalog
//! (matching `ridx-format::LangItemKind`), constructor shapes, and arena type cloner (`clone_ty_into`).

use super::arena::InferTyArena;
use super::ty::{InferTyId, InferTyKind, TySliceRange};
use super::uf_table::{InlineTyBuf, UnionFindTable};
use crate::name_res::{GlobalDefId, ImplTargetKey};

/// Compiler `#[lang = "..."]` items and associated trait items used during local type inference.
/// Discriminants match `ridx_format::LangItemKind` 1-to-1 for zero-cost bridging.
#[repr(u16)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TyLangItem {
    Sized = 0,
    Copy = 1,
    Clone = 2,
    Sync = 3,
    Freeze = 4,
    Unpin = 5,
    Drop = 6,
    Tuple = 7,
    FnPtrTrait = 8,
    Option = 9,
    OptionSome = 10,
    OptionNone = 11,
    Result = 12,
    ResultOk = 13,
    ResultErr = 14,
    ControlFlow = 15,
    ControlFlowContinue = 16,
    ControlFlowBreak = 17,
    Poll = 18,
    PollReady = 19,
    PollPending = 20,
    Pin = 21,
    OwnedBox = 22,
    String = 23,
    RangeFull = 28,
    Range = 29,
    RangeFrom = 30,
    RangeTo = 31,
    RangeInclusive = 32,
    RangeToInclusive = 33,
    Try = 34,
    TryOutput = 35,
    Fn = 43,
    FnMut = 45,
    FnOnce = 47,
    FnOnceOutput = 49,
    AsyncFn = 50,
    AsyncFnMut = 52,
    AsyncFnOnce = 54,
    AsyncFnOnceOutput = 56,
    Deref = 59,
    DerefTarget = 60,
    DerefMut = 62,
    Index = 66,
    IndexOutput = 67,
    IndexMut = 69,
    Iterator = 71,
    IteratorItem = 72,
    IntoIterator = 74,
    IntoIteratorItem = 75,
    Future = 78,
    FutureOutput = 79,
    IntoFuture = 81,
    IntoFutureOutput = 82,
    Add = 87,
    Sub = 88,
    Mul = 89,
    Div = 90,
    Rem = 91,
    Neg = 92,
    Not = 93,
    BitXor = 94,
    BitAnd = 95,
    BitOr = 96,
    Shl = 97,
    Shr = 98,
    Other = u16::MAX,
}

impl TyLangItem {
    #[inline]
    pub const fn is_fn_trait(self) -> bool {
        matches!(self, Self::Fn | Self::FnMut | Self::FnOnce | Self::AsyncFn | Self::AsyncFnMut | Self::AsyncFnOnce)
    }

    #[inline]
    pub const fn is_async_fn_trait(self) -> bool {
        matches!(self, Self::AsyncFn | Self::AsyncFnMut | Self::AsyncFnOnce)
    }

    #[inline]
    pub const fn is_range_adt(self) -> bool {
        matches!(self, Self::Range | Self::RangeFrom | Self::RangeTo | Self::RangeInclusive | Self::RangeToInclusive | Self::RangeFull)
    }
}

/// Value/record constructor shape of a `struct`, `union`, or `enum` variant.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CtorShapeKind {
    #[default]
    None = 0,
    Unit = 1,
    Tuple = 2,
    Record = 3,
}

/// Candidate method produced during `3.3` inherent / trait / bound method resolution (`Copy`, `0` heap allocs).
#[derive(Clone, Copy, Debug)]
pub struct MethodCand {
    /// Resolved `AssocFn` `GlobalDefId` (`impl`'s `AssocFn` when defined/overridden, else `trait`'s `AssocFn`).
    pub method_def: GlobalDefId,
    /// Enclosing `Trait` `GlobalDefId` (`GlobalDefId::NONE` for inherent methods).
    pub trait_def: GlobalDefId,
    /// Callable `GlobalDefId` holding generic `Fn*` parameter bounds (`trait` `AssocFn` when default or fallback).
    pub bound_owner_def: GlobalDefId,
    /// Number of non-lifetime generic parameters on the `impl` (`0` for direct `Param`/`Dyn`/`Opaque` bounds).
    pub container_generics: u16,
    /// Number of non-lifetime generic parameters declared on the method itself.
    pub method_generics: u16,
    /// `true` when `method_sig` uses `Trait` parameter indexing (`Param(0) = Self`, `1..1+N = trait_args`).
    pub has_fixed_trait_subst: bool,
    pub fixed_self_ty: InferTyId,
    pub fixed_trait_args: TySliceRange,
    pub is_blanket_impl: bool,
    /// CST `NodeId` when backed by `LocalCstDefs` (`u32::MAX` when backed by `TyDefDb`).
    pub cst_fn_node: u32,
}

/// External/indexed type & signature provider decoupling `ridx-syntax::ty_infer` from `ridx-format`.
/// All methods materialize types directly into the caller's reusable [`InferTyArena`] (`0` heap allocs).
pub trait TyDefDb {
    /// Function or associated function signature: appends parameter types to `inputs_out` and returns
    /// `(generic_type_or_const_param_count, output_ty)`.
    fn fn_sig(&self, def: GlobalDefId, arena: &mut InferTyArena, inputs_out: &mut Vec<InferTyId>) -> Option<(u16, InferTyId)>;

    /// Definition type for an ADT (`Struct`/`Enum`/`Union`), `Variant` (returns owning `Enum` ADT type),
    /// `TypeAlias` (returns aliased target type), `Const`, or `Static`, along with `(generic_count, ctor_shape)`.
    fn def_ty(&self, def: GlobalDefId, arena: &mut InferTyArena) -> Option<(u16, CtorShapeKind, InferTyId)>;

    /// Tuple constructor field types for a tuple `Struct` or tuple `Variant`; returns `generic_count`.
    fn ctor_tuple_fields(&self, def: GlobalDefId, arena: &mut InferTyArena, fields_out: &mut Vec<InferTyId>) -> Option<u16>;

    /// Field type by name (or `"0"`, `"1"` for tuple fields) on a `Struct`, `Union`, or `Variant`,
    /// returning `(generic_count, field_ty)` parameterized by `Param { idx, .. }`.
    fn field_ty(&self, owner_def: GlobalDefId, name: &str, arena: &mut InferTyArena) -> Option<(u16, InferTyId)>;

    /// Field type + field `GlobalDefId` by name (or `"0"`, `"1"` for tuple fields).
    fn field_ty_and_def(&self, owner_def: GlobalDefId, name: &str, arena: &mut InferTyArena) -> Option<(u16, InferTyId, GlobalDefId)> {
        self.field_ty(owner_def, name, arena).map(|(gc, ty)| (gc, ty, GlobalDefId::NONE))
    }

    /// Resolves a known compiler [`TyLangItem`] to its [`GlobalDefId`].
    fn lang_item(&self, item: TyLangItem) -> Option<GlobalDefId>;

    /// Reverse lookup: returns the [`TyLangItem`] tag of `def`, if `def` is a lang item.
    fn lang_item_of_def(&self, def: GlobalDefId) -> Option<TyLangItem>;

    /// Projects an associated type (`TryOutput`, `FutureOutput`, `IntoFutureOutput`, `DerefTarget`,
    /// `IndexOutput`, `IntoIteratorItem`, `IteratorItem`, or operator `Output`) for `self_ty` and `trait_args`.
    fn project_assoc_ty(&self, assoc_item: TyLangItem, self_ty: InferTyId, trait_args: &[InferTyId], arena: &mut InferTyArena, vars: &mut UnionFindTable) -> Option<InferTyId>;

    /// Projects an arbitrary trait associated type `<self_ty as trait_def<trait_args>>::assoc_def`.
    fn project_trait_assoc(
        &self,
        _trait_def: GlobalDefId,
        _assoc_def: GlobalDefId,
        _self_ty: InferTyId,
        _trait_args: &[InferTyId],
        _arena: &mut InferTyArena,
        _vars: &mut UnionFindTable,
    ) -> Option<InferTyId> {
        None
    }

    /// Queries an `Fn*` / `AsyncFn*` trait bound on generic parameter `param_idx` of `owner_def`,
    /// appending expected closure parameter types (containing `Param { idx, .. }`) to `inputs_out`
    /// and returning `(fn_trait_item, ret_ty)`.
    fn param_fn_bound(&self, owner_def: GlobalDefId, param_idx: u16, arena: &mut InferTyArena, inputs_out: &mut Vec<InferTyId>) -> Option<(TyLangItem, InferTyId)>;

    /// Appends inherent `impl` method candidates (`has_self == true`) matching `(target, method_name)`.
    fn inherent_method_cands(&self, _target: ImplTargetKey, _method_name: &str, _arena: &mut InferTyArena, _out: &mut Vec<MethodCand>) {}

    /// Appends concrete and blanket `impl Trait for ...` method candidates (`has_self == true`) for `trait_def`
    /// (including supertraits of `trait_def`) matching `(target, method_name)` and compatible with `step_ty`.
    #[allow(clippy::too_many_arguments)]
    fn trait_impl_method_cands(
        &self,
        _target: ImplTargetKey,
        _trait_def: GlobalDefId,
        _method_name: &str,
        _step_ty: InferTyId,
        _arena: &mut InferTyArena,
        _vars: &mut UnionFindTable,
        _out: &mut Vec<MethodCand>,
    ) {
    }

    /// Looks up `method_name` (`has_self == true`) directly on `trait_def` (or its supertraits) for `Param`/`Dyn`/`Opaque` receivers.
    fn trait_decl_method_cand(&self, _trait_def: GlobalDefId, _trait_args: &[InferTyId], _self_ty: InferTyId, _method_name: &str, _arena: &mut InferTyArena) -> Option<MethodCand> {
        None
    }

    /// Materializes `cand.method_def`'s formal parameter types (`inputs_out[0]` = `self_param_ty`) and return type.
    fn method_sig(&self, cand: &MethodCand, arena: &mut InferTyArena, inputs_out: &mut Vec<InferTyId>) -> Option<InferTyId> {
        self.fn_sig(cand.method_def, arena, inputs_out).map(|(_, out)| out)
    }

    /// Appends direct supertraits (and optional instantiated trait args) of `trait_def`.
    fn trait_supertraits(&self, _trait_def: GlobalDefId, _out: &mut Vec<GlobalDefId>) {}

    /// Appends trait bounds `Param(param_idx): BoundTrait` declared on `owner_def` (`Fn` or `AssocFn` or `Impl`).
    fn param_trait_bounds(&self, _owner_def: GlobalDefId, _param_idx: u16, _out: &mut Vec<GlobalDefId>) {}

    /// Verifies blanket-impl `where` bounds after binding `subst` against a method receiver.
    fn check_blanket_method_bounds(&self, _method_def: GlobalDefId, _subst: &[InferTyId], _arena: &mut InferTyArena, _vars: &mut UnionFindTable) -> bool {
        true
    }

    /// Optional `core::ffi::CStr` definition for `c"..."` literals (`&CStr`).
    fn cstr_def(&self) -> Option<GlobalDefId> {
        None
    }
}

/// Zero-state [`TyDefDb`] when running pure local CST inference without an external `.ridx` index.
#[derive(Clone, Copy, Debug, Default)]
pub struct EmptyTyDefDb;

impl TyDefDb for EmptyTyDefDb {
    fn fn_sig(&self, _: GlobalDefId, _: &mut InferTyArena, _: &mut Vec<InferTyId>) -> Option<(u16, InferTyId)> {
        None
    }
    fn def_ty(&self, _: GlobalDefId, _: &mut InferTyArena) -> Option<(u16, CtorShapeKind, InferTyId)> {
        None
    }
    fn ctor_tuple_fields(&self, _: GlobalDefId, _: &mut InferTyArena, _: &mut Vec<InferTyId>) -> Option<u16> {
        None
    }
    fn field_ty(&self, _: GlobalDefId, _: &str, _: &mut InferTyArena) -> Option<(u16, InferTyId)> {
        None
    }
    fn lang_item(&self, _: TyLangItem) -> Option<GlobalDefId> {
        None
    }
    fn lang_item_of_def(&self, _: GlobalDefId) -> Option<TyLangItem> {
        None
    }
    fn project_assoc_ty(&self, _: TyLangItem, _: InferTyId, _: &[InferTyId], _: &mut InferTyArena, _: &mut UnionFindTable) -> Option<InferTyId> {
        None
    }
    fn param_fn_bound(&self, _: GlobalDefId, _: u16, _: &mut InferTyArena, _: &mut Vec<InferTyId>) -> Option<(TyLangItem, InferTyId)> {
        None
    }
}

/// Copies a template [`InferTyId`] from `src` into `dst` with zero heap allocations.
pub fn clone_ty_into(src: &InferTyArena, id: InferTyId, dst: &mut InferTyArena) -> InferTyId {
    clone_ty_rec(src, id, dst, 0)
}

fn clone_ty_rec(src: &InferTyArena, id: InferTyId, dst: &mut InferTyArena, depth: u8) -> InferTyId {
    if !id.is_some() || (id.0 as usize) < InferTyId::BUILTIN_COUNT || depth > 64 {
        return id;
    }
    match src.kind(id) {
        InferTyKind::Unknown => InferTyId::UNKNOWN,
        InferTyKind::Never => InferTyId::NEVER,
        InferTyKind::Prim(p) => InferTyId::from_prim(p),
        InferTyKind::Ref { is_mut, inner } => {
            let sub = clone_ty_rec(src, inner, dst, depth + 1);
            dst.mk_ref(is_mut, sub)
        }
        InferTyKind::Ptr { is_mut, inner } => {
            let sub = clone_ty_rec(src, inner, dst, depth + 1);
            dst.mk_ptr(is_mut, sub)
        }
        InferTyKind::Slice(elem) => {
            let sub = clone_ty_rec(src, elem, dst, depth + 1);
            dst.mk_slice(sub)
        }
        InferTyKind::Array { elem, len } => {
            let sub = clone_ty_rec(src, elem, dst, depth + 1);
            dst.mk_array(sub, len)
        }
        InferTyKind::Param { idx, def } => dst.mk_param(idx, def),
        InferTyKind::Tuple(r) => {
            let buf = clone_slice(src, r, dst, depth + 1);
            dst.mk_tuple(buf.as_slice())
        }
        InferTyKind::Adt { def, args } => {
            let buf = clone_slice(src, args, dst, depth + 1);
            dst.mk_adt(def, buf.as_slice())
        }
        InferTyKind::FnPtr { sig } => {
            let buf = clone_slice(src, sig, dst, depth + 1);
            let s = buf.as_slice();
            dst.mk_fn_ptr(&s[..s.len() - 1], s[s.len() - 1])
        }
        InferTyKind::FnDef { def, args } => {
            let buf = clone_slice(src, args, dst, depth + 1);
            dst.mk_fn_def(def, buf.as_slice())
        }
        InferTyKind::Closure { id: cid, sig } => {
            let buf = clone_slice(src, sig, dst, depth + 1);
            let s = buf.as_slice();
            dst.mk_closure(cid, &s[..s.len() - 1], s[s.len() - 1])
        }
        InferTyKind::Alias { trait_def, assoc_def, self_ty, args } => {
            let ns = clone_ty_rec(src, self_ty, dst, depth + 1);
            let buf = clone_slice(src, args, dst, depth + 1);
            dst.mk_alias(trait_def, assoc_def, ns, buf.as_slice())
        }
        InferTyKind::Dyn { principal, args } => {
            let buf = clone_slice(src, args, dst, depth + 1);
            dst.mk_dyn(principal, buf.as_slice())
        }
        InferTyKind::Opaque { def, args } => {
            let buf = clone_slice(src, args, dst, depth + 1);
            dst.mk_opaque(def, buf.as_slice())
        }
        InferTyKind::Infer(_) => InferTyId::UNKNOWN,
    }
}

fn clone_slice(src: &InferTyArena, r: super::ty::TySliceRange, dst: &mut InferTyArena, depth: u8) -> InlineTyBuf {
    let mut buf = InlineTyBuf::new();
    for &elem in src.slice(r) {
        buf.push(clone_ty_rec(src, elem, dst, depth));
    }
    buf
}
