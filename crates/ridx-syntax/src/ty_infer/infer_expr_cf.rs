/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Branching and loop control-flow expression inference (`if`/`else`, `match`, `loop`, `while`,
//! `for`, `break`, `return`) for [`super::body_ctx::Runner`].

use super::body_ctx::{BreakTarget, Runner};
use super::expectation::{BranchCoercer, Expectation};
use super::infer_expr::is_expr_node;
use super::infer_pat::BindingMode;
use super::lang_ops::unwrap_for_iter_elem;
use super::ty::InferTyId;
use crate::syntax_kind::SyntaxKind as K;
use crate::token::TokenKind as T;
use crate::tree::NodeId;

pub(super) fn extract_label_span(st: &crate::tree::SyntaxTree<'_>, node: NodeId) -> Option<(u32, u32)> {
    let lbl = st.child(node, K::Label)?;
    let (_, tok) = st.own_tokens(lbl).find(|(_, t)| t.kind == T::Lifetime)?;
    Some((tok.start, tok.end))
}

impl<'a, 's> Runner<'a, 's> {
    pub(super) fn infer_if_expr(&mut self, expr: NodeId, expectation: Expectation) -> InferTyId {
        let mut kids = self.st.children(expr).filter(|&c| is_expr_node(self.st.kind(c)));
        if let Some(cond) = kids.next() {
            let cty = self.infer_expr(cond, Expectation::HasType(InferTyId::BOOL));
            let _ = self.coerce_expr(cty, InferTyId::BOOL);
        }
        let Some(then_blk) = kids.next() else {
            return InferTyId::UNIT;
        };
        if let Some(else_node) = kids.next() {
            let mut coercer = BranchCoercer::new(&mut self.ctx.table, expectation);
            let t_exp = coercer.arm_expectation();
            let t_ty = self.infer_expr(then_blk, t_exp);
            coercer.merge_arm(&mut self.ctx.table, t_ty);
            let e_exp = coercer.arm_expectation();
            let e_ty = self.infer_expr(else_node, e_exp);
            coercer.merge_arm(&mut self.ctx.table, e_ty);
            coercer.finish(&mut self.ctx.table)
        } else {
            let t_ty = self.infer_expr(then_blk, Expectation::HasType(InferTyId::UNIT));
            let _ = self.coerce_expr(t_ty, InferTyId::UNIT);
            InferTyId::UNIT
        }
    }

    pub(super) fn infer_match_expr(&mut self, expr: NodeId, expectation: Expectation) -> InferTyId {
        let scrut_ty = self.st.children(expr).find(|&c| is_expr_node(self.st.kind(c))).map_or(InferTyId::UNKNOWN, |s| self.infer_expr(s, Expectation::None));
        let mut coercer = BranchCoercer::new(&mut self.ctx.table, expectation);
        if let Some(mal) = self.st.child(expr, K::MatchArmList) {
            for arm in self.st.children(mal).filter(|&c| self.st.kind(c) == K::MatchArm) {
                if let Some(pat) = self.st.children(arm).find(|&c| self.st.kind(c).is_pat()) {
                    self.infer_pat(pat, scrut_ty, BindingMode::ByValue, None);
                }
                if let Some(mg) = self.st.child(arm, K::MatchGuard) {
                    if let Some(ge) = self.st.children(mg).find(|&c| is_expr_node(self.st.kind(c))) {
                        let gty = self.infer_expr(ge, Expectation::HasType(InferTyId::BOOL));
                        let _ = self.coerce_expr(gty, InferTyId::BOOL);
                    }
                }
                if let Some(body) = self.st.children(arm).find(|&c| is_expr_node(self.st.kind(c))) {
                    let arm_exp = coercer.arm_expectation();
                    let bty = self.infer_expr(body, arm_exp);
                    coercer.merge_arm(&mut self.ctx.table, bty);
                }
            }
        }
        coercer.finish(&mut self.ctx.table)
    }

    pub(super) fn infer_loop_expr(&mut self, expr: NodeId, expectation: Expectation) -> InferTyId {
        let (ls, le) = extract_label_span(self.st, expr).unwrap_or((0, 0));
        let coercer = BranchCoercer::new(&mut self.ctx.table, expectation);
        self.ctx.break_stack.push(BreakTarget { label_start: ls, label_end: le, coercer, break_count: 0, allow_unlabeled: true, allow_value: true });
        if let Some(body) = self.st.child(expr, K::BlockExpr) {
            let _ = self.infer_expr(body, Expectation::HasType(InferTyId::UNIT));
        }
        let Some(bt) = self.ctx.break_stack.pop() else {
            return InferTyId::NEVER;
        };
        if bt.break_count == 0 {
            InferTyId::NEVER
        } else {
            bt.coercer.finish(&mut self.ctx.table)
        }
    }

    pub(super) fn infer_while_expr(&mut self, expr: NodeId) -> InferTyId {
        let (ls, le) = extract_label_span(self.st, expr).unwrap_or((0, 0));
        let mut kids = self.st.children(expr).filter(|&c| is_expr_node(self.st.kind(c)));
        if let Some(cond) = kids.next() {
            let cty = self.infer_expr(cond, Expectation::HasType(InferTyId::BOOL));
            let _ = self.coerce_expr(cty, InferTyId::BOOL);
        }
        let coercer = BranchCoercer::new(&mut self.ctx.table, Expectation::HasType(InferTyId::UNIT));
        self.ctx.break_stack.push(BreakTarget { label_start: ls, label_end: le, coercer, break_count: 0, allow_unlabeled: true, allow_value: false });
        if let Some(body) = kids.next() {
            let _ = self.infer_expr(body, Expectation::HasType(InferTyId::UNIT));
        }
        self.ctx.break_stack.pop();
        InferTyId::UNIT
    }

    pub(super) fn infer_for_expr(&mut self, expr: NodeId) -> InferTyId {
        let (ls, le) = extract_label_span(self.st, expr).unwrap_or((0, 0));
        let mut kids = self.st.children(expr).filter(|&c| is_expr_node(self.st.kind(c)));
        let iter_ty = kids.next().map_or(InferTyId::UNKNOWN, |it| self.infer_expr(it, Expectation::None));
        let elem_ty = unwrap_for_iter_elem(&mut self.ctx.table, self.db, iter_ty);
        if let Some(pat) = self.st.children(expr).find(|&c| self.st.kind(c).is_pat()) {
            self.infer_pat(pat, elem_ty, BindingMode::ByValue, None);
        }
        let coercer = BranchCoercer::new(&mut self.ctx.table, Expectation::HasType(InferTyId::UNIT));
        self.ctx.break_stack.push(BreakTarget { label_start: ls, label_end: le, coercer, break_count: 0, allow_unlabeled: true, allow_value: false });
        if let Some(body) = kids.next() {
            let _ = self.infer_expr(body, Expectation::HasType(InferTyId::UNIT));
        }
        self.ctx.break_stack.pop();
        InferTyId::UNIT
    }

    pub(super) fn infer_break_expr(&mut self, expr: NodeId) -> InferTyId {
        let target_lbl = self.st.child(expr, K::Lifetime).map(|lt| self.st.text(lt));
        let target_idx = self.ctx.break_stack.iter().rposition(|bt| match target_lbl {
            Some(lbl) => bt.label_end > bt.label_start && &self.st.src[bt.label_start as usize..bt.label_end as usize] == lbl,
            None => bt.allow_unlabeled,
        });
        let val_exp = target_idx.filter(|&i| self.ctx.break_stack[i].allow_value).map_or(Expectation::None, |i| self.ctx.break_stack[i].coercer.arm_expectation());
        let val_ty = self.st.children(expr).find(|&c| is_expr_node(self.st.kind(c))).map_or(InferTyId::UNIT, |v| self.infer_expr(v, val_exp));
        if let Some(i) = target_idx {
            let allow_val = self.ctx.break_stack[i].allow_value;
            self.ctx.break_stack[i].break_count += 1;
            if allow_val {
                self.ctx.break_stack[i].coercer.merge_arm(&mut self.ctx.table, val_ty);
            }
        }
        InferTyId::NEVER
    }

    pub(super) fn infer_return_expr(&mut self, expr: NodeId) -> InferTyId {
        let ret_exp = self.ctx.ret_stack.last().copied().unwrap_or(InferTyId::UNIT);
        let val_ty = self.st.children(expr).find(|&c| is_expr_node(self.st.kind(c))).map_or(InferTyId::UNIT, |v| self.infer_expr(v, Expectation::HasType(ret_exp)));
        let _ = self.coerce_expr(val_ty, ret_exp);
        InferTyId::NEVER
    }
}
