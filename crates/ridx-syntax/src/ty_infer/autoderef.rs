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
use super::ty::{InferTyId, InferTyKind};
use super::uf_table::InlineTyBuf;
use super::InferenceTable;
use crate::name_res::{FileNameRes, ImplTargetKey};
use crate::tree::SyntaxTree;

/// Maximum autoderef steps per `.method()` / `.field` lookup (with cycle guard).
pub const MAX_AUTODEREF_STEPS: usize = 12;

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
    autoderef_step_rec(table, db, local_defs, st, file_res, ty, 0)
}

fn autoderef_step_rec(
    table: &mut InferenceTable,
    db: &dyn TyDefDb,
    local_defs: &LocalCstDefs,
    st: &SyntaxTree<'_>,
    file_res: &FileNameRes,
    ty: InferTyId,
    depth: u8,
) -> Option<InferTyId> {
    if depth > 8 {
        return None;
    }
    let r = normalize_ty(table, db, local_defs, st, file_res, ty);
    match table.arena.kind(r) {
        InferTyKind::Ref { inner, .. } => Some(normalize_ty(table, db, local_defs, st, file_res, inner)),
        InferTyKind::Ptr { .. } => None,
        InferTyKind::Adt { def, args } => {
            match db.lang_item_of_def(def) {
                Some(TyLangItem::OwnedBox) if args.len >= 1 => {
                    let inner = table.arena.slice(args)[0];
                    return Some(normalize_ty(table, db, local_defs, st, file_res, inner));
                }
                Some(TyLangItem::String) => return Some(InferTyId::STR),
                Some(TyLangItem::Pin) if args.len >= 1 => {
                    let ptr_ty = table.arena.slice(args)[0];
                    return autoderef_step_rec(table, db, local_defs, st, file_res, ptr_ty, depth + 1);
                }
                _ => {}
            }
            if let Some(proj) = db.project_assoc_ty(TyLangItem::DerefTarget, r, &[], &mut table.arena, &mut table.vars) {
                return Some(normalize_ty(table, db, local_defs, st, file_res, proj));
            }
            local_defs.project_local_deref(r, st, file_res, db, table).map(|proj| normalize_ty(table, db, local_defs, st, file_res, proj))
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
    let first = normalize_ty(table, db, local_defs, st, file_res, start_ty);
    out[0] = first;
    let mut len = 1usize;
    if matches!(table.arena.kind(first), InferTyKind::Unknown | InferTyKind::Infer(_)) {
        return len;
    }
    while len < MAX_AUTODEREF_STEPS {
        let prev = out[len - 1];
        let Some(next) = autoderef_step(table, db, local_defs, st, file_res, prev) else {
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
    normalize_ty_rec(table, db, local_defs, st, file_res, ty, 0)
}

fn normalize_ty_rec(table: &mut InferenceTable, db: &dyn TyDefDb, local_defs: &LocalCstDefs, st: &SyntaxTree<'_>, file_res: &FileNameRes, ty: InferTyId, depth: u8) -> InferTyId {
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
            let norm_self = normalize_ty_rec(table, db, local_defs, st, file_res, self_ty, depth + 1);
            let mut norm_args = InlineTyBuf::new();
            for idx in 0..args.len as usize {
                let a = table.arena.slice(args)[idx];
                norm_args.push(normalize_ty_rec(table, db, local_defs, st, file_res, a, depth + 1));
            }
            if let Some(li) = db.lang_item_of_def(assoc_def) {
                if let Some(p) = db.project_assoc_ty(li, norm_self, norm_args.as_slice(), &mut table.arena, &mut table.vars) {
                    return normalize_ty_rec(table, db, local_defs, st, file_res, p, depth + 1);
                }
            }
            if let Some(p) = db.project_trait_assoc(trait_def, assoc_def, norm_self, norm_args.as_slice(), &mut table.arena, &mut table.vars) {
                return normalize_ty_rec(table, db, local_defs, st, file_res, p, depth + 1);
            }
            if let Some(p) = local_defs.project_trait_assoc(trait_def, assoc_def, norm_self, norm_args.as_slice(), st, file_res, db, table) {
                return normalize_ty_rec(table, db, local_defs, st, file_res, p, depth + 1);
            }
            table.arena.mk_alias(trait_def, assoc_def, norm_self, norm_args.as_slice())
        }
        InferTyKind::Ref { is_mut, inner } if flags.has_alias() => {
            let sub = normalize_ty_rec(table, db, local_defs, st, file_res, inner, depth + 1);
            table.arena.mk_ref(is_mut, sub)
        }
        InferTyKind::Adt { def, args } if flags.has_alias() => {
            let mut b = InlineTyBuf::new();
            for idx in 0..args.len as usize {
                let a = table.arena.slice(args)[idx];
                b.push(normalize_ty_rec(table, db, local_defs, st, file_res, a, depth + 1));
            }
            table.arena.mk_adt(def, b.as_slice())
        }
        InferTyKind::Tuple(rng) if flags.has_alias() => {
            let mut b = InlineTyBuf::new();
            for idx in 0..rng.len as usize {
                let a = table.arena.slice(rng)[idx];
                b.push(normalize_ty_rec(table, db, local_defs, st, file_res, a, depth + 1));
            }
            table.arena.mk_tuple(b.as_slice())
        }
        InferTyKind::Slice(elem) if flags.has_alias() => {
            let sub = normalize_ty_rec(table, db, local_defs, st, file_res, elem, depth + 1);
            table.arena.mk_slice(sub)
        }
        InferTyKind::Opaque { def, args } if flags.has_alias() => {
            let mut b = InlineTyBuf::new();
            for idx in 0..args.len as usize {
                let a = table.arena.slice(args)[idx];
                b.push(normalize_ty_rec(table, db, local_defs, st, file_res, a, depth + 1));
            }
            table.arena.mk_opaque(def, b.as_slice())
        }
        _ => r,
    }
}
