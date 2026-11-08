/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Core expression, literal, path, block, `let`, `if`/`match`, and loop/control-flow inference
//! (`Runner::infer_expr`); call, struct, operator, `?`, `.await`, range, index, and closure
//! inference live in [`super::infer_expr_ops`].

use super::body_ctx::{BreakTarget, Runner};
use super::db::{CtorShapeKind, TyLangItem};
use super::expectation::{BranchCoercer, Expectation};
use super::infer_expr_cf::extract_label_span;
use super::infer_pat::BindingMode;
use super::lower_ty::effective_target_def;
use super::tok_ty_scan::eval_const_uint_expr;
use super::ty::{InferTyId, InferTyKind};
use super::uf_table::InlineTyBuf;
use crate::name_res::{ItemKind, ResTarget};
use crate::syntax_kind::SyntaxKind as K;
use crate::token::TokenKind as T;
use crate::tree::NodeId;

#[inline]
pub(super) fn is_expr_node(k: K) -> bool {
    k.is_expr() || k == K::MacroCall
}

impl<'a, 's> Runner<'a, 's> {
    pub(super) fn infer_expr(&mut self, expr: NodeId, expectation: Expectation) -> InferTyId {
        let raw = match self.st.kind(expr) {
            K::Literal => self.infer_literal_tokens(expr),
            K::PathExpr => self.st.child(expr, K::Path).map_or(InferTyId::UNKNOWN, |p| self.infer_value_path(p)),
            K::ParenExpr => {
                let inner = self.st.children(expr).find(|&c| is_expr_node(self.st.kind(c))).map_or(InferTyId::UNIT, |c| self.infer_expr(c, expectation));
                return self.record_expr(expr, inner);
            }
            K::TupleExpr => self.infer_tuple_expr(expr, expectation),
            K::ArrayExpr => self.infer_array_expr(expr, expectation),
            K::BlockExpr => {
                let ty = self.infer_block_expr(expr, expectation);
                return self.record_expr(expr, ty);
            }
            K::IfExpr => {
                let ty = self.infer_if_expr(expr, expectation);
                return self.record_expr(expr, ty);
            }
            K::MatchExpr => {
                let ty = self.infer_match_expr(expr, expectation);
                return self.record_expr(expr, ty);
            }
            K::LoopExpr => {
                let ty = self.infer_loop_expr(expr, expectation);
                return self.record_expr(expr, ty);
            }
            K::WhileExpr => self.infer_while_expr(expr),
            K::ForExpr => self.infer_for_expr(expr),
            K::BreakExpr => self.infer_break_expr(expr),
            K::ContinueExpr => InferTyId::NEVER,
            K::ReturnExpr => self.infer_return_expr(expr),
            K::LetExpr => self.infer_let_expr(expr),
            K::UnderscoreExpr => self.ctx.table.new_ty_var(),
            K::CallExpr => self.infer_call_expr(expr, expectation),
            K::RecordExpr => self.infer_record_expr(expr, expectation),
            K::FieldExpr => self.infer_field_expr(expr),
            K::IndexExpr => self.infer_index_expr_node(expr),
            K::TryExpr => self.infer_try_expr(expr),
            K::AwaitExpr => self.infer_await_expr(expr),
            K::RefExpr => return self.infer_ref_expr(expr, expectation),
            K::PrefixExpr => self.infer_prefix_expr(expr, expectation),
            K::BinExpr => self.infer_bin_expr(expr),
            K::CastExpr => self.infer_cast_expr(expr),
            K::RangeExpr => self.infer_range_expr(expr),
            K::ClosureExpr => return self.infer_closure_expr(expr, expectation),
            K::MethodCallExpr => self.infer_method_call_fallback(expr),
            _ => InferTyId::UNKNOWN,
        };
        let ty = match expectation.coercion_target(&mut self.ctx.table) {
            Some(exp) => self.coerce_expr(raw, exp),
            None => self.ctx.table.shallow_resolve(raw),
        };
        self.record_expr(expr, ty)
    }

    pub(super) fn collect_path_turbofish(&mut self, path_node: NodeId, out: &mut InlineTyBuf) {
        let tenv = self.ty_env(true);
        for seg in self.st.children(path_node).filter(|&c| self.st.kind(c) == K::PathSegment) {
            if let Some(gal) = self.st.child(seg, K::GenericArgList) {
                for ga in self.st.children(gal) {
                    if matches!(self.st.kind(ga), K::TypeArg | K::AssocTypeArg) {
                        if let Some(tn) = self.st.children(ga).find(|&c| self.st.kind(c).is_type()) {
                            out.push(tenv.lower_cst_ty(&mut self.ctx.table, tn));
                        }
                    } else if self.st.kind(ga) == K::ConstArg {
                        out.push(InferTyId::UNKNOWN);
                    }
                }
            }
        }
    }

    pub(super) fn infer_value_path(&mut self, path_node: NodeId) -> InferTyId {
        let mut explicit = InlineTyBuf::new();
        self.collect_path_turbofish(path_node, &mut explicit);
        let Some(rref) = self.file_res.path_tail_ref(path_node) else {
            return InferTyId::UNKNOWN;
        };
        self.instantiate_target_value(rref.target, explicit.as_slice())
    }

    pub(super) fn instantiate_target_value(&mut self, target: ResTarget, explicit_args: &[InferTyId]) -> InferTyId {
        match target {
            ResTarget::Local(lid) => self.get_local(lid),
            ResTarget::Prim(p) => InferTyId::from_prim(p),
            ResTarget::Generic(gid) => {
                let g = self.file_res.generic_data(gid);
                if g.is_const {
                    let tenv = self.ty_env(false);
                    self.st.children(g.decl_node).find(|&c| self.st.kind(c).is_type()).map_or(InferTyId::UNKNOWN, |tn| tenv.lower_cst_ty(&mut self.ctx.table, tn))
                } else {
                    InferTyId::UNKNOWN
                }
            }
            ResTarget::SelfTy { .. } => {
                let Some(sty) = self.self_ty else {
                    return InferTyId::UNKNOWN;
                };
                let res_sty = self.ctx.table.shallow_resolve(sty);
                if let InferTyKind::Adt { def, args } = self.ctx.table.arena.kind(res_sty) {
                    let mut subst = InlineTyBuf::new();
                    for &a in self.ctx.table.arena.slice(args) {
                        subst.push(a);
                    }
                    if let Some(fp) = self.tuple_ctor_fn_ptr(def, subst.as_slice(), sty) {
                        return fp;
                    }
                }
                sty
            }
            ResTarget::Def { kind, .. } => {
                let eff_def = effective_target_def(target);
                if !eff_def.is_some() {
                    return InferTyId::UNKNOWN;
                }
                match kind {
                    ItemKind::Fn | ItemKind::AssocFn => {
                        let mut scratch = core::mem::take(&mut self.ctx.sig_inputs_buf);
                        let gc = self
                            .db
                            .fn_sig(eff_def, &mut self.ctx.table.arena, &mut scratch)
                            .or_else(|| self.local_defs.fn_sig(eff_def, self.st, self.file_res, self.db, &mut self.ctx.table, &mut scratch))
                            .map(|(g, _)| g)
                            .unwrap_or(explicit_args.len() as u16);
                        self.ctx.sig_inputs_buf = scratch;
                        let mut subst = InlineTyBuf::new();
                        for i in 0..gc as usize {
                            subst.push(explicit_args.get(i).copied().unwrap_or_else(|| self.ctx.table.new_ty_var()));
                        }
                        self.ctx.table.arena.mk_fn_def(eff_def, subst.as_slice())
                    }
                    ItemKind::Struct | ItemKind::Variant | ItemKind::TypeAlias => {
                        let info =
                            self.db.def_ty(eff_def, &mut self.ctx.table.arena).or_else(|| self.local_defs.def_ty(eff_def, self.st, self.file_res, self.db, &mut self.ctx.table));
                        let Some((gc, shape, tpl_ty)) = info else {
                            return InferTyId::UNKNOWN;
                        };
                        let mut subst = InlineTyBuf::new();
                        for i in 0..gc as usize {
                            subst.push(explicit_args.get(i).copied().unwrap_or_else(|| self.ctx.table.new_ty_var()));
                        }
                        let inst_ty = self.ctx.table.instantiate(tpl_ty, subst.as_slice());
                        if shape == CtorShapeKind::Tuple {
                            if let Some(fp) = self.tuple_ctor_fn_ptr(eff_def, subst.as_slice(), inst_ty) {
                                return fp;
                            }
                        }
                        inst_ty
                    }
                    ItemKind::Const | ItemKind::Static | ItemKind::AssocConst => self
                        .db
                        .def_ty(eff_def, &mut self.ctx.table.arena)
                        .or_else(|| self.local_defs.def_ty(eff_def, self.st, self.file_res, self.db, &mut self.ctx.table))
                        .map_or(InferTyId::UNKNOWN, |(_, _, t)| t),
                    _ => InferTyId::UNKNOWN,
                }
            }
        }
    }

    fn tuple_ctor_fn_ptr(&mut self, def: crate::name_res::GlobalDefId, subst: &[InferTyId], ret_ty: InferTyId) -> Option<InferTyId> {
        let mut raw = core::mem::take(&mut self.ctx.sig_inputs_buf);
        let ok = self.db.ctor_tuple_fields(def, &mut self.ctx.table.arena, &mut raw).is_some()
            || self.local_defs.ctor_tuple_fields(def, self.st, self.file_res, self.db, &mut self.ctx.table, &mut raw).is_some();
        let res = ok.then(|| {
            let mut ins = InlineTyBuf::new();
            for &f in &raw {
                ins.push(self.ctx.table.instantiate(f, subst));
            }
            self.ctx.table.arena.mk_fn_ptr(ins.as_slice(), ret_ty)
        });
        self.ctx.sig_inputs_buf = raw;
        res
    }

    fn infer_tuple_expr(&mut self, expr: NodeId, expectation: Expectation) -> InferTyId {
        let mut kids = [NodeId(0); 24];
        let mut len = 0usize;
        for ch in self.st.children(expr).filter(|&c| is_expr_node(self.st.kind(c))) {
            if len < kids.len() {
                kids[len] = ch;
                len += 1;
            }
        }
        let mut exps = core::mem::take(&mut self.ctx.exp_elems_buf);
        expectation.tuple_elem_expectations(&mut self.ctx.table, len, &mut exps);
        let mut out = InlineTyBuf::new();
        for (i, &ch) in kids[..len].iter().enumerate() {
            let e_exp = exps.get(i).copied().unwrap_or(Expectation::None);
            out.push(self.infer_expr(ch, e_exp));
        }
        self.ctx.exp_elems_buf = exps;
        self.ctx.table.arena.mk_tuple(out.as_slice())
    }

    fn infer_array_expr(&mut self, expr: NodeId, expectation: Expectation) -> InferTyId {
        let elem_exp = expectation.array_elem_expectation(&mut self.ctx.table);
        let is_repeat = self.st.own_tokens(expr).any(|(_, t)| t.kind == T::Semi);
        let mut expr_kids = self.st.children(expr).filter(|&c| is_expr_node(self.st.kind(c)));
        if is_repeat {
            let elem_ty = match expr_kids.next() {
                Some(e) => self.infer_expr(e, elem_exp),
                None => self.ctx.table.new_ty_var(),
            };
            let len_node = expr_kids.next();
            if let Some(ln) = len_node {
                let lty = self.infer_expr(ln, Expectation::HasType(InferTyId::USIZE));
                let _ = self.coerce_expr(lty, InferTyId::USIZE);
            }
            let n = len_node.and_then(|ln| eval_const_uint_expr(self.st, ln)).unwrap_or(u64::MAX);
            return self.ctx.table.arena.mk_array(elem_ty, n);
        }
        let mut coercer = BranchCoercer::new(&mut self.ctx.table, elem_exp);
        let mut count = 0u64;
        for ch in expr_kids {
            let arm_exp = coercer.arm_expectation();
            let ety = self.infer_expr(ch, arm_exp);
            coercer.merge_arm(&mut self.ctx.table, ety);
            count += 1;
        }
        let elem_ty = if count == 0 { elem_exp.coercion_target(&mut self.ctx.table).unwrap_or_else(|| self.ctx.table.new_ty_var()) } else { coercer.finish(&mut self.ctx.table) };
        self.ctx.table.arena.mk_array(elem_ty, count)
    }

    fn infer_block_expr(&mut self, block: NodeId, expectation: Expectation) -> InferTyId {
        let is_async = self.st.own_tokens(block).next().is_some_and(|(_, t)| t.kind == T::Async);
        let inner_exp = if is_async {
            expectation
                .only_has_type(&mut self.ctx.table)
                .map(|t| super::lang_ops::unwrap_await_ty(&mut self.ctx.table, self.db, t))
                .filter(|&t| t != InferTyId::UNKNOWN)
                .map_or(Expectation::None, Expectation::HasType)
        } else {
            expectation
        };
        let label_span = extract_label_span(self.st, block);
        if let Some((ls, le)) = label_span {
            let coercer = BranchCoercer::new(&mut self.ctx.table, inner_exp);
            self.ctx.break_stack.push(BreakTarget { label_start: ls, label_end: le, coercer, break_count: 0, allow_unlabeled: false, allow_value: true });
        }
        let async_ret_var = is_async.then(|| {
            let v = inner_exp.coercion_target(&mut self.ctx.table).unwrap_or_else(|| self.ctx.table.new_ty_var());
            self.ctx.ret_stack.push(v);
            v
        });

        let mut diverged = false;
        let mut tail_ty: Option<InferTyId> = None;
        for ch in self.st.children(block) {
            match self.st.kind(ch) {
                K::LetStmt => {
                    if self.infer_let_stmt(ch) == InferTyId::NEVER {
                        diverged = true;
                    }
                }
                K::ExprStmt => {
                    if let Some(en) = self.st.children(ch).find(|&c| is_expr_node(self.st.kind(c))) {
                        if self.infer_expr(en, Expectation::None) == InferTyId::NEVER {
                            diverged = true;
                        }
                    }
                }
                k if is_expr_node(k) => {
                    let t = self.infer_expr(ch, inner_exp);
                    if t == InferTyId::NEVER {
                        diverged = true;
                    }
                    tail_ty = Some(t);
                }
                _ => {}
            }
        }
        let mut res_ty = tail_ty.unwrap_or(if diverged { InferTyId::NEVER } else { InferTyId::UNIT });
        if let Some(exp) = inner_exp.coercion_target(&mut self.ctx.table) {
            res_ty = self.coerce_expr(res_ty, exp);
        }
        if let Some(aret) = async_ret_var {
            self.ctx.ret_stack.pop();
            res_ty = self.coerce_expr(res_ty, aret);
            let out_ty = self.ctx.table.shallow_resolve(aret);
            if let Some(fut_def) = self.db.lang_item(TyLangItem::Future) {
                res_ty = self.ctx.table.arena.mk_opaque(fut_def, &[out_ty]);
            }
        }
        if label_span.is_some() {
            if let Some(mut bt) = self.ctx.break_stack.pop() {
                if bt.break_count > 0 {
                    bt.coercer.merge_arm(&mut self.ctx.table, res_ty);
                    res_ty = bt.coercer.finish(&mut self.ctx.table);
                }
            }
        }
        res_ty
    }

    fn infer_let_stmt(&mut self, stmt: NodeId) -> InferTyId {
        let tenv = self.ty_env(true);
        let ann_ty = self.st.children(stmt).find(|&c| self.st.kind(c).is_type()).map(|tn| tenv.lower_cst_ty(&mut self.ctx.table, tn));
        let init_ty = self.st.children(stmt).find(|&c| is_expr_node(self.st.kind(c))).map(|en| self.infer_expr(en, Expectation::from_option(ann_ty)));
        if let Some(le) = self.st.child(stmt, K::LetElse) {
            if let Some(eb) = self.st.child(le, K::BlockExpr) {
                let _ = self.infer_expr(eb, Expectation::HasType(InferTyId::NEVER));
            }
        }
        let pat_ty = match (ann_ty, init_ty) {
            (Some(ann), Some(init)) => {
                let _ = self.coerce_expr(init, ann);
                ann
            }
            (Some(ann), None) => ann,
            (None, Some(init)) => init,
            (None, None) => self.ctx.table.new_ty_var(),
        };
        if let Some(pat) = self.st.children(stmt).find(|&c| self.st.kind(c).is_pat()) {
            self.infer_pat(pat, pat_ty, BindingMode::ByValue, None);
        }
        if init_ty == Some(InferTyId::NEVER) {
            InferTyId::NEVER
        } else {
            InferTyId::UNIT
        }
    }

    pub(super) fn infer_let_expr(&mut self, expr: NodeId) -> InferTyId {
        let init_ty = self.st.children(expr).find(|&c| is_expr_node(self.st.kind(c))).map_or(InferTyId::UNKNOWN, |en| self.infer_expr(en, Expectation::None));
        if let Some(pat) = self.st.children(expr).find(|&c| self.st.kind(c).is_pat()) {
            self.infer_pat(pat, init_ty, BindingMode::ByValue, None);
        }
        InferTyId::BOOL
    }
}
