/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Body macro inference (`Runner::infer_macro_call`), zero-allocation sub-tree [`Runner`] dispatch
//! over [`crate::parser::ParseScratch`], and hygiene-preserving span/binding mapping (`Checkbox 3.6.2`).

use crate::name_res::mbe_expand::{map_span_range, ExpandedSpanMap};
use crate::name_res::{GlobalDefId, ItemKind, PathCtx, ResTarget};
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::token::{Token, TokenKind as T};
use crate::tree::{NodeId, SyntaxTree};

use super::body_ctx::{ResolvedDotRef, Runner};
use super::expectation::Expectation;
use super::infer_macro_std::classify_std_macro_path;
use super::infer_pat::BindingMode;
use super::tok_ty_scan::next_sig_idx;
pub(super) use super::tok_ty_scan::{SubInferEnv, SubLocalEntry};
use super::ty::InferTyId;

pub(super) const MAX_RUNTIME_MACRO_DEPTH: usize = 6;

impl<'a, 's> Runner<'a, 's> {
    #[inline]
    pub(super) fn map_span_to_orig(&self, start: u32, end: u32) -> Option<(u32, u32)> {
        match self.sub {
            None | Some(SubInferEnv::StdSlice { .. }) => Some((start, end)),
            Some(SubInferEnv::Expanded { spans, .. }) => map_span_range(spans, start, end),
        }
    }

    #[inline]
    pub(super) fn map_expr_span_to_orig(&self, start: u32, end: u32, kind: K) -> Option<(u32, u32)> {
        self.map_span_to_orig(start, end).or(match (kind, self.sub) {
            (K::MacroCall, Some(SubInferEnv::Expanded { callsite_start, callsite_end, .. })) => Some((callsite_start, callsite_end)),
            _ => None,
        })
    }

    #[inline]
    pub(super) fn closure_def_span(&self, expr: NodeId) -> (u32, u32) {
        let er = self.st.range(expr);
        let hdr_end = self.st.child(expr, K::RetType).or_else(|| self.st.child(expr, K::ParamList)).map_or(er.end, |n| self.st.range(n).end);
        let (hs, he) = (er.start as u32, hdr_end as u32);
        match self.sub {
            None | Some(SubInferEnv::StdSlice { .. }) => (hs, he),
            Some(SubInferEnv::Expanded { spans, callsite_start, callsite_end, .. }) => map_span_range(spans, hs, he).unwrap_or((callsite_start, callsite_end)),
        }
    }

    #[inline]
    pub(super) fn record_dot_ref(&mut self, start: u32, end: u32, def: GlobalDefId, kind: ItemKind) {
        if let Some((os, oe)) = self.map_span_to_orig(start, end) {
            self.out.dot_refs.push(ResolvedDotRef { start: os, end: oe, def, kind });
        }
    }

    #[inline]
    pub(super) fn record_incomplete_dot(&mut self, off: u32, ty: InferTyId) {
        let mapped = match self.sub {
            None | Some(SubInferEnv::StdSlice { .. }) => Some(off),
            Some(SubInferEnv::Expanded { spans, .. }) => off.checked_sub(1).and_then(|s| map_span_range(spans, s, off)).map(|(_, e)| e),
        };
        if let Some(moff) = mapped {
            self.out.incomplete_dots.push((moff, ty));
        }
    }

    #[inline]
    pub(super) fn bind_sub_local(&mut self, name: &str, ty: InferTyId) {
        if let Some(entry) = SubLocalEntry::new(name, ty) {
            self.ctx.sub_locals_buf.push(entry);
        }
    }

    #[inline]
    pub(super) fn lookup_sub_local(&self, name: &str) -> Option<InferTyId> {
        self.ctx.sub_locals_buf.iter().rev().find(|e| e.matches(name)).map(|e| e.ty)
    }

    pub(super) fn lookup_sub_or_expanded_value(&mut self, path_node: NodeId) -> Option<InferTyId> {
        let env = self.sub?;
        let mut segs = self.st.children(path_node).filter(|&c| self.st.kind(c) == K::PathSegment);
        let first_seg = segs.next()?;
        let tail_seg = segs.last().unwrap_or(first_seg);
        if first_seg == tail_seg && !self.st.own_tokens(path_node).any(|(_, t)| t.kind == T::Colon) {
            let ident = norm_ident(self.st.text(first_seg).trim());
            if let Some(sty) = self.lookup_sub_local(ident) {
                return Some(sty);
            }
        }
        let (ts, te) = self.st.own_tokens(tail_seg).find(|(_, t)| matches!(t.kind, T::Ident | T::RawIdent | T::SelfKw | T::SelfType)).map_or_else(
            || {
                let r = self.st.range(tail_seg);
                (r.start as u32, r.end as u32)
            },
            |(_, t)| (t.start, t.end),
        );
        let (orig_s, orig_e, root_slot_base, root_slots_len) = match env {
            SubInferEnv::StdSlice { .. } => (ts, te, self.ctx.local_slot_base, self.ctx.local_slots.len() as u32),
            SubInferEnv::Expanded { spans, root_slot_base, root_slots_len, .. } => {
                let (os, oe) = map_span_range(spans, ts, te)?;
                if let Some(ResTarget::Local(pre_lid)) = self.file_res.path_tail_ref_in_st(self.st, path_node).map(|r| r.target) {
                    if self.file_res.locals.get(pre_lid.0 as usize).is_some_and(|ld| map_span_range(spans, ld.name_start, ld.name_end).is_some()) {
                        return None;
                    }
                }
                (os, oe, root_slot_base, root_slots_len)
            }
        };
        let caller_ref = self.root_file_res.refs_at_span(orig_s, orig_e).iter().find(|rf| matches!(rf.ctx, PathCtx::Expr | PathCtx::PatValue))?;
        match caller_ref.target {
            ResTarget::Local(root_lid) => {
                if let Some(&(_, sh_ty)) = self.ctx.macro_local_shadows.iter().rev().find(|&&(id, _)| id == root_lid) {
                    return Some(sh_ty);
                }
                let off = root_lid.0.wrapping_sub(root_slot_base) as usize;
                (off < root_slots_len as usize).then(|| self.ctx.local_slots.get(off).copied().filter(|t| t.is_some())).flatten().or(Some(InferTyId::UNKNOWN))
            }
            target => Some(self.finish_value_path_target(path_node, target)),
        }
    }

    pub(super) fn infer_macro_call(&mut self, mac_node: NodeId, expectation: Expectation, in_stmt: bool) -> InferTyId {
        let raw = self
            .ctx
            .macro_depth
            .lt(&MAX_RUNTIME_MACRO_DEPTH)
            .then(|| self.try_infer_pre_expanded(mac_node, expectation, in_stmt))
            .flatten()
            .unwrap_or_else(|| self.infer_unexpanded_macro(mac_node, expectation, in_stmt));
        let ty = match expectation.coercion_target(&mut self.ctx.table) {
            Some(exp) => self.coerce_expr(raw, exp),
            None => self.ctx.table.shallow_resolve(raw),
        };
        self.record_expr(mac_node, ty)
    }

    fn try_infer_pre_expanded(&mut self, mac_node: NodeId, expectation: Expectation, in_stmt: bool) -> Option<InferTyId> {
        if self.sub.is_some() {
            return None;
        }
        let pre = self.local_defs.find_body_macro(mac_node)?;
        let idx = self.ctx.macro_depth;
        if idx >= self.ctx.macro_scratch.len() {
            return None;
        }
        let mut scratch = core::mem::replace(&mut self.ctx.macro_scratch[idx], crate::parser::ParseScratch::new());
        self.ctx.macro_depth += 1;

        let prev_slot_base = self.ctx.local_slot_base;
        let sub_slot_off = self.ctx.local_slots.len();
        let pre_locs = pre.file_res.locals.len();
        self.ctx.local_slots.resize(sub_slot_off + pre_locs, InferTyId::NONE);
        self.ctx.local_slot_base = 0u32.wrapping_sub(sub_slot_off as u32);
        let local_mark = self.ctx.sub_locals_buf.len() as u32;

        let (cs_node, cs_start, cs_end, root_slot_base, root_slots_len) = match self.sub {
            Some(SubInferEnv::Expanded { callsite_node, callsite_start, callsite_end, root_slot_base, root_slots_len, .. }) => {
                (callsite_node, callsite_start, callsite_end, root_slot_base, root_slots_len)
            }
            Some(SubInferEnv::StdSlice { callsite_node, .. }) => {
                let cr = self.root_st.range(callsite_node);
                (callsite_node, cr.start as u32, cr.end as u32, prev_slot_base, sub_slot_off as u32)
            }
            None => {
                let cr = self.st.range(mac_node);
                (mac_node, cr.start as u32, cr.end as u32, prev_slot_base, sub_slot_off as u32)
            }
        };
        let sub_env = SubInferEnv::Expanded { callsite_node: cs_node, callsite_start: cs_start, callsite_end: cs_end, spans: &pre.spans, root_slot_base, root_slots_len };

        let (res_ty, pre_st) = {
            let pre_st = scratch.borrow_prebuilt(&pre.src, &pre.tokens, &pre.nodes);
            let ty = {
                let mut child = Runner {
                    ctx: self.ctx,
                    st: &pre_st,
                    file_res: &pre.file_res,
                    root_st: self.root_st,
                    root_file_res: self.root_file_res,
                    db: self.db,
                    local_defs: self.local_defs,
                    fn_node: pre.fn_node,
                    root_fn_node: self.root_fn_node,
                    fn_def: self.fn_def,
                    impl_node: None,
                    root_impl_node: self.root_impl_node,
                    impl_gc: self.impl_gc,
                    self_ty: self.self_ty,
                    loc_lo: 0,
                    loc_hi: pre_locs,
                    expr_depth: self.expr_depth + 1,
                    sub: Some(sub_env),
                    out: self.out,
                };
                let bty = child.infer_expr(pre.block_node, if in_stmt { Expectation::None } else { expectation });
                child.ctx.sub_locals_buf.truncate(local_mark as usize);
                if in_stmt {
                    child.export_stmt_macro_shadows(pre.block_node, &pre.spans, sub_slot_off);
                }
                bty
            };
            (ty, pre_st)
        };

        scratch.reclaim(pre_st);
        self.ctx.local_slots.truncate(sub_slot_off);
        self.ctx.local_slot_base = prev_slot_base;
        self.ctx.macro_depth -= 1;
        self.ctx.macro_scratch[idx] = scratch;
        Some(res_ty)
    }

    fn export_stmt_macro_shadows(&mut self, block_node: NodeId, spans: &[ExpandedSpanMap], sub_slot_off: usize) {
        for ch in self.st.children(block_node) {
            match self.st.kind(ch) {
                K::LetStmt => {
                    let Some(pat) = self.st.children(ch).find(|&c| self.st.kind(c) == K::IdentPat) else { continue };
                    let (Some(nn), Some(pre_lid)) = (self.st.child(pat, K::Name), self.find_decl_local(pat)) else { continue };
                    let Some(bound_ty) = self.ctx.local_slots.get(sub_slot_off + pre_lid.0 as usize).copied().filter(|t| t.is_some() && *t != InferTyId::UNKNOWN) else {
                        continue;
                    };
                    let nr = self.st.range(nn);
                    let Some((orig_s, orig_e)) = map_span_range(spans, nr.start as u32, nr.end as u32) else { continue };
                    if let Some(ResTarget::Local(root_lid)) = self.root_file_res.refs_at_span(orig_s, orig_e).iter().find_map(|r| match r.target {
                        t @ ResTarget::Local(_) => Some(t),
                        _ => None,
                    }) {
                        self.ctx.macro_local_shadows.push((root_lid, bound_ty));
                    }
                }
                K::Static | K::Const => {
                    if let (Some(nn), Some(tn)) = (self.st.child(ch, K::Name), self.st.children(ch).find(|&c| self.st.kind(c).is_type())) {
                        let ty = self.ty_env(true).lower_cst_ty(&mut self.ctx.table, tn);
                        if ty != InferTyId::UNKNOWN {
                            self.bind_sub_local(norm_ident(self.st.text(nn)), ty);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn infer_unexpanded_macro(&mut self, mac_node: NodeId, expectation: Expectation, in_stmt: bool) -> InferTyId {
        let Some(tt) = self.st.child(mac_node, K::TokenTree) else { return InferTyId::UNKNOWN };
        let toks = self.st.tokens_of(tt);
        if toks.len() < 2 {
            return InferTyId::UNKNOWN;
        }
        let inner = &toks[1..toks.len() - 1];
        if let Some(kind) = self.st.child(mac_node, K::Path).and_then(|p| classify_std_macro_path(self.st, p)) {
            if let Some(ty) = self.try_infer_std_macro(mac_node, kind, inner, expectation, in_stmt) {
                return ty;
            }
        }
        self.infer_fallback_macro_tt(inner);
        InferTyId::UNKNOWN
    }

    pub(super) fn slice_sub_env(&self) -> SubInferEnv<'a> {
        match self.sub {
            Some(exp @ SubInferEnv::Expanded { .. }) => exp,
            Some(SubInferEnv::StdSlice { callsite_node }) => SubInferEnv::StdSlice { callsite_node },
            None => SubInferEnv::StdSlice { callsite_node: self.fn_node },
        }
    }

    pub(super) fn run_on_slice_st<'b>(&'b mut self, slice_st: &'b SyntaxTree<'s>, sub_env: SubInferEnv<'b>, f: impl FnOnce(&mut Runner<'b, 's>) -> InferTyId) -> InferTyId {
        let mut child = Runner {
            ctx: self.ctx,
            st: slice_st,
            file_res: self.file_res,
            root_st: self.root_st,
            root_file_res: self.root_file_res,
            db: self.db,
            local_defs: self.local_defs,
            fn_node: self.fn_node,
            root_fn_node: self.root_fn_node,
            fn_def: self.fn_def,
            impl_node: self.impl_node,
            root_impl_node: self.root_impl_node,
            impl_gc: self.impl_gc,
            self_ty: self.self_ty,
            loc_lo: self.loc_lo,
            loc_hi: self.loc_hi,
            expr_depth: self.expr_depth + 1,
            sub: Some(sub_env),
            out: self.out,
        };
        f(&mut child)
    }

    pub(super) fn infer_tok_expr_slice(&mut self, toks: &[Token], expectation: Expectation) -> InferTyId {
        let idx = self.ctx.macro_depth;
        if idx >= self.ctx.macro_scratch.len() || next_sig_idx(toks, 0).is_none() {
            return InferTyId::UNKNOWN;
        }
        let mut scratch = core::mem::replace(&mut self.ctx.macro_scratch[idx], crate::parser::ParseScratch::new());
        self.ctx.macro_depth += 1;
        let mark = self.ctx.sub_locals_buf.len();
        let sub_env = self.slice_sub_env();
        let (ty, slice_st) = {
            let (slice_st, expr_opt) = scratch.parse_expr_slice(self.st.src, toks);
            let t = expr_opt.map_or(InferTyId::UNKNOWN, |en| self.run_on_slice_st(&slice_st, sub_env, |ch| ch.infer_expr(en, expectation)));
            (t, slice_st)
        };
        self.ctx.sub_locals_buf.truncate(mark);
        scratch.reclaim(slice_st);
        self.ctx.macro_depth -= 1;
        self.ctx.macro_scratch[idx] = scratch;
        ty
    }

    pub(super) fn infer_tok_pat_slice(&mut self, toks: &[Token], expected_ty: InferTyId) {
        let idx = self.ctx.macro_depth;
        if idx >= self.ctx.macro_scratch.len() || next_sig_idx(toks, 0).is_none() {
            return;
        }
        let mut scratch = core::mem::replace(&mut self.ctx.macro_scratch[idx], crate::parser::ParseScratch::new());
        self.ctx.macro_depth += 1;
        let mark = self.ctx.sub_locals_buf.len();
        let sub_env = self.slice_sub_env();
        let slice_st = {
            let (slice_st, pat_opt) = scratch.parse_pat_slice(self.st.src, toks);
            if let Some(pn) = pat_opt {
                let _ = self.run_on_slice_st(&slice_st, sub_env, |ch| ch.infer_pat(pn, expected_ty, BindingMode::ByValue, None));
            }
            slice_st
        };
        self.ctx.sub_locals_buf.truncate(mark);
        scratch.reclaim(slice_st);
        self.ctx.macro_depth -= 1;
        self.ctx.macro_scratch[idx] = scratch;
    }

    pub(super) fn infer_std_thread_local(&mut self, mac_node: NodeId) -> InferTyId {
        let Some(tt) = self.st.child(mac_node, K::TokenTree) else { return InferTyId::UNIT };
        let toks = self.st.tokens_of(tt);
        let idx = self.ctx.macro_depth;
        if idx >= self.ctx.macro_scratch.len() || toks.first().is_none_or(|t| t.kind != T::LBrace) {
            return InferTyId::UNIT;
        }
        let mut scratch = core::mem::replace(&mut self.ctx.macro_scratch[idx], crate::parser::ParseScratch::new());
        self.ctx.macro_depth += 1;
        let sub_env = self.slice_sub_env();
        let slice_st = {
            let (slice_st, expr_opt) = scratch.parse_expr_slice(self.st.src, toks);
            if let Some(en) = expr_opt {
                let _ = self.run_on_slice_st(&slice_st, sub_env, |ch| {
                    for item in slice_st.children(en).filter(|&c| matches!(slice_st.kind(c), K::Static | K::Const)) {
                        let ann_ty = slice_st.children(item).find(|&c| slice_st.kind(c).is_type()).map(|tn| ch.ty_env(true).lower_cst_ty(&mut ch.ctx.table, tn));
                        let mark = ch.ctx.sub_locals_buf.len();
                        if let Some(ie) = slice_st.children(item).find(|&c| super::infer_expr::is_expr_node(slice_st.kind(c))) {
                            let ity = ch.infer_expr(ie, Expectation::from_option(ann_ty));
                            if let Some(at) = ann_ty {
                                let _ = ch.coerce_expr(ity, at);
                            }
                        }
                        ch.ctx.sub_locals_buf.truncate(mark);
                        if let (Some(nn), Some(at)) = (slice_st.child(item, K::Name), ann_ty) {
                            if let Some(lk) = ch.db.std_macro_ty(super::db::StdMacroTy::ThreadLocalKey(at), &mut ch.ctx.table.arena) {
                                ch.bind_sub_local(norm_ident(slice_st.text(nn)), lk);
                            }
                        }
                    }
                    InferTyId::UNIT
                });
            }
            slice_st
        };
        scratch.reclaim(slice_st);
        self.ctx.macro_depth -= 1;
        self.ctx.macro_scratch[idx] = scratch;
        InferTyId::UNIT
    }
}
