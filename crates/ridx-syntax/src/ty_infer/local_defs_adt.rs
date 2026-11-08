/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Local CST struct/union/enum-variant constructor shape, field lowering, and `Fn*` bound extraction
//! helpers for [`super::local_defs::LocalCstDefs`].

use super::db::{CtorShapeKind, TyDefDb, TyLangItem};
use super::local_defs::{effective_local_def_id, LocalCstDefs, LocalVariantNode};
use super::lower_ty::{effective_target_def, LowerTyEnv};
use super::tok_ty_scan::{for_each_top_comma, is_double_colon, matching_delim, next_sig_idx, rfind_sig_idx, skip_leading_attrs_vis};
use super::ty::{InferTyId, InferTyKind};
use super::uf_table::InlineTyBuf;
use super::InferenceTable;
use crate::name_res::{CrateDefMap, CrateId, FileId, FileNameRes, GlobalDefId, ItemKind};
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::token::{Token, TokenKind as T};
use crate::tree::{NodeId, SyntaxTree};

pub(super) fn detect_struct_shape(st: &SyntaxTree<'_>, node: NodeId) -> CtorShapeKind {
    let Some(tt) = st.child(node, K::TokenTree) else {
        return CtorShapeKind::Unit;
    };
    let toks = st.tokens_of(tt);
    match next_sig_idx(toks, 0).map(|i| toks[i].kind) {
        Some(T::LParen) => CtorShapeKind::Tuple,
        Some(T::LBrace) => CtorShapeKind::Record,
        _ => CtorShapeKind::Unit,
    }
}

impl LocalCstDefs {
    pub(super) fn index_enum_variants(&mut self, st: &SyntaxTree<'_>, enum_node: NodeId, enum_def: GlobalDefId, enum_gc: u16, def_map: Option<&CrateDefMap>, file: FileId) {
        let Some(tt) = st.child(enum_node, K::TokenTree) else {
            return;
        };
        let toks = st.tokens_of(tt);
        let (Some(open), Some(close)) = (next_sig_idx(toks, 0), rfind_sig_idx(toks)) else {
            return;
        };
        if toks[open].kind != T::LBrace || open >= close {
            return;
        }
        let base_off = st.node(tt).first as usize;
        for_each_top_comma(&toks[open + 1..close], |clause| {
            let Some(ni) = skip_leading_attrs_vis(clause, 0) else {
                return;
            };
            let ntok = clause[ni];
            if !matches!(ntok.kind, T::Ident | T::RawIdent) {
                return;
            }
            let mut shape = CtorShapeKind::Unit;
            let mut body_range = (0u32, 0u32);
            if let Some(pi) = next_sig_idx(clause, ni + 1) {
                if matches!(clause[pi].kind, T::LParen | T::LBrace) {
                    if let Some(end_p) = matching_delim(clause, pi) {
                        shape = if clause[pi].kind == T::LParen { CtorShapeKind::Tuple } else { CtorShapeKind::Record };
                        let sub_base = (clause.as_ptr() as usize - toks.as_ptr() as usize) / core::mem::size_of::<Token>();
                        body_range = ((base_off + sub_base + pi + 1) as u32, (base_off + sub_base + end_p) as u32);
                    }
                }
            }
            let vdef = def_map
                .and_then(|dm| dm.items.iter().find(|it| it.file == file && it.kind == ItemKind::Variant && it.span_start == ntok.start && it.span_end == ntok.end))
                .map_or_else(|| GlobalDefId::new(CrateId::LOCAL_OVERLAY, 0xA000_0000 | ntok.start), |it| effective_local_def_id(it.index_def, it.id));
            self.variants.push(LocalVariantNode { def: vdef, enum_def, enum_node, enum_generics: enum_gc, shape, tok_start: body_range.0, tok_end: body_range.1 });
        });
    }

    pub fn def_ty(&self, def: GlobalDefId, st: &SyntaxTree<'_>, file_res: &FileNameRes, db: &dyn TyDefDb, table: &mut InferenceTable) -> Option<(u16, CtorShapeKind, InferTyId)> {
        if let Some(v) = self.variants.iter().find(|v| v.def == def) {
            let mut params = InlineTyBuf::new();
            for idx in 0..v.enum_generics {
                params.push(table.arena.mk_param(idx, GlobalDefId::NONE));
            }
            return Some((v.enum_generics, v.shape, table.arena.mk_adt(v.enum_def, params.as_slice())));
        }
        let item = self.items.iter().find(|i| i.def == def)?;
        match item.kind {
            ItemKind::Struct | ItemKind::Enum | ItemKind::Union => {
                let mut params = InlineTyBuf::new();
                for idx in 0..item.generic_count {
                    params.push(table.arena.mk_param(idx, GlobalDefId::NONE));
                }
                Some((item.generic_count, item.shape, table.arena.mk_adt(item.def, params.as_slice())))
            }
            ItemKind::Const | ItemKind::Static => {
                let ty = self.const_or_static_ty(def, st, file_res, db, table)?;
                Some((0, CtorShapeKind::None, ty))
            }
            ItemKind::TypeAlias => {
                let (gc, ty) = self.lower_type_alias(def, st, file_res, db, table, 0)?;
                Some((gc, CtorShapeKind::None, ty))
            }
            _ => None,
        }
    }

    pub fn ctor_tuple_fields(
        &self,
        def: GlobalDefId,
        st: &SyntaxTree<'_>,
        file_res: &FileNameRes,
        db: &dyn TyDefDb,
        table: &mut InferenceTable,
        fields_out: &mut Vec<InferTyId>,
    ) -> Option<u16> {
        let (owner_node, gc, inner_toks) = self.body_tokens_of(st, def, CtorShapeKind::Tuple)?;
        let env = LowerTyEnv { st, file_res, db, local_defs: self, self_ty: None, impl_owner: None, impl_generics: 0, item_owner: Some(owner_node), in_body: false };
        fields_out.clear();
        for_each_top_comma(inner_toks, |clause| {
            if let Some(ti) = skip_leading_attrs_vis(clause, 0) {
                fields_out.push(env.lower_tok_slice_ty(table, &clause[ti..], 0));
            }
        });
        Some(gc)
    }

    pub fn field_ty(
        &self,
        owner_def: GlobalDefId,
        name: &str,
        st: &SyntaxTree<'_>,
        file_res: &FileNameRes,
        db: &dyn TyDefDb,
        table: &mut InferenceTable,
    ) -> Option<(u16, InferTyId)> {
        self.field_ty_and_def(owner_def, name, st, file_res, db, table).map(|(gc, ty, _)| (gc, ty))
    }

    pub fn field_ty_and_def(
        &self,
        owner_def: GlobalDefId,
        name: &str,
        st: &SyntaxTree<'_>,
        file_res: &FileNameRes,
        db: &dyn TyDefDb,
        table: &mut InferenceTable,
    ) -> Option<(u16, InferTyId, GlobalDefId)> {
        if let Ok(tuple_idx) = name.parse::<usize>() {
            let (owner_node, gc, inner_toks) = self.body_tokens_of(st, owner_def, CtorShapeKind::Tuple)?;
            let env = LowerTyEnv { st, file_res, db, local_defs: self, self_ty: None, impl_owner: None, impl_generics: 0, item_owner: Some(owner_node), in_body: false };
            let mut cur_i = 0usize;
            let mut found = None;
            for_each_top_comma(inner_toks, |clause| {
                if found.is_some() {
                    return;
                }
                if let Some(ti) = skip_leading_attrs_vis(clause, 0) {
                    if cur_i == tuple_idx {
                        let fdef = GlobalDefId::new(CrateId::LOCAL_OVERLAY, 0xB000_0000 | clause[ti].start);
                        found = Some((env.lower_tok_slice_ty(table, &clause[ti..], 0), fdef));
                    }
                    cur_i += 1;
                }
            });
            return found.map(|(t, d)| (gc, t, d));
        }
        let (owner_node, gc, inner_toks) = self.body_tokens_of(st, owner_def, CtorShapeKind::Record)?;
        let env = LowerTyEnv { st, file_res, db, local_defs: self, self_ty: None, impl_owner: None, impl_generics: 0, item_owner: Some(owner_node), in_body: false };
        let mut found = None;
        for_each_top_comma(inner_toks, |clause| {
            if found.is_some() {
                return;
            }
            let Some(ni) = skip_leading_attrs_vis(clause, 0) else {
                return;
            };
            if !matches!(clause[ni].kind, T::Ident | T::RawIdent) || norm_ident(clause[ni].text(st.src)) != name {
                return;
            }
            if let Some(ci) = next_sig_idx(clause, ni + 1).filter(|&i| clause[i].kind == T::Colon && !is_double_colon(clause, i)) {
                let fdef = GlobalDefId::new(CrateId::LOCAL_OVERLAY, 0xB000_0000 | clause[ni].start);
                found = Some((env.lower_tok_slice_ty(table, &clause[ci + 1..], 0), fdef));
            }
        });
        found.map(|(t, d)| (gc, t, d))
    }

    fn body_tokens_of<'s>(&self, st: &'s SyntaxTree<'_>, def: GlobalDefId, expected: CtorShapeKind) -> Option<(NodeId, u16, &'s [Token])> {
        if let Some(v) = self.variants.iter().find(|v| v.def == def && v.shape == expected) {
            return Some((v.enum_node, v.enum_generics, &st.tokens[v.tok_start as usize..v.tok_end as usize]));
        }
        let item = self.items.iter().find(|i| i.def == def && i.shape == expected)?;
        let tt = st.child(item.node, K::TokenTree)?;
        let toks = st.tokens_of(tt);
        let (open, close) = (next_sig_idx(toks, 0)?, rfind_sig_idx(toks)?);
        (open < close).then_some((item.node, item.generic_count, &toks[open + 1..close]))
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn extract_cst_fn_bound(
    st: &SyntaxTree<'_>,
    file_res: &FileNameRes,
    db: &dyn TyDefDb,
    fn_node: NodeId,
    impl_gc: u16,
    param_idx: u16,
    env: &LowerTyEnv<'_, '_>,
    table: &mut InferenceTable,
    inputs_out: &mut Vec<InferTyId>,
) -> Option<(TyLangItem, InferTyId)> {
    let mut target_name: Option<&str> = None;
    if param_idx >= impl_gc {
        let local_idx = (param_idx - impl_gc) as usize;
        if let Some(gpl) = st.child(fn_node, K::GenericParamList) {
            if let Some(tp) = st.children(gpl).filter(|&c| matches!(st.kind(c), K::TypeParam | K::ConstParam)).nth(local_idx) {
                target_name = st.child(tp, K::Name).map(|nn| norm_ident(st.text(nn)));
                if let Some(tbl) = st.child(tp, K::TypeBoundList) {
                    if let Some(res) = scan_bound_list_for_fn(st, file_res, db, tbl, env, table, inputs_out) {
                        return Some(res);
                    }
                }
            }
        }
    }
    let name = target_name?;
    let wc = st.child(fn_node, K::WhereClause)?;
    for wp in st.children(wc).filter(|&c| st.kind(c) == K::WherePred) {
        let Some(lhs) = st.children(wp).find(|&c| st.kind(c).is_type()) else {
            continue;
        };
        if norm_ident(st.text(lhs).trim()) != name {
            continue;
        }
        if let Some(tbl) = st.child(wp, K::TypeBoundList) {
            if let Some(res) = scan_bound_list_for_fn(st, file_res, db, tbl, env, table, inputs_out) {
                return Some(res);
            }
        }
    }
    None
}

fn scan_bound_list_for_fn(
    st: &SyntaxTree<'_>,
    file_res: &FileNameRes,
    db: &dyn TyDefDb,
    tbl: NodeId,
    env: &LowerTyEnv<'_, '_>,
    table: &mut InferenceTable,
    inputs_out: &mut Vec<InferTyId>,
) -> Option<(TyLangItem, InferTyId)> {
    for tb in st.children(tbl).filter(|&c| st.kind(c) == K::TypeBound) {
        let Some(p) = st.child(tb, K::Path) else {
            continue;
        };
        let Some(rref) = file_res.path_tail_ref(p) else {
            continue;
        };
        let tdef = effective_target_def(rref.target);
        let li = db.lang_item_of_def(tdef).or_else(|| {
            let tail = st.children(p).filter(|&c| st.kind(c) == K::PathSegment).last()?;
            let (_, tok) = st.own_tokens(tail).find(|(_, t)| matches!(t.kind, T::Ident | T::RawIdent))?;
            match norm_ident(tok.text(st.src)) {
                "Fn" => Some(TyLangItem::Fn),
                "FnMut" => Some(TyLangItem::FnMut),
                "FnOnce" => Some(TyLangItem::FnOnce),
                "AsyncFn" => Some(TyLangItem::AsyncFn),
                "AsyncFnMut" => Some(TyLangItem::AsyncFnMut),
                "AsyncFnOnce" => Some(TyLangItem::AsyncFnOnce),
                _ => None,
            }
        })?;
        if !li.is_fn_trait() {
            continue;
        }
        let tail = st.children(p).filter(|&c| st.kind(c) == K::PathSegment).last()?;
        let mut args = InlineTyBuf::new();
        env.collect_segment_args(table, tail, &mut args, 0);
        if args.as_slice().len() >= 2 {
            let tup = args.as_slice()[0];
            let ret = args.as_slice()[1];
            inputs_out.clear();
            if let InferTyKind::Tuple(r) = table.arena.kind(tup) {
                inputs_out.extend_from_slice(table.arena.slice(r));
            }
            return Some((li, ret));
        }
    }
    None
}
