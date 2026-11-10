/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Constituent type walking (`LiveTy.defs`) matching `rust-analyzer` `goto_type_definition`:
//! collects all ADTs, traits, and type aliases in pre-order (including omitted default generic
//! type arguments such as `A = Global` and implicit `Sized` on `impl Trait` opaques) while
//! excluding `impl` blocks, associated items, functions, and non-`Self` type parameters.

use std::path::Path;

use ridx_format::{DefKind, GenericParamKind, TyKindView};
use ridx_syntax::name_res::{CrateDefMap, CrateId, GlobalDefId, ItemKind as IK};
use ridx_syntax::ty_infer::{FnInferResult, InferTyArena, InferTyId, InferTyKind, LocalCstDefs, TyDefDb, TyLangItem};
use ridx_syntax::SyntaxKind as K;

use super::query::LiveLocation;
use super::query_loc::{resolve_def_location, CachedParsed};
use super::query_ty_fmt::TyReader;
use super::v5_db::{RidxV5ItemDb, CRATE_ROOT_DEF_IDX, IMPL_SYNTH_BIT, PROC_MACRO_SYNTH_BIT, TY_DEF_SYNTH_BIT};

#[inline]
fn is_type_def_gid(db: &RidxV5ItemDb<'_>, dm: &CrateDefMap, raw_def: GlobalDefId, is_param_zero: bool) -> bool {
    if !raw_def.is_some() {
        return false;
    }
    if raw_def.krate == CrateId::LOCAL_OVERLAY {
        return dm.items.iter().find(|it| it.file.is_some() && (it.index_def == raw_def || (0x8000_0000 | it.id.0) == raw_def.index || it.id.0 == raw_def.index)).is_some_and(
            |it| {
                if is_param_zero {
                    it.kind == IK::Trait
                } else {
                    matches!(it.kind, IK::Struct | IK::Enum | IK::Union | IK::Trait | IK::TypeAlias)
                }
            },
        );
    }
    let def = db.canonical_def(raw_def);
    if !def.is_some() || def.index == CRATE_ROOT_DEF_IDX || (def.index & (IMPL_SYNTH_BIT | PROC_MACRO_SYNTH_BIT)) != 0 {
        return false;
    }
    if (def.index & TY_DEF_SYNTH_BIT) != 0 {
        return !is_param_zero && (def.index & 0x4000_0000) == 0;
    }
    let Some(rdr) = db.ws.readers().get(def.krate.0 as usize) else {
        return false;
    };
    let Some(d) = rdr.definitions().get(def.index as usize) else {
        return false;
    };
    let dk = DefKind::from(d.kind);
    if is_param_zero {
        matches!(dk, DefKind::Trait | DefKind::TraitAlias)
    } else {
        matches!(dk, DefKind::Struct | DefKind::Enum | DefKind::Union | DefKind::Trait | DefKind::TyAlias | DefKind::TraitAlias | DefKind::ForeignTy)
    }
}

#[inline]
#[allow(clippy::too_many_arguments)]
fn push_ty_def_loc(
    db: &RidxV5ItemDb<'_>,
    dm: &CrateDefMap,
    active_canon: &Path,
    parsed: &CachedParsed,
    local_defs: Option<&LocalCstDefs>,
    g: GlobalDefId,
    is_param_zero: bool,
    out: &mut Vec<LiveLocation>,
) {
    if !is_type_def_gid(db, dm, g, is_param_zero) {
        return;
    }
    if let Some(loc) = resolve_def_location(db, dm, active_canon, parsed, local_defs, g) {
        if !out.contains(&loc) {
            out.push(loc);
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn collect_ty_def_locs<A: TyReader>(
    db: &RidxV5ItemDb<'_>,
    dm: &CrateDefMap,
    active_canon: &Path,
    parsed: &CachedParsed,
    local_defs: Option<&LocalCstDefs>,
    fn_infer: Option<&FnInferResult>,
    cursor_off: u32,
    arena: &A,
    ty: InferTyId,
    depth: u8,
    out: &mut Vec<LiveLocation>,
) {
    collect_ty_def_locs_inner::<A, A>(db, dm, active_canon, parsed, local_defs, fn_infer, cursor_off, arena, ty, None, depth, out);
}

#[allow(clippy::too_many_arguments)]
fn collect_ty_def_locs_inner<A: TyReader, S: TyReader>(
    db: &RidxV5ItemDb<'_>,
    dm: &CrateDefMap,
    active_canon: &Path,
    parsed: &CachedParsed,
    local_defs: Option<&LocalCstDefs>,
    fn_infer: Option<&FnInferResult>,
    cursor_off: u32,
    arena: &A,
    ty: InferTyId,
    param_subst: Option<(&S, &[InferTyId])>,
    depth: u8,
    out: &mut Vec<LiveLocation>,
) {
    if !ty.is_some() || depth > 24 {
        return;
    }
    match arena.kind(ty) {
        InferTyKind::Adt { def, args } => {
            push_ty_def_loc(db, dm, active_canon, parsed, local_defs, def, false, out);
            let explicit = arena.slice(args);
            for &a in explicit {
                collect_ty_def_locs_inner(db, dm, active_canon, parsed, local_defs, fn_infer, cursor_off, arena, a, param_subst, depth + 1, out);
            }
            collect_omitted_default_args(db, dm, active_canon, parsed, local_defs, fn_infer, cursor_off, arena, def, explicit, depth + 1, out);
        }
        InferTyKind::Dyn { principal, args } => {
            push_ty_def_loc(db, dm, active_canon, parsed, local_defs, principal, false, out);
            for &a in arena.slice(args) {
                collect_ty_def_locs_inner(db, dm, active_canon, parsed, local_defs, fn_infer, cursor_off, arena, a, param_subst, depth + 1, out);
            }
        }
        InferTyKind::Opaque { def: principal, args } => {
            push_ty_def_loc(db, dm, active_canon, parsed, local_defs, principal, false, out);
            if let Some(sized_gid) = db.lang_item(TyLangItem::Sized) {
                push_ty_def_loc(db, dm, active_canon, parsed, local_defs, sized_gid, false, out);
            }
            for &a in arena.slice(args) {
                collect_ty_def_locs_inner(db, dm, active_canon, parsed, local_defs, fn_infer, cursor_off, arena, a, param_subst, depth + 1, out);
            }
            if let Some(res) = fn_infer.filter(|_| param_subst.is_none()) {
                collect_rpit_origin_defs(db, dm, active_canon, parsed, local_defs, res, cursor_off, arena, ty, principal, depth + 1, out);
            }
        }
        InferTyKind::Alias { trait_def, assoc_def: _, self_ty, args } => {
            push_ty_def_loc(db, dm, active_canon, parsed, local_defs, trait_def, false, out);
            collect_ty_def_locs_inner(db, dm, active_canon, parsed, local_defs, fn_infer, cursor_off, arena, self_ty, param_subst, depth + 1, out);
            for &a in arena.slice(args) {
                collect_ty_def_locs_inner(db, dm, active_canon, parsed, local_defs, fn_infer, cursor_off, arena, a, param_subst, depth + 1, out);
            }
        }
        InferTyKind::Ref { inner, .. } | InferTyKind::Ptr { inner, .. } | InferTyKind::Slice(inner) | InferTyKind::Array { elem: inner, .. } => {
            collect_ty_def_locs_inner(db, dm, active_canon, parsed, local_defs, fn_infer, cursor_off, arena, inner, param_subst, depth + 1, out);
        }
        InferTyKind::Tuple(r) | InferTyKind::FnPtr { sig: r } | InferTyKind::Closure { sig: r, .. } | InferTyKind::FnDef { args: r, .. } => {
            for &el in arena.slice(r) {
                collect_ty_def_locs_inner(db, dm, active_canon, parsed, local_defs, fn_infer, cursor_off, arena, el, param_subst, depth + 1, out);
            }
        }
        InferTyKind::Param { idx, def } => {
            if let Some((subst_arena, subst_args)) = param_subst {
                if let Some(&mapped) = subst_args.get(idx as usize) {
                    collect_ty_def_locs_inner::<S, S>(db, dm, active_canon, parsed, local_defs, fn_infer, cursor_off, subst_arena, mapped, None, depth + 1, out);
                    return;
                }
            }
            if idx == 0 {
                push_ty_def_loc(db, dm, active_canon, parsed, local_defs, def, true, out);
            }
        }
        _ => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_rpit_origin_defs<A: TyReader>(
    db: &RidxV5ItemDb<'_>,
    dm: &CrateDefMap,
    active_canon: &Path,
    parsed: &CachedParsed,
    local_defs: Option<&LocalCstDefs>,
    res: &FnInferResult,
    cursor_off: u32,
    arena: &A,
    opaque_ty: InferTyId,
    principal: GlobalDefId,
    depth: u8,
    out: &mut Vec<LiveLocation>,
) {
    if depth > 24 || db.lang_item_of_def(principal).is_some_and(|l| l.is_fn_trait()) {
        return;
    }
    let mut extra_gids: Vec<GlobalDefId> = Vec::new();
    let mut extra_tys: Vec<InferTyId> = Vec::new();
    {
        let st = parsed.tree();
        let cands = res
            .expr_tys
            .iter()
            .rev()
            .filter(|&&(n, t)| t == opaque_ty && matches!(st.kind(n), K::MethodCallExpr | K::CallExpr) && st.range(n).start <= cursor_off as usize)
            .chain(res.expr_tys.iter().filter(|&&(n, t)| t == opaque_ty && matches!(st.kind(n), K::MethodCallExpr | K::CallExpr) && st.range(n).start > cursor_off as usize));
        for &(enode, _) in cands {
            let callee_def = if st.kind(enode) == K::MethodCallExpr {
                st.child(enode, K::Name).map(|nm| st.range(nm)).and_then(|nr| res.dot_refs.iter().find(|d| d.start as usize == nr.start && d.end as usize == nr.end)).map(|d| d.def)
            } else {
                st.children(enode).next().and_then(|cn| res.expr_tys.iter().find(|&&(n, _)| n == cn)).and_then(|&(_, cty)| match arena.kind(cty) {
                    InferTyKind::FnDef { def, .. } => Some(def),
                    _ => None,
                })
            };
            let Some(raw_callee) = callee_def else { continue };
            let cdef = db.canonical_def(raw_callee);
            let r_idx = cdef.krate.0 as usize;
            let Some(reader) = db.ws.readers().get(r_idx) else { continue };
            let Some(sig) = reader.fn_sig_for_def(cdef.index) else { continue };
            let TyKindView::ImplTrait { bounds } = reader.ty_arena().kind(sig.output()) else { continue };
            let c_prin = db.canonical_def(principal);
            for b in bounds {
                if !b.is_maybe {
                    let g2 = db.ty_def_gid(r_idx, b.trait_def);
                    if g2.is_some() && db.canonical_def(g2) != c_prin {
                        extra_gids.push(g2);
                    }
                }
            }
            if reader.generic_params_for_def(cdef.index).any(|p| !matches!(p.kind, GenericParamKind::Lifetime)) {
                if let Some(al) = st.child(enode, K::ArgList) {
                    for arg_n in st.children(al) {
                        if let Some(&(_, aty)) = res.expr_tys.iter().find(|&&(n, _)| n == arg_n) {
                            if aty != opaque_ty {
                                extra_tys.push(aty);
                            }
                        }
                    }
                }
            }
            break;
        }
    }
    for g2 in extra_gids {
        push_ty_def_loc(db, dm, active_canon, parsed, local_defs, g2, false, out);
    }
    for aty in extra_tys {
        collect_ty_def_locs_inner::<A, A>(db, dm, active_canon, parsed, local_defs, Some(res), cursor_off, arena, aty, None, depth + 1, out);
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_omitted_default_args<A: TyReader>(
    db: &RidxV5ItemDb<'_>,
    dm: &CrateDefMap,
    active_canon: &Path,
    parsed: &CachedParsed,
    local_defs: Option<&LocalCstDefs>,
    fn_infer: Option<&FnInferResult>,
    cursor_off: u32,
    arena: &A,
    raw_def: GlobalDefId,
    explicit: &[InferTyId],
    depth: u8,
    out: &mut Vec<LiveLocation>,
) {
    if depth > 24 || raw_def.krate == CrateId::LOCAL_OVERLAY {
        return;
    }
    let cdef = db.canonical_def(raw_def);
    if !cdef.is_some() || cdef.index == CRATE_ROOT_DEF_IDX || (cdef.index & (IMPL_SYNTH_BIT | PROC_MACRO_SYNTH_BIT | TY_DEF_SYNTH_BIT)) != 0 {
        return;
    }
    let Some(reader) = db.ws.readers().get(cdef.krate.0 as usize) else {
        return;
    };
    let k = explicit.len();
    let mut non_lt_idx = 0usize;
    let mut scratch: Option<InferTyArena> = None;
    for gp in reader.generic_params_for_def(cdef.index) {
        if matches!(gp.kind, GenericParamKind::Lifetime) || gp.is_self() {
            continue;
        }
        let cur = non_lt_idx;
        non_lt_idx += 1;
        if cur < k || !matches!(gp.kind, GenericParamKind::Type) || !gp.has_default() {
            continue;
        }
        let sa = scratch.get_or_insert_with(InferTyArena::default);
        if let Some(dty) = db.v5_generic_param_default_ty(cdef, cur as u16, sa) {
            collect_ty_def_locs_inner(db, dm, active_canon, parsed, local_defs, fn_infer, cursor_off, sa, dty, Some((arena, explicit)), depth + 1, out);
        }
    }
}
