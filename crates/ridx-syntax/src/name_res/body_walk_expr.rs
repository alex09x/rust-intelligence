/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Block, statement, and expression walker methods for [`super::body_walk::FileResolver`].

use super::body_ribs::block_has_inner_items;
use super::body_walk::FileResolver;
use super::db::{GlobalDefId, ItemKind};
use super::path_res::{PathCtx, ResTarget};
use super::per_ns::LocalItemId;
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::token::TokenKind as T;
use crate::tree::NodeId;

impl<'a, 's> FileResolver<'a, 's> {
    #[inline]
    pub(super) fn expr_cfg_enabled(&self, node: NodeId) -> bool {
        if self.st.children(node).next().is_none_or(|c| self.st.kind(c) != K::Attr) {
            return true;
        }
        super::attr::eval_outer_attrs(self.st, node, self.db, self.map.krate).enabled
    }

    pub(super) fn walk_block_expr(&mut self, block: NodeId, is_fn_body: bool) {
        let is_const_block = !is_fn_body && self.st.own_tokens(block).next().is_some_and(|(_, t)| t.kind == T::Const);
        let saved_lb = is_const_block.then(|| self.ribs.enter_assoc_item_barrier());
        let loc_mark = self.ribs.locals_active.len();
        let has_items = block_has_inner_items(self.st, block);
        if has_items {
            self.ribs.push_block_rib(self.map, self.db, self.mod_tree, self.ext_cache, self.file, self.cur_mod, self.st, block);
        }

        for ch in self.st.children(block) {
            match self.st.kind(ch) {
                K::LetStmt => {
                    if !self.expr_cfg_enabled(ch) {
                        continue;
                    }
                    if let Some(tn) = self.st.children(ch).find(|&c| self.st.kind(c).is_type()) {
                        self.walk_type(tn);
                    }
                    if let Some(en) = self.st.children(ch).find(|&c| self.st.kind(c).is_expr()) {
                        self.walk_expr(en);
                    }
                    if let Some(le) = self.st.child(ch, K::LetElse) {
                        if let Some(eb) = self.st.child(le, K::BlockExpr) {
                            self.walk_block_expr(eb, false);
                        }
                    }
                    if let Some(pn) = self.st.children(ch).find(|&c| self.st.kind(c).is_pat()) {
                        self.walk_pat(pn, None, false);
                    }
                }
                K::ExprStmt => {
                    if self.expr_cfg_enabled(ch) {
                        if let Some(en) = self.st.children(ch).find(|&c| self.st.kind(c).is_expr()) {
                            self.walk_expr(en);
                        }
                    }
                }
                k if k.is_item() => self.walk_block_scoped_item(ch),
                k if k.is_expr() && self.expr_cfg_enabled(ch) => self.walk_expr(ch),
                _ => {}
            }
        }

        if has_items {
            self.ribs.pop_block_rib();
        }
        self.ribs.locals_active.truncate(loc_mark);
        if let Some(lb) = saved_lb {
            self.ribs.local_barrier = lb;
        }
    }

    pub(super) fn walk_expr(&mut self, expr: NodeId) {
        if !self.expr_cfg_enabled(expr) {
            return;
        }
        match self.st.kind(expr) {
            K::PathExpr => {
                if let Some(p) = self.st.child(expr, K::Path) {
                    self.resolve_cst_path(p, PathCtx::Expr);
                }
            }
            K::RecordExpr => self.walk_record_expr(expr),
            K::BlockExpr => self.walk_block_expr(expr, false),
            K::LetExpr => {
                if let Some(en) = self.st.children(expr).find(|&c| self.st.kind(c).is_expr()) {
                    self.walk_expr(en);
                }
                if let Some(pn) = self.st.children(expr).find(|&c| self.st.kind(c).is_pat()) {
                    self.walk_pat(pn, None, false);
                }
            }
            K::IfExpr => {
                let mut expr_kids = self.st.children(expr).filter(|&c| self.st.kind(c).is_expr());
                let (cond_opt, then_opt, else_opt) = (expr_kids.next(), expr_kids.next(), expr_kids.next());
                let cond_mark = self.ribs.locals_active.len();
                if let Some(cond) = cond_opt {
                    self.walk_expr(cond);
                }
                if let Some(then_blk) = then_opt {
                    self.walk_expr(then_blk);
                }
                self.ribs.locals_active.truncate(cond_mark);
                if let Some(else_br) = else_opt {
                    self.walk_expr(else_br);
                }
            }
            K::WhileExpr => {
                let mut expr_kids = self.st.children(expr).filter(|&c| self.st.kind(c).is_expr());
                let (cond_opt, body_opt) = (expr_kids.next(), expr_kids.next());
                let cond_mark = self.ribs.locals_active.len();
                if let Some(cond) = cond_opt {
                    self.walk_expr(cond);
                }
                if let Some(body) = body_opt {
                    self.walk_expr(body);
                }
                self.ribs.locals_active.truncate(cond_mark);
            }
            K::ForExpr => {
                let pat_opt = self.st.children(expr).find(|&c| self.st.kind(c).is_pat());
                let mut expr_kids = self.st.children(expr).filter(|&c| self.st.kind(c).is_expr());
                let (iter_opt, body_opt) = (expr_kids.next(), expr_kids.next());
                if let Some(it_expr) = iter_opt {
                    self.walk_expr(it_expr);
                }
                let for_mark = self.ribs.locals_active.len();
                if let Some(pat) = pat_opt {
                    self.walk_pat(pat, None, false);
                }
                if let Some(body) = body_opt {
                    self.walk_expr(body);
                }
                self.ribs.locals_active.truncate(for_mark);
            }
            K::MatchExpr => {
                if let Some(scrut) = self.st.children(expr).find(|&c| self.st.kind(c).is_expr()) {
                    self.walk_expr(scrut);
                }
                if let Some(al) = self.st.child(expr, K::MatchArmList) {
                    for arm in self.st.children(al).filter(|&c| self.st.kind(c) == K::MatchArm) {
                        if !self.expr_cfg_enabled(arm) {
                            continue;
                        }
                        let arm_mark = self.ribs.locals_active.len();
                        if let Some(pn) = self.st.children(arm).find(|&c| self.st.kind(c).is_pat()) {
                            self.walk_pat(pn, None, false);
                        }
                        if let Some(mg) = self.st.child(arm, K::MatchGuard) {
                            if let Some(ge) = self.st.children(mg).find(|&c| self.st.kind(c).is_expr()) {
                                self.walk_expr(ge);
                            }
                        }
                        if let Some(be) = self.st.children(arm).find(|&c| self.st.kind(c).is_expr()) {
                            self.walk_expr(be);
                        }
                        self.ribs.locals_active.truncate(arm_mark);
                    }
                }
            }
            K::ClosureExpr => {
                let cl_mark = self.ribs.locals_active.len();
                if let Some(pl) = self.st.child(expr, K::ParamList) {
                    for param in self.st.children(pl).filter(|&c| self.st.kind(c) == K::Param) {
                        if !self.expr_cfg_enabled(param) {
                            continue;
                        }
                        if let Some(tn) = self.st.children(param).find(|&c| self.st.kind(c).is_type()) {
                            self.walk_type(tn);
                        }
                        if let Some(pn) = self.st.children(param).find(|&c| self.st.kind(c).is_pat()) {
                            self.walk_pat(pn, None, false);
                        }
                    }
                }
                if let Some(rt) = self.st.child(expr, K::RetType) {
                    if let Some(tn) = self.st.children(rt).find(|&c| self.st.kind(c).is_type()) {
                        self.walk_type(tn);
                    }
                }
                if let Some(body) = self.st.children(expr).find(|&c| self.st.kind(c).is_expr()) {
                    self.walk_expr(body);
                }
                self.ribs.locals_active.truncate(cl_mark);
            }
            K::CastExpr => {
                for ch in self.st.children(expr) {
                    if self.st.kind(ch).is_expr() {
                        self.walk_expr(ch);
                    } else if self.st.kind(ch).is_type() {
                        self.walk_type(ch);
                    }
                }
            }
            K::MethodCallExpr => {
                for ch in self.st.children(expr) {
                    match self.st.kind(ch) {
                        k if k.is_expr() => self.walk_expr(ch),
                        K::GenericArgList => self.walk_generic_arg_list(ch, GlobalDefId::NONE),
                        K::ArgList => {
                            for arg in self.st.children(ch).filter(|&a| self.st.kind(a).is_expr()) {
                                self.walk_expr(arg);
                            }
                        }
                        _ => {}
                    }
                }
            }
            K::CallExpr => {
                for ch in self.st.children(expr) {
                    if self.st.kind(ch).is_expr() {
                        self.walk_expr(ch);
                    } else if self.st.kind(ch) == K::ArgList {
                        for arg in self.st.children(ch).filter(|&a| self.st.kind(a).is_expr()) {
                            self.walk_expr(arg);
                        }
                    }
                }
            }
            K::MacroCall => {
                if let Some(p) = self.st.child(expr, K::Path) {
                    self.resolve_cst_path(p, PathCtx::MacroCall);
                }
            }
            _ => {
                for ch in self.st.children(expr) {
                    if self.st.kind(ch).is_expr() {
                        self.walk_expr(ch);
                    }
                }
            }
        }
    }

    fn walk_record_expr(&mut self, rec_expr: NodeId) {
        let owner_def =
            self.st.child(rec_expr, K::Path).and_then(|p| self.resolve_cst_path(p, PathCtx::RecordExpr)).map_or(GlobalDefId::NONE, |rt| self.adt_or_variant_def_from_target(rt));
        let Some(fl) = self.st.child(rec_expr, K::RecordExprFieldList) else {
            return;
        };
        for ch in self.st.children(fl) {
            if self.st.kind(ch) == K::RecordExprField {
                if !self.expr_cfg_enabled(ch) {
                    continue;
                }
                if self.st.own_tokens(ch).any(|(_, t)| t.kind == T::Colon) && owner_def.is_some() {
                    if let Some((_, ftok)) = self.st.own_tokens(ch).find(|(_, t)| matches!(t.kind, T::Ident | T::RawIdent | T::Int)) {
                        if let Some(fdef) = self.db.adt_field(owner_def, norm_ident(ftok.text(self.st.src))) {
                            let rt = ResTarget::Def { def: fdef, local: LocalItemId::NONE, kind: ItemKind::Field };
                            self.emit_ref(ftok.start, ftok.end, NodeId(u32::MAX), rt, PathCtx::RecordExpr, false);
                        }
                    }
                }
                if let Some(en) = self.st.children(ch).find(|&c| self.st.kind(c).is_expr()) {
                    self.walk_expr(en);
                }
            } else if self.st.kind(ch).is_expr() {
                self.walk_expr(ch);
            }
        }
    }
}
