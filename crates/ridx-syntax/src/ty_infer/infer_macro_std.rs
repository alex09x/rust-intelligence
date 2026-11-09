/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Zero-allocation stdlib macro family inference (`vec!`, `format!`, `write!`, `assert!`,
//! `assert_eq!`, `matches!`, `dbg!`, `panic!`, `env!`, `include_bytes!`, `pin!`) and call-site
//! argument inference via [`BodyInferCtx::macro_scratch`] (`Checkbox 3.6.2`).

use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::token::{Token, TokenKind as T};
use crate::tree::{NodeId, SyntaxTree};

use super::autoderef::{build_autoderef_steps_with_owner, MAX_AUTODEREF_STEPS};
use super::body_ctx::Runner;
use super::db::{StdMacroTy, TyLangItem};
use super::expectation::{BranchCoercer, Expectation};
use super::tok_ty_scan::{find_top_tok, for_each_top_comma, is_double_colon, next_sig_idx, rfind_sig_idx};
use super::ty::{InferTyId, InferTyKind};
use super::uf_table::InlineTyBuf;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum StdMacroKind {
    Vec,
    Format,
    FormatArgs,
    Write,
    Print,
    Assert,
    AssertEq,
    Matches,
    Dbg,
    Panic,
    StrConst,
    OptionEnv,
    IncludeBytes,
    Cfg,
    U32Const,
    Pin,
    Ready,
    ThreadLocal,
}

pub(super) fn classify_std_macro_path(st: &SyntaxTree<'_>, path_node: NodeId) -> Option<StdMacroKind> {
    let mut segs = [""; 4];
    let mut len = 0usize;
    for s in st.children(path_node).filter(|&c| st.kind(c) == K::PathSegment) {
        if len >= segs.len() {
            return None;
        }
        segs[len] = norm_ident(st.text(s).trim());
        len += 1;
    }
    if len == 0 {
        return None;
    }
    let tail = segs[len - 1];
    if len >= 2 && !matches!(tail, "pin" | "ready" | "thread_local") && !matches!(segs[0], "std" | "core" | "alloc") {
        return None;
    }
    match tail {
        "vec" => Some(StdMacroKind::Vec),
        "format" => Some(StdMacroKind::Format),
        "format_args" | "format_args_nl" => Some(StdMacroKind::FormatArgs),
        "write" | "writeln" => Some(StdMacroKind::Write),
        "print" | "println" | "eprint" | "eprintln" => Some(StdMacroKind::Print),
        "assert" | "debug_assert" => Some(StdMacroKind::Assert),
        t if (t.starts_with("assert_") || t.starts_with("debug_assert_")) && (t.ends_with("_eq") || t.ends_with("_ne")) => Some(StdMacroKind::AssertEq),
        "matches" => Some(StdMacroKind::Matches),
        "dbg" => Some(StdMacroKind::Dbg),
        "panic" | "todo" | "unimplemented" | "unreachable" => Some(StdMacroKind::Panic),
        "env" | "concat" | "stringify" | "include_str" | "file" | "module_path" => Some(StdMacroKind::StrConst),
        "option_env" => Some(StdMacroKind::OptionEnv),
        "include_bytes" => Some(StdMacroKind::IncludeBytes),
        "cfg" => Some(StdMacroKind::Cfg),
        "line" | "column" => Some(StdMacroKind::U32Const),
        "pin" => Some(StdMacroKind::Pin),
        "ready" => Some(StdMacroKind::Ready),
        "thread_local" => Some(StdMacroKind::ThreadLocal),
        _ => None,
    }
}

impl<'a, 's> Runner<'a, 's> {
    pub(super) fn try_infer_std_macro(&mut self, mac_node: NodeId, kind: StdMacroKind, inner_toks: &[Token], expectation: Expectation, in_stmt: bool) -> Option<InferTyId> {
        let ty = match kind {
            StdMacroKind::Vec => self.infer_std_vec_macro(inner_toks, expectation),
            StdMacroKind::Format => {
                self.infer_fmt_args_clauses(inner_toks, 0);
                self.db.std_macro_ty(StdMacroTy::String, &mut self.ctx.table.arena).unwrap_or(InferTyId::UNKNOWN)
            }
            StdMacroKind::FormatArgs => {
                self.infer_fmt_args_clauses(inner_toks, 0);
                self.db.std_macro_ty(StdMacroTy::FmtArguments, &mut self.ctx.table.arena).unwrap_or(InferTyId::UNKNOWN)
            }
            StdMacroKind::Write => self.infer_std_write_macro(mac_node, inner_toks),
            StdMacroKind::Print => {
                self.infer_fmt_args_clauses(inner_toks, 0);
                InferTyId::UNIT
            }
            StdMacroKind::Assert => {
                let mut idx = 0usize;
                for_each_top_comma(inner_toks, |cl| {
                    if idx == 0 {
                        let cty = self.infer_tok_expr_slice(cl, Expectation::HasType(InferTyId::BOOL));
                        let _ = self.coerce_expr(cty, InferTyId::BOOL);
                    } else {
                        self.infer_fmt_clause(cl, idx >= 2);
                    }
                    idx += 1;
                });
                InferTyId::UNIT
            }
            StdMacroKind::AssertEq => {
                let mut idx = 0usize;
                let mut lty = InferTyId::UNKNOWN;
                for_each_top_comma(inner_toks, |cl| {
                    if idx == 0 {
                        lty = self.infer_tok_expr_slice(cl, Expectation::None);
                    } else if idx == 1 {
                        let rty = self.infer_tok_expr_slice(cl, Expectation::None);
                        if lty != InferTyId::UNKNOWN && rty != InferTyId::UNKNOWN {
                            self.unify_assert_eq_pair(lty, rty);
                        }
                    } else {
                        let sl = self.ctx.table.shallow_resolve(lty);
                        if matches!(sl, InferTyId::F32 | InferTyId::F64) {
                            if let Some(i0) = next_sig_idx(cl, 0).filter(|&i| matches!(cl[i].kind, T::Ident | T::RawIdent)) {
                                if let Some(i1) = next_sig_idx(cl, i0 + 1).filter(|&i| cl[i].kind == T::Eq) {
                                    let _ = self.infer_tok_expr_slice(&cl[i1 + 1..], Expectation::HasType(sl));
                                    idx += 1;
                                    return;
                                }
                            }
                        }
                        self.infer_fmt_clause(cl, idx >= 3);
                    }
                    idx += 1;
                });
                InferTyId::UNIT
            }
            StdMacroKind::Matches => self.infer_std_matches_macro(inner_toks),
            StdMacroKind::Dbg => self.infer_std_dbg_macro(inner_toks, expectation),
            StdMacroKind::Panic => {
                self.infer_fmt_args_clauses(inner_toks, 0);
                InferTyId::NEVER
            }
            StdMacroKind::StrConst | StdMacroKind::OptionEnv | StdMacroKind::IncludeBytes => {
                for_each_top_comma(inner_toks, |cl| {
                    if next_sig_idx(cl, 0).is_some() {
                        let _ = self.infer_tok_expr_slice(cl, Expectation::None);
                    }
                });
                match kind {
                    StdMacroKind::StrConst => InferTyId::STR_REF,
                    StdMacroKind::OptionEnv => self.db.std_macro_ty(StdMacroTy::OptionStr, &mut self.ctx.table.arena).unwrap_or(InferTyId::UNKNOWN),
                    _ => {
                        let arr = self.ctx.table.arena.mk_array(InferTyId::U8, self.local_defs.include_bytes_len(mac_node).unwrap_or(u64::MAX));
                        self.ctx.table.arena.mk_ref(false, arr)
                    }
                }
            }
            StdMacroKind::Cfg => InferTyId::BOOL,
            StdMacroKind::U32Const => InferTyId::U32,
            StdMacroKind::Pin if in_stmt => self.infer_std_pin_stmt(inner_toks),
            StdMacroKind::Pin => {
                let val_ty = self.infer_tok_expr_slice(inner_toks, Expectation::None);
                let mref = self.ctx.table.arena.mk_ref(true, val_ty);
                self.db.lang_item(TyLangItem::Pin).map_or(InferTyId::UNKNOWN, |pd| self.ctx.table.arena.mk_adt(pd, &[mref]))
            }
            StdMacroKind::Ready => {
                let pty = self.infer_tok_expr_slice(inner_toks, Expectation::None);
                let norm_p = self.norm(pty);
                match self.ctx.table.arena.kind(norm_p) {
                    InferTyKind::Adt { args, .. } if args.len >= 1 => self.ctx.table.arena.slice(args)[0],
                    _ => InferTyId::UNKNOWN,
                }
            }
            StdMacroKind::ThreadLocal => self.infer_std_thread_local(mac_node),
        };
        Some(ty)
    }

    fn infer_std_pin_stmt(&mut self, inner_toks: &[Token]) -> InferTyId {
        for_each_top_comma(inner_toks, |cl| {
            let Some(i0) = next_sig_idx(cl, 0) else { return };
            let cl = rfind_sig_idx(cl).filter(|&i| cl[i].kind == T::Semi).map_or(cl, |semi| &cl[..semi]);
            let (id_i, expr_cl) = find_top_tok(cl, T::Eq).map_or_else(|| (rfind_sig_idx(cl).unwrap_or(i0), cl), |eq| (rfind_sig_idx(&cl[..eq]).unwrap_or(i0), &cl[eq + 1..]));
            let val_ty = self.infer_tok_expr_slice(expr_cl, Expectation::None);
            let mref = self.ctx.table.arena.mk_ref(true, val_ty);
            let pin_ty = self.db.lang_item(TyLangItem::Pin).map_or(InferTyId::UNKNOWN, |pd| self.ctx.table.arena.mk_adt(pd, &[mref]));
            let id_tok = cl[id_i];
            if matches!(id_tok.kind, T::Ident | T::RawIdent) {
                self.bind_sub_local(norm_ident(id_tok.text(self.st.src)), pin_ty);
                if let Some(crate::name_res::ResTarget::Local(lid)) = self.root_file_res.refs_at_span(id_tok.start, id_tok.end).iter().find_map(|r| match r.target {
                    t @ crate::name_res::ResTarget::Local(_) => Some(t),
                    _ => None,
                }) {
                    self.ctx.macro_local_shadows.push((lid, pin_ty));
                }
            }
        });
        InferTyId::UNIT
    }

    fn infer_std_vec_macro(&mut self, inner_toks: &[Token], expectation: Expectation) -> InferTyId {
        let exp_elem = self.expected_vec_elem_ty(expectation);
        let elem_expect = Expectation::from_option(exp_elem);
        if next_sig_idx(inner_toks, 0).is_none() {
            let ety = exp_elem.unwrap_or_else(|| self.ctx.table.new_ty_var());
            return self.db.std_macro_ty(StdMacroTy::Vec(ety), &mut self.ctx.table.arena).unwrap_or(InferTyId::UNKNOWN);
        }
        if let Some(semi) = find_top_tok(inner_toks, T::Semi) {
            let ety = self.infer_tok_expr_slice(&inner_toks[..semi], elem_expect);
            let nty = self.infer_tok_expr_slice(&inner_toks[semi + 1..], Expectation::HasType(InferTyId::USIZE));
            let _ = self.coerce_expr(nty, InferTyId::USIZE);
            return self.db.std_macro_ty(StdMacroTy::Vec(ety), &mut self.ctx.table.arena).unwrap_or(InferTyId::UNKNOWN);
        }
        let mut coercer = BranchCoercer::new(&mut self.ctx.table, elem_expect);
        let mut count = 0u32;
        for_each_top_comma(inner_toks, |cl| {
            if next_sig_idx(cl, 0).is_some() {
                let arm_exp = coercer.arm_expectation();
                let ety = self.infer_tok_expr_slice(cl, arm_exp);
                coercer.merge_arm(&mut self.ctx.table, ety);
                count += 1;
            }
        });
        let elem_ty = if count == 0 { exp_elem.unwrap_or_else(|| self.ctx.table.new_ty_var()) } else { coercer.finish(&mut self.ctx.table) };
        self.db.std_macro_ty(StdMacroTy::Vec(elem_ty), &mut self.ctx.table.arena).unwrap_or(InferTyId::UNKNOWN)
    }

    fn expected_vec_elem_ty(&mut self, expectation: Expectation) -> Option<InferTyId> {
        let exp = expectation.coercion_target(&mut self.ctx.table)?;
        let res = self.ctx.table.shallow_resolve(exp);
        if let InferTyKind::Adt { def, args } = self.ctx.table.arena.kind(res) {
            let probe = self.db.std_macro_ty(StdMacroTy::Vec(InferTyId::UNIT), &mut self.ctx.table.arena)?;
            if let InferTyKind::Adt { def: vec_def, .. } = self.ctx.table.arena.kind(probe) {
                if def == vec_def && args.len >= 1 {
                    return Some(self.ctx.table.arena.slice(args)[0]);
                }
            }
        }
        None
    }

    fn infer_std_write_macro(&mut self, mac_node: NodeId, inner_toks: &[Token]) -> InferTyId {
        let mut dst_ty = InferTyId::UNKNOWN;
        let mut idx = 0usize;
        for_each_top_comma(inner_toks, |cl| {
            if idx == 0 {
                dst_ty = self.infer_tok_expr_slice(cl, Expectation::None);
            } else {
                self.infer_fmt_clause(cl, idx >= 2);
            }
            idx += 1;
        });
        if dst_ty != InferTyId::UNKNOWN {
            let mut steps = [InferTyId::UNKNOWN; MAX_AUTODEREF_STEPS];
            let owner = self.norm_owner();
            let step_len = build_autoderef_steps_with_owner(&mut self.ctx.table, self.db, self.local_defs, self.root_st, self.root_file_res, owner, dst_ty, &mut steps);
            if let Some(matched) = self.probe_method_steps(mac_node, "write_fmt", &steps[..step_len], &[]) {
                return matched.inst_ret;
            }
        }
        self.db.std_macro_ty(StdMacroTy::FmtResult, &mut self.ctx.table.arena).unwrap_or(InferTyId::UNKNOWN)
    }

    fn infer_std_matches_macro(&mut self, inner_toks: &[Token]) -> InferTyId {
        let Some(comma) = find_top_tok(inner_toks, T::Comma) else {
            let _ = self.infer_tok_expr_slice(inner_toks, Expectation::None);
            return InferTyId::BOOL;
        };
        let scrut_ty = self.infer_tok_expr_slice(&inner_toks[..comma], Expectation::None);
        let rest = &inner_toks[comma + 1..];
        let if_pos = find_top_tok(rest, T::If);
        let pat_toks = if_pos.map_or(rest, |ip| &rest[..ip]);
        let mark = self.ctx.sub_locals_buf.len();
        self.infer_tok_pat_slice(pat_toks, scrut_ty);
        if let Some(ip) = if_pos {
            let gty = self.infer_tok_expr_slice(&rest[ip + 1..], Expectation::HasType(InferTyId::BOOL));
            let _ = self.coerce_expr(gty, InferTyId::BOOL);
        }
        self.ctx.sub_locals_buf.truncate(mark);
        InferTyId::BOOL
    }

    fn infer_std_dbg_macro(&mut self, inner_toks: &[Token], expectation: Expectation) -> InferTyId {
        if next_sig_idx(inner_toks, 0).is_none() {
            return InferTyId::UNIT;
        }
        let mut elems = InlineTyBuf::new();
        let has_comma = find_top_tok(inner_toks, T::Comma).is_some();
        for_each_top_comma(inner_toks, |cl| {
            if next_sig_idx(cl, 0).is_some() {
                let exp = if !has_comma && elems.as_slice().is_empty() { expectation } else { Expectation::None };
                elems.push(self.infer_tok_expr_slice(cl, exp));
            }
        });
        match elems.as_slice() {
            [] => InferTyId::UNIT,
            [single] => *single,
            many => self.ctx.table.arena.mk_tuple(many),
        }
    }

    fn infer_fmt_args_clauses(&mut self, inner_toks: &[Token], start_named_idx: usize) {
        let mut idx = 0usize;
        for_each_top_comma(inner_toks, |cl| {
            self.infer_fmt_clause(cl, idx > start_named_idx);
            idx += 1;
        });
    }

    fn infer_fmt_clause(&mut self, clause: &[Token], strip_named: bool) {
        let Some(i0) = next_sig_idx(clause, 0) else { return };
        let mut sub = &clause[i0..];
        if strip_named && matches!(sub[0].kind, T::Ident | T::RawIdent) {
            if let Some(i1) = next_sig_idx(sub, 1).filter(|&i| sub[i].kind == T::Eq && !sub[i].is_joint() && !is_double_colon(sub, i)) {
                sub = &sub[i1 + 1..];
            }
        }
        if next_sig_idx(sub, 0).is_some() {
            let _ = self.infer_tok_expr_slice(sub, Expectation::None);
        }
    }

    pub(super) fn infer_fallback_macro_tt(&mut self, inner: &[Token]) {
        if self.ctx.macro_depth >= super::infer_macro::MAX_RUNTIME_MACRO_DEPTH || inner.len() > 512 {
            return;
        }
        let (mut start, mut d_par, mut idx, mut islands) = (0usize, 0i32, 0usize, 0u8);
        while idx < inner.len() && islands < 24 {
            match inner[idx].kind {
                T::LParen | T::LBrace | T::LBracket => d_par += 1,
                T::RParen | T::RBrace | T::RBracket => d_par -= 1,
                T::Comma | T::Semi if d_par == 0 => {
                    self.infer_fallback_clause(&inner[start..idx]);
                    islands += 1;
                    start = idx + 1;
                }
                T::Eq if d_par == 0 && inner.get(idx + 1).is_some_and(|t| t.kind == T::Gt) => {
                    self.infer_fallback_clause(&inner[start..idx]);
                    islands += 1;
                    idx += 2;
                    start = idx;
                    continue;
                }
                _ => {}
            }
            idx += 1;
        }
        if start < inner.len() && islands < 24 {
            self.infer_fallback_clause(&inner[start..]);
        }
    }

    fn infer_fallback_clause(&mut self, clause: &[Token]) {
        let Some(mut i0) = next_sig_idx(clause, 0) else { return };
        if matches!(clause[i0].kind, T::Percent | T::Question) {
            let Some(i1) = next_sig_idx(clause, i0 + 1) else { return };
            i0 = i1;
        }
        self.infer_fmt_clause(&clause[i0..], true);
    }

    fn unify_assert_eq_pair(&mut self, lty: InferTyId, rty: InferTyId) {
        if self.ctx.table.equate(lty, rty).is_ok() {
            return;
        }
        let l_elem = self.peel_slice_or_array_elem(lty);
        let r_elem = self.peel_slice_or_array_elem(rty);
        if let (Some(le), Some(re)) = (l_elem, r_elem) {
            let _ = self.ctx.table.equate(le, re);
            return;
        }
        let _ = self.coerce_expr(rty, lty);
        let lres = self.ctx.table.shallow_resolve(lty);
        if self.ctx.table.arena.flags(lres).has_infer() {
            let _ = self.coerce_expr(lty, rty);
        }
    }

    fn peel_slice_or_array_elem(&mut self, ty: InferTyId) -> Option<InferTyId> {
        let mut steps = [InferTyId::UNKNOWN; MAX_AUTODEREF_STEPS];
        let owner = self.norm_owner();
        let n = build_autoderef_steps_with_owner(&mut self.ctx.table, self.db, self.local_defs, self.root_st, self.root_file_res, owner, ty, &mut steps);
        steps[..n].iter().find_map(|&st| match self.ctx.table.arena.kind(st) {
            InferTyKind::Slice(e) | InferTyKind::Array { elem: e, .. } => Some(e),
            _ => None,
        })
    }
}
