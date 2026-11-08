/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Compiler lang-item & builtin type operations: `?` (`Try`), `.await` (`IntoFuture` / `Future`),
//! `*` / autoderef (`Deref`), `base[idx]` (`Index` + slice/range indexing), `for` (`IntoIterator`),
//! `..` range constructors, and expected `Fn*` / `AsyncFn*` signature extraction for closures.

use super::db::{TyDefDb, TyLangItem};
use super::expectation::Expectation;
use super::local_defs::LocalCstDefs;
use super::ty::{InferTyId, InferTyKind};
use super::InferenceTable;
use crate::name_res::{FileNameRes, GlobalDefId, PrimTy};
use crate::tree::SyntaxTree;

/// Performs one dereference step (`&T`/`*const T` -> `T`, `Box<T>` -> `T`, `String` -> `str`, `Deref::Target`).
pub fn deref_step(table: &mut InferenceTable, db: &dyn TyDefDb, ty: InferTyId) -> Option<InferTyId> {
    deref_step_rec(table, db, ty, 0)
}

fn deref_step_rec(table: &mut InferenceTable, db: &dyn TyDefDb, raw_ty: InferTyId, depth: u8) -> Option<InferTyId> {
    if depth > 8 {
        return None;
    }
    let ty = table.shallow_resolve(raw_ty);
    match table.arena.kind(ty) {
        InferTyKind::Ref { inner, .. } | InferTyKind::Ptr { inner, .. } => Some(table.shallow_resolve(inner)),
        InferTyKind::Adt { def, args } => {
            match db.lang_item_of_def(def) {
                Some(TyLangItem::OwnedBox) if args.len >= 1 => return Some(table.shallow_resolve(table.arena.slice(args)[0])),
                Some(TyLangItem::String) => return Some(InferTyId::STR),
                Some(TyLangItem::Pin) if args.len >= 1 => {
                    let ptr_ty = table.arena.slice(args)[0];
                    if let Some(target) = deref_step_rec(table, db, ptr_ty, depth + 1) {
                        return Some(target);
                    }
                }
                _ => {}
            }
            db.project_assoc_ty(TyLangItem::DerefTarget, ty, &[], &mut table.arena, &mut table.vars)
        }
        _ => db.project_assoc_ty(TyLangItem::DerefTarget, ty, &[], &mut table.arena, &mut table.vars),
    }
}

/// Unwraps `expr?` via `Option<T> -> T`, `Result<T, E> -> T`, `ControlFlow<B, C> -> C`, `Poll<Result<T, E>>`, or `Try::Output`.
pub fn unwrap_try_ty(table: &mut InferenceTable, db: &dyn TyDefDb, operand: InferTyId) -> InferTyId {
    let ty = table.shallow_resolve(operand);
    if ty == InferTyId::NEVER {
        return InferTyId::NEVER;
    }
    if let InferTyKind::Adt { def, args } = table.arena.kind(ty) {
        match db.lang_item_of_def(def) {
            Some(TyLangItem::Option | TyLangItem::Result) if args.len >= 1 => {
                return table.shallow_resolve(table.arena.slice(args)[0]);
            }
            Some(TyLangItem::ControlFlow) if args.len >= 2 => {
                return table.shallow_resolve(table.arena.slice(args)[1]);
            }
            Some(TyLangItem::Poll) if args.len >= 1 => {
                let inner = table.shallow_resolve(table.arena.slice(args)[0]);
                if let InferTyKind::Adt { def: idef, args: iargs } = table.arena.kind(inner) {
                    match db.lang_item_of_def(idef) {
                        Some(TyLangItem::Result) if iargs.len >= 1 => {
                            let ok_ty = table.arena.slice(iargs)[0];
                            return table.arena.mk_adt(def, &[ok_ty]);
                        }
                        Some(TyLangItem::Option) if iargs.len >= 1 => {
                            let opt_in = table.shallow_resolve(table.arena.slice(iargs)[0]);
                            if let InferTyKind::Adt { def: rdef, args: rargs } = table.arena.kind(opt_in) {
                                if db.lang_item_of_def(rdef) == Some(TyLangItem::Result) && rargs.len >= 1 {
                                    let ok_ty = table.arena.slice(rargs)[0];
                                    let new_opt = table.arena.mk_adt(idef, &[ok_ty]);
                                    return table.arena.mk_adt(def, &[new_opt]);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    db.project_assoc_ty(TyLangItem::TryOutput, ty, &[], &mut table.arena, &mut table.vars).unwrap_or(InferTyId::UNKNOWN)
}

/// Unwraps `expr.await` via `Opaque`/`Dyn` `Future`/`IntoFuture`, `IntoFuture::Output`, `Future::Output`, or `Pin`/`Box`/`&mut` wrappers.
pub fn unwrap_await_ty(table: &mut InferenceTable, db: &dyn TyDefDb, operand: InferTyId) -> InferTyId {
    let mut cur = table.shallow_resolve(operand);
    if cur == InferTyId::NEVER {
        return InferTyId::NEVER;
    }
    for _ in 0..6 {
        if let InferTyKind::Opaque { def, args } | InferTyKind::Dyn { principal: def, args } = table.arena.kind(cur) {
            if matches!(db.lang_item_of_def(def), Some(TyLangItem::Future | TyLangItem::IntoFuture)) && args.len >= 1 {
                return table.shallow_resolve(table.arena.slice(args)[0]);
            }
        }
        if let Some(out) = db.project_assoc_ty(TyLangItem::IntoFutureOutput, cur, &[], &mut table.arena, &mut table.vars) {
            return table.shallow_resolve(out);
        }
        if let Some(out) = db.project_assoc_ty(TyLangItem::FutureOutput, cur, &[], &mut table.arena, &mut table.vars) {
            return table.shallow_resolve(out);
        }
        let next = match table.arena.kind(cur) {
            InferTyKind::Ref { inner, .. } => Some(table.shallow_resolve(inner)),
            InferTyKind::Adt { def, args } if matches!(db.lang_item_of_def(def), Some(TyLangItem::Pin | TyLangItem::OwnedBox)) && args.len >= 1 => {
                Some(table.shallow_resolve(table.arena.slice(args)[0]))
            }
            _ => None,
        };
        let Some(nx) = next else {
            break;
        };
        cur = nx;
    }
    InferTyId::UNKNOWN
}

/// Resolves `base[idx]` across built-in arrays/slices/strings (`usize` and `Range*<usize>`) and `Index::Output` autoderef.
pub fn infer_index_op(table: &mut InferenceTable, db: &dyn TyDefDb, base_ty: InferTyId, idx_ty: InferTyId) -> InferTyId {
    let idx_res = table.shallow_resolve(idx_ty);
    let range_elem = match table.arena.kind(idx_res) {
        InferTyKind::Adt { def, args } if db.lang_item_of_def(def).is_some_and(|l| l.is_range_adt()) => Some(if args.len >= 1 { Some(table.arena.slice(args)[0]) } else { None }),
        _ => None,
    };

    let mut cur = table.shallow_resolve(base_ty);
    for _ in 0..12 {
        match table.arena.kind(cur) {
            InferTyKind::Array { elem, .. } | InferTyKind::Slice(elem) => {
                if let Some(opt_bound) = range_elem {
                    if let Some(b) = opt_bound {
                        let _ = table.coerce(b, InferTyId::USIZE);
                    }
                    return table.arena.mk_slice(elem);
                }
                if table.coerce(idx_res, InferTyId::USIZE).is_ok() {
                    return table.shallow_resolve(elem);
                }
            }
            InferTyKind::Prim(PrimTy::Str) => {
                if let Some(opt_bound) = range_elem {
                    if let Some(b) = opt_bound {
                        let _ = table.coerce(b, InferTyId::USIZE);
                    }
                    return InferTyId::STR;
                }
            }
            _ => {}
        }
        if let Some(out) = db.project_assoc_ty(TyLangItem::IndexOutput, cur, &[idx_res], &mut table.arena, &mut table.vars) {
            return table.shallow_resolve(out);
        }
        let Some(next) = deref_step(table, db, cur) else {
            break;
        };
        if next == cur {
            break;
        }
        cur = next;
    }
    InferTyId::UNKNOWN
}

/// Extracts the element type yielded by `for pat in iter_expr` (`IntoIterator::Item` / builtin slices & ranges).
pub fn unwrap_for_iter_elem(table: &mut InferenceTable, db: &dyn TyDefDb, iter_ty: InferTyId) -> InferTyId {
    let ty = table.shallow_resolve(iter_ty);
    match table.arena.kind(ty) {
        InferTyKind::Array { elem, .. } | InferTyKind::Slice(elem) => return table.shallow_resolve(elem),
        InferTyKind::Ref { is_mut, inner } => {
            let in_res = table.shallow_resolve(inner);
            if let InferTyKind::Array { elem, .. } | InferTyKind::Slice(elem) = table.arena.kind(in_res) {
                return table.arena.mk_ref(is_mut, elem);
            }
        }
        InferTyKind::Adt { def, args } => match db.lang_item_of_def(def) {
            Some(TyLangItem::Range | TyLangItem::RangeInclusive | TyLangItem::RangeFrom | TyLangItem::Option | TyLangItem::Result) if args.len >= 1 => {
                return table.shallow_resolve(table.arena.slice(args)[0]);
            }
            _ => {}
        },
        InferTyKind::Opaque { def, args } | InferTyKind::Dyn { principal: def, args }
            if matches!(db.lang_item_of_def(def), Some(TyLangItem::Iterator | TyLangItem::IntoIterator)) && args.len >= 1 =>
        {
            return table.shallow_resolve(table.arena.slice(args)[0]);
        }
        _ => {}
    }
    db.project_assoc_ty(TyLangItem::IntoIteratorItem, ty, &[], &mut table.arena, &mut table.vars)
        .or_else(|| db.project_assoc_ty(TyLangItem::IteratorItem, ty, &[], &mut table.arena, &mut table.vars))
        .unwrap_or(InferTyId::UNKNOWN)
}

/// Constructs a `core::ops::Range*` ADT when the lang item is registered in `db`.
pub fn mk_range_adt(table: &mut InferenceTable, db: &dyn TyDefDb, item: TyLangItem, bound_ty: Option<InferTyId>) -> InferTyId {
    let Some(def) = db.lang_item(item) else {
        return bound_ty.unwrap_or(InferTyId::UNIT);
    };
    match bound_ty {
        Some(b) => table.arena.mk_adt(def, &[b]),
        None => table.arena.mk_adt(def, &[]),
    }
}

/// Extracts expected `(param_tys, ret_ty)` for a closure from an outer `Expectation` (`FnPtr`, `Closure`,
/// `dyn Fn*`, `impl Fn*`, `&dyn Fn*`, `Box<dyn Fn*>`, or an in-scope generic parameter `F: Fn*(...)`).
#[allow(clippy::too_many_arguments)]
pub fn extract_expected_callable_sig(
    table: &mut InferenceTable,
    db: &dyn TyDefDb,
    local_defs: &LocalCstDefs,
    st: &SyntaxTree<'_>,
    file_res: &FileNameRes,
    current_fn_def: GlobalDefId,
    expectation: Expectation,
    inputs_out: &mut Vec<InferTyId>,
) -> Option<(InferTyId, bool)> {
    let raw = match expectation.resolve(table) {
        Expectation::HasType(t) | Expectation::RvalueLikeUnsized(t) => t,
        Expectation::None | Expectation::CastableTo(_) => return None,
    };
    extract_callable_from_ty(table, db, local_defs, st, file_res, current_fn_def, raw, inputs_out, 0)
}

#[allow(clippy::too_many_arguments)]
fn extract_callable_from_ty(
    table: &mut InferenceTable,
    db: &dyn TyDefDb,
    local_defs: &LocalCstDefs,
    st: &SyntaxTree<'_>,
    file_res: &FileNameRes,
    current_fn_def: GlobalDefId,
    ty: InferTyId,
    inputs_out: &mut Vec<InferTyId>,
    depth: u8,
) -> Option<(InferTyId, bool)> {
    if depth > 4 {
        return None;
    }
    let resolved = table.shallow_resolve(ty);
    match table.arena.kind(resolved) {
        InferTyKind::FnPtr { sig } | InferTyKind::Closure { sig, .. } if sig.len >= 1 => {
            inputs_out.clear();
            let s = table.arena.slice(sig);
            inputs_out.extend_from_slice(&s[..s.len() - 1]);
            Some((s[s.len() - 1], false))
        }
        InferTyKind::Dyn { principal, args } | InferTyKind::Opaque { def: principal, args } => {
            let li = db.lang_item_of_def(principal)?;
            if !li.is_fn_trait() || args.len < 2 {
                return None;
            }
            let tup = table.shallow_resolve(table.arena.slice(args)[0]);
            let ret = table.shallow_resolve(table.arena.slice(args)[1]);
            inputs_out.clear();
            if let InferTyKind::Tuple(r) = table.arena.kind(tup) {
                inputs_out.extend_from_slice(table.arena.slice(r));
            }
            Some((ret, li.is_async_fn_trait()))
        }
        InferTyKind::Ref { inner, .. } => extract_callable_from_ty(table, db, local_defs, st, file_res, current_fn_def, inner, inputs_out, depth + 1),
        InferTyKind::Adt { def, args } if db.lang_item_of_def(def) == Some(TyLangItem::OwnedBox) && args.len >= 1 => {
            let inner = table.arena.slice(args)[0];
            extract_callable_from_ty(table, db, local_defs, st, file_res, current_fn_def, inner, inputs_out, depth + 1)
        }
        InferTyKind::Param { idx, .. } if current_fn_def.is_some() => {
            if let Some((li, ret)) = db.param_fn_bound(current_fn_def, idx, &mut table.arena, inputs_out) {
                return Some((ret, li.is_async_fn_trait()));
            }
            if let Some((li, ret)) = local_defs.param_fn_bound(current_fn_def, idx, st, file_res, db, table, inputs_out) {
                return Some((ret, li.is_async_fn_trait()));
            }
            None
        }
        _ => None,
    }
}
