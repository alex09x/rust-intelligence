/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Autoderef chain construction (`Deref::Target`, `&T`, `Box<T>`, `String`, `Pin<P>` — excluding
//! raw pointers `*const T` / `*mut T`), `ImplTargetKey` classification, and associated-type
//! projection normalization (`Alias` -> concrete `InferTyId`).

use super::arena::InferTyArena;
use super::db::{TyDefDb, TyLangItem};
use super::local_defs::LocalCstDefs;
use super::lower_ty::LowerTyEnv;
use super::ty::{InferTyId, InferTyKind};
use super::uf_table::InlineTyBuf;
use super::InferenceTable;
use crate::name_res::{FileNameRes, GlobalDefId, ImplTargetKey};
use crate::tree::{NodeId, SyntaxTree};

/// Maximum autoderef steps per `.method()` / `.field` lookup (with cycle guard).
pub const MAX_AUTODEREF_STEPS: usize = 12;

/// Optional enclosing item context enabling `<Param as Trait>::Assoc` normalization (`Checkbox 3.4`).
#[derive(Clone, Copy, Debug)]
pub struct NormOwner {
    pub fn_def: GlobalDefId,
    pub impl_node: Option<NodeId>,
    pub impl_gc: u16,
    pub fn_node: Option<NodeId>,
    pub self_ty: Option<InferTyId>,
}

impl NormOwner {
    pub const NONE: Self = Self { fn_def: GlobalDefId::NONE, impl_node: None, impl_gc: 0, fn_node: None, self_ty: None };
}

/// Maps a shallow-resolved [`InferTyId`] to its indexed [`ImplTargetKey`] head.
#[inline]
pub fn impl_target_key_of_ty(arena: &InferTyArena, ty: InferTyId) -> ImplTargetKey {
    match arena.kind(ty) {
        InferTyKind::Adt { def, .. } if def.is_some() => ImplTargetKey::Adt(def),
        InferTyKind::Prim(p) => ImplTargetKey::Prim(p),
        InferTyKind::Ref { is_mut, .. } => ImplTargetKey::Ref { is_mut },
        InferTyKind::Ptr { is_mut, .. } => ImplTargetKey::Ptr { is_mut },
        InferTyKind::Slice(_) => ImplTargetKey::Slice,
        InferTyKind::Array { .. } => ImplTargetKey::Array,
        InferTyKind::Tuple(r) => ImplTargetKey::Tuple(r.len),
        InferTyKind::FnPtr { sig } => ImplTargetKey::FnPtr { arity: sig.len.saturating_sub(1) },
        InferTyKind::Dyn { principal, .. } if principal.is_some() => ImplTargetKey::Dyn(principal),
        InferTyKind::Never => ImplTargetKey::Never,
        InferTyKind::Param { .. } => ImplTargetKey::Param,
        _ => ImplTargetKey::Unknown,
    }
}

/// Performs a single Rust method/field autoderef step (`&T`/`&mut T`, `Box<T>`, `String`, `Pin<P>`,
/// or `Deref::Target`), deliberately NOT dereferencing raw pointers (`*const T` / `*mut T`).
pub fn autoderef_step(table: &mut InferenceTable, db: &dyn TyDefDb, local_defs: &LocalCstDefs, st: &SyntaxTree<'_>, file_res: &FileNameRes, ty: InferTyId) -> Option<InferTyId> {
    autoderef_step_rec(table, db, local_defs, st, file_res, NormOwner::NONE, ty, 0)
}

#[allow(clippy::too_many_arguments)]
fn autoderef_step_rec(
    table: &mut InferenceTable,
    db: &dyn TyDefDb,
    local_defs: &LocalCstDefs,
    st: &SyntaxTree<'_>,
    file_res: &FileNameRes,
    owner: NormOwner,
    ty: InferTyId,
    depth: u8,
) -> Option<InferTyId> {
    if depth > 8 {
        return None;
    }
    let r = normalize_ty_with_owner(table, db, local_defs, st, file_res, owner, ty);
    match table.arena.kind(r) {
        InferTyKind::Ref { inner, .. } => Some(normalize_ty_with_owner(table, db, local_defs, st, file_res, owner, inner)),
        InferTyKind::Ptr { .. } => None,
        InferTyKind::Param { idx, .. } => {
            let deref_tr = db.lang_item(TyLangItem::Deref).unwrap_or(GlobalDefId::NONE);
            let deref_tgt = db.lang_item(TyLangItem::DerefTarget).unwrap_or(GlobalDefId::NONE);
            if owner.fn_def.is_some() && deref_tr.is_some() {
                if let Some(p) = db.project_param_assoc(owner.fn_def, idx, deref_tr, deref_tgt, &[], &mut table.arena) {
                    return Some(normalize_ty_with_owner(table, db, local_defs, st, file_res, owner, p));
                }
            }
            if owner.fn_node.is_some() || owner.impl_node.is_some() {
                let env = LowerTyEnv {
                    st,
                    file_res,
                    db,
                    local_defs,
                    self_ty: owner.self_ty,
                    impl_owner: owner.impl_node,
                    impl_generics: owner.impl_gc,
                    item_owner: owner.fn_node,
                    in_body: false,
                };
                if let Some(p) = super::trait_solve::project_cst_param_assoc(&env, table, idx, deref_tr, deref_tgt, &[], depth + 1) {
                    return Some(normalize_ty_with_owner(table, db, local_defs, st, file_res, owner, p));
                }
            }
            None
        }
        InferTyKind::Adt { def, args } => {
            match db.lang_item_of_def(def) {
                Some(TyLangItem::OwnedBox) if args.len >= 1 => {
                    let inner = table.arena.slice(args)[0];
                    return Some(normalize_ty_with_owner(table, db, local_defs, st, file_res, owner, inner));
                }
                Some(TyLangItem::String) => return Some(InferTyId::STR),
                Some(TyLangItem::Pin) if args.len >= 1 => {
                    let ptr_ty = table.arena.slice(args)[0];
                    return autoderef_step_rec(table, db, local_defs, st, file_res, owner, ptr_ty, depth + 1);
                }
                _ => {}
            }
            if let Some(proj) = db.project_assoc_ty(TyLangItem::DerefTarget, r, &[], &mut table.arena, &mut table.vars) {
                return Some(normalize_ty_with_owner(table, db, local_defs, st, file_res, owner, proj));
            }
            local_defs.project_local_deref(r, st, file_res, db, table).map(|proj| normalize_ty_with_owner(table, db, local_defs, st, file_res, owner, proj))
        }
        _ => None,
    }
}

/// Populates `out[..len]` with the cycle-checked autoderef chain starting at `start_ty`.
pub fn build_autoderef_steps(
    table: &mut InferenceTable,
    db: &dyn TyDefDb,
    local_defs: &LocalCstDefs,
    st: &SyntaxTree<'_>,
    file_res: &FileNameRes,
    start_ty: InferTyId,
    out: &mut [InferTyId; MAX_AUTODEREF_STEPS],
) -> usize {
    build_autoderef_steps_with_owner(table, db, local_defs, st, file_res, NormOwner::NONE, start_ty, out)
}

#[allow(clippy::too_many_arguments)]
pub fn build_autoderef_steps_with_owner(
    table: &mut InferenceTable,
    db: &dyn TyDefDb,
    local_defs: &LocalCstDefs,
    st: &SyntaxTree<'_>,
    file_res: &FileNameRes,
    owner: NormOwner,
    start_ty: InferTyId,
    out: &mut [InferTyId; MAX_AUTODEREF_STEPS],
) -> usize {
    let first = normalize_ty_with_owner(table, db, local_defs, st, file_res, owner, start_ty);
    out[0] = first;
    let mut len = 1usize;
    if matches!(table.arena.kind(first), InferTyKind::Unknown | InferTyKind::Infer(_)) {
        return len;
    }
    while len < MAX_AUTODEREF_STEPS {
        let prev = out[len - 1];
        let Some(next) = autoderef_step_rec(table, db, local_defs, st, file_res, owner, prev, 0) else {
            break;
        };
        if matches!(table.arena.kind(next), InferTyKind::Unknown | InferTyKind::Infer(_)) || out[..len].contains(&next) {
            break;
        }
        out[len] = next;
        len += 1;
    }
    len
}

/// Normalizes `<SelfTy as Trait>::Assoc` (`InferTyKind::Alias`) nodes inside `ty` when `SelfTy` is known.
pub fn normalize_ty(table: &mut InferenceTable, db: &dyn TyDefDb, local_defs: &LocalCstDefs, st: &SyntaxTree<'_>, file_res: &FileNameRes, ty: InferTyId) -> InferTyId {
    normalize_ty_rec(table, db, local_defs, st, file_res, NormOwner::NONE, ty, 0)
}

/// Normalizes `<SelfTy as Trait>::Assoc` including `<Param as Trait>::Assoc` active bounds (`Checkbox 3.4`).
pub fn normalize_ty_with_owner(
    table: &mut InferenceTable,
    db: &dyn TyDefDb,
    local_defs: &LocalCstDefs,
    st: &SyntaxTree<'_>,
    file_res: &FileNameRes,
    owner: NormOwner,
    ty: InferTyId,
) -> InferTyId {
    normalize_ty_rec(table, db, local_defs, st, file_res, owner, ty, 0)
}

#[allow(clippy::too_many_arguments)]
fn normalize_ty_rec(
    table: &mut InferenceTable,
    db: &dyn TyDefDb,
    local_defs: &LocalCstDefs,
    st: &SyntaxTree<'_>,
    file_res: &FileNameRes,
    owner: NormOwner,
    ty: InferTyId,
    depth: u8,
) -> InferTyId {
    let r = table.shallow_resolve(ty);
    if depth > 12 {
        return r;
    }
    let flags = table.arena.flags(r);
    if !flags.has_alias() && !flags.has_infer() {
        return r;
    }
    match table.arena.kind(r) {
        InferTyKind::Alias { trait_def, assoc_def, self_ty, args } => {
            let norm_self = normalize_ty_rec(table, db, local_defs, st, file_res, owner, self_ty, depth + 1);
            let mut norm_args = InlineTyBuf::new();
            for idx in 0..args.len as usize {
                let a = table.arena.slice(args)[idx];
                norm_args.push(normalize_ty_rec(table, db, local_defs, st, file_res, owner, a, depth + 1));
            }
            if let InferTyKind::Param { idx, .. } = table.arena.kind(norm_self) {
                if owner.fn_def.is_some() {
                    if let Some(p) = db.project_param_assoc(owner.fn_def, idx, trait_def, assoc_def, norm_args.as_slice(), &mut table.arena) {
                        return normalize_ty_rec(table, db, local_defs, st, file_res, owner, p, depth + 1);
                    }
                }
                if owner.fn_node.is_some() || owner.impl_node.is_some() {
                    let env = LowerTyEnv {
                        st,
                        file_res,
                        db,
                        local_defs,
                        self_ty: owner.self_ty,
                        impl_owner: owner.impl_node,
                        impl_generics: owner.impl_gc,
                        item_owner: owner.fn_node,
                        in_body: false,
                    };
                    if let Some(p) = super::trait_solve::project_cst_param_assoc(&env, table, idx, trait_def, assoc_def, norm_args.as_slice(), depth + 1) {
                        return normalize_ty_rec(table, db, local_defs, st, file_res, owner, p, depth + 1);
                    }
                }
            }
            if let Some(li) = db.lang_item_of_def(assoc_def) {
                if let Some(p) = db.project_assoc_ty(li, norm_self, norm_args.as_slice(), &mut table.arena, &mut table.vars) {
                    return normalize_ty_rec(table, db, local_defs, st, file_res, owner, p, depth + 1);
                }
            }
            if let Some(p) = db.project_trait_assoc(trait_def, assoc_def, norm_self, norm_args.as_slice(), &mut table.arena, &mut table.vars) {
                return normalize_ty_rec(table, db, local_defs, st, file_res, owner, p, depth + 1);
            }
            if let Some(p) = local_defs.project_trait_assoc(trait_def, assoc_def, norm_self, norm_args.as_slice(), st, file_res, db, table) {
                return normalize_ty_rec(table, db, local_defs, st, file_res, owner, p, depth + 1);
            }
            table.arena.mk_alias(trait_def, assoc_def, norm_self, norm_args.as_slice())
        }
        InferTyKind::Ref { is_mut, inner } if flags.has_alias() => {
            let sub = normalize_ty_rec(table, db, local_defs, st, file_res, owner, inner, depth + 1);
            table.arena.mk_ref(is_mut, sub)
        }
        InferTyKind::Adt { def, args } if flags.has_alias() => {
            let mut b = InlineTyBuf::new();
            for idx in 0..args.len as usize {
                let a = table.arena.slice(args)[idx];
                b.push(normalize_ty_rec(table, db, local_defs, st, file_res, owner, a, depth + 1));
            }
            table.arena.mk_adt(def, b.as_slice())
        }
        InferTyKind::Tuple(rng) if flags.has_alias() => {
            let mut b = InlineTyBuf::new();
            for idx in 0..rng.len as usize {
                let a = table.arena.slice(rng)[idx];
                b.push(normalize_ty_rec(table, db, local_defs, st, file_res, owner, a, depth + 1));
            }
            table.arena.mk_tuple(b.as_slice())
        }
        InferTyKind::Slice(elem) if flags.has_alias() => {
            let sub = normalize_ty_rec(table, db, local_defs, st, file_res, owner, elem, depth + 1);
            table.arena.mk_slice(sub)
        }
        InferTyKind::Opaque { def, args } if flags.has_alias() => {
            let mut b = InlineTyBuf::new();
            for idx in 0..args.len as usize {
                let a = table.arena.slice(args)[idx];
                b.push(normalize_ty_rec(table, db, local_defs, st, file_res, owner, a, depth + 1));
            }
            table.arena.mk_opaque(def, b.as_slice())
        }
        _ => r,
    }
}
