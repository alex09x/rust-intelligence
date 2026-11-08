/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Low-level pre-lexed token-slice path scanner and delimiter helpers for [`super::adt_tok_walk`].

use super::attr::{eval_attr_bracket_tokens, EffectiveAttrs};
use super::body_walk::FileResolver;
use super::db::{GlobalDefId, ItemKind, NsMask};
use super::path_ctx_resolve::RawSeg;
use super::path_res::{PathCtx, ResTarget};
use super::per_ns::LocalItemId;
use super::type_head_eval::StepState;
use crate::sema_fn::norm_ident;
use crate::token::{Token, TokenKind as T};
use crate::tree::NodeId;

impl<'a, 's> FileResolver<'a, 's> {
    /// Scans a pre-lexed token slice (type or const discriminant) and resolves every path inside it.
    pub(super) fn walk_tok_slice_paths(&mut self, toks: &[Token], mode: PathCtx) {
        let mut idx = 0usize;
        while let Some((ti, tok)) = next_sig(toks, idx) {
            match tok.kind {
                T::Dyn | T::Impl => idx = ti + 1,
                T::For => {
                    idx = ti + 1;
                    if let Some((li, lt)) = next_sig(toks, idx) {
                        if lt.kind == T::Lt {
                            if let Some(close_lt) = find_matching_angle(toks, li) {
                                idx = close_lt + 1;
                            }
                        }
                    }
                }
                T::Fn => {
                    idx = ti + 1;
                    if let Some((pi, pt)) = next_sig(toks, idx) {
                        if pt.kind == T::LParen {
                            if let Some(close_p) = find_matching_delim(toks, pi) {
                                self.for_each_comma_clause(&toks[pi + 1..close_p], |this, pclause| {
                                    if let Some(pidx) = this.skip_attrs_and_check_cfg(pclause) {
                                        let mut ty_start = pidx;
                                        if let Some((ni, nt)) = next_sig(pclause, pidx) {
                                            if matches!(nt.kind, T::Ident | T::RawIdent | T::Underscore) {
                                                if let Some((ci, ct)) = next_sig(pclause, ni + 1) {
                                                    if ct.kind == T::Colon && !is_colon2(pclause, ci) {
                                                        ty_start = ci + 1;
                                                    }
                                                }
                                            }
                                        }
                                        this.walk_tok_slice_paths(&pclause[ty_start..], PathCtx::Type);
                                    }
                                });
                                idx = close_p + 1;
                            }
                        }
                    }
                }
                T::LBracket => {
                    if let Some(close_b) = find_matching_delim(toks, ti) {
                        let inner = &toks[ti + 1..close_b];
                        if let Some(semi) = find_top_level_tok(inner, T::Semi) {
                            self.walk_tok_slice_paths(&inner[..semi], PathCtx::Type);
                            self.walk_tok_slice_paths(&inner[semi + 1..], PathCtx::Expr);
                        } else {
                            self.walk_tok_slice_paths(inner, mode);
                        }
                        idx = close_b + 1;
                    } else {
                        idx = ti + 1;
                    }
                }
                T::LParen | T::LBrace => {
                    if let Some(close_d) = find_matching_delim(toks, ti) {
                        self.walk_tok_slice_paths(&toks[ti + 1..close_d], mode);
                        idx = close_d + 1;
                    } else {
                        idx = ti + 1;
                    }
                }
                T::Lt if mode != PathCtx::Expr => {
                    if let Some(close_lt) = find_matching_angle(toks, ti) {
                        idx = self.scan_and_resolve_ufcs_tok_path(toks, ti, close_lt, mode);
                    } else {
                        idx = ti + 1;
                    }
                }
                T::Colon if is_colon2(toks, ti) => idx = self.scan_and_resolve_tok_path(toks, ti, true, mode),
                T::Ident | T::RawIdent | T::SelfType | T::SelfKw | T::Super | T::Crate => {
                    idx = self.scan_and_resolve_tok_path(toks, ti, false, mode);
                }
                _ => idx = ti + 1,
            }
        }
    }

    fn scan_and_resolve_ufcs_tok_path(&mut self, toks: &[Token], lt_idx: usize, close_lt: usize, mode: PathCtx) -> usize {
        let inner = &toks[lt_idx + 1..close_lt];
        let trait_def = if let Some(as_off) = find_top_level_tok(inner, T::As) {
            self.walk_tok_slice_paths(&inner[..as_off], PathCtx::Type);
            let rhs = &inner[as_off + 1..];
            let mut tr = GlobalDefId::NONE;
            if let Some((ri, rtok)) = next_sig(rhs, 0) {
                let lead = rtok.kind == T::Colon && is_colon2(rhs, ri);
                if lead || matches!(rtok.kind, T::Ident | T::RawIdent | T::SelfType | T::SelfKw | T::Super | T::Crate) {
                    let mut segs = [RawSeg::EMPTY; 16];
                    let slen = self.collect_tok_segs_for_ufcs_trait(rhs, ri, lead, &mut segs);
                    if let Some(ResTarget::Def { def, kind: ItemKind::Trait, .. }) =
                        (slen > 0).then(|| self.resolve_raw_path(NodeId(u32::MAX), lead, &segs[..slen], PathCtx::TypeBound)).flatten()
                    {
                        tr = def;
                    }
                }
            }
            tr
        } else {
            self.walk_tok_slice_paths(inner, PathCtx::Type);
            GlobalDefId::NONE
        };
        let mut idx = close_lt + 1;
        let mut segs = [RawSeg::EMPTY; 16];
        let mut slen = 0usize;
        while let Some((ci, ct)) = next_sig(toks, idx) {
            if ct.kind != T::Colon || !is_colon2(toks, ci) {
                break;
            }
            let after_c2 = colon2_after(toks, ci);
            let Some((si, stok)) = next_sig(toks, after_c2) else {
                break;
            };
            if !matches!(stok.kind, T::Ident | T::RawIdent) {
                break;
            }
            if slen < 16 {
                segs[slen] = RawSeg { start: stok.start, end: stok.end, tok_kind: stok.kind, name: norm_ident(stok.text(self.st.src)), seg_node: NodeId(u32::MAX) };
                slen += 1;
            }
            idx = si + 1;
            if let Some((li, lt)) = next_sig(toks, idx) {
                if lt.kind == T::Lt {
                    if let Some(c_ang) = find_matching_angle(toks, li) {
                        self.walk_tok_generic_args(&toks[li + 1..c_ang], GlobalDefId::NONE);
                        idx = c_ang + 1;
                    }
                }
            }
        }
        if slen > 0 && trait_def.is_some() {
            self.resolve_remaining_segments(NodeId(u32::MAX), StepState::TraitTarget(trait_def), &segs[..slen], mode);
        }
        idx
    }

    fn collect_tok_segs_for_ufcs_trait<'b>(&mut self, rhs: &'b [Token], mut idx: usize, leading_c2: bool, segs: &mut [RawSeg<'s>; 16]) -> usize {
        if leading_c2 {
            idx = colon2_after(rhs, idx);
        }
        let mut slen = 0usize;
        while let Some((si, stok)) = next_sig(rhs, idx) {
            if !matches!(stok.kind, T::Ident | T::RawIdent | T::SelfType | T::SelfKw | T::Super | T::Crate) {
                break;
            }
            if slen < 16 {
                segs[slen] = RawSeg { start: stok.start, end: stok.end, tok_kind: stok.kind, name: norm_ident(stok.text(self.st.src)), seg_node: NodeId(u32::MAX) };
                slen += 1;
            }
            idx = si + 1;
            if let Some((li, lt)) = next_sig(rhs, idx) {
                if lt.kind == T::Lt {
                    if let Some(ca) = find_matching_angle(rhs, li) {
                        self.walk_tok_generic_args(&rhs[li + 1..ca], GlobalDefId::NONE);
                        idx = ca + 1;
                    }
                }
            }
            if let Some((ci, ct)) = next_sig(rhs, idx) {
                if ct.kind == T::Colon && is_colon2(rhs, ci) {
                    idx = colon2_after(rhs, ci);
                    continue;
                }
            }
            break;
        }
        slen
    }

    fn scan_and_resolve_tok_path(&mut self, toks: &[Token], mut idx: usize, leading_colon2: bool, mode: PathCtx) -> usize {
        if leading_colon2 {
            idx = colon2_after(toks, idx);
        }
        let mut segs = [RawSeg::EMPTY; 16];
        let mut slen = 0usize;
        let mut arg_ranges: [(usize, usize); 16] = [(0, 0); 16];
        let mut alen = 0usize;

        while let Some((si, stok)) = next_sig(toks, idx) {
            if !matches!(stok.kind, T::Ident | T::RawIdent | T::SelfType | T::SelfKw | T::Super | T::Crate) {
                break;
            }
            if slen < 16 {
                segs[slen] = RawSeg { start: stok.start, end: stok.end, tok_kind: stok.kind, name: norm_ident(stok.text(self.st.src)), seg_node: NodeId(u32::MAX) };
                slen += 1;
            }
            idx = si + 1;
            let mut probe = idx;
            if let Some((ci, ct)) = next_sig(toks, probe) {
                if ct.kind == T::Colon && is_colon2(toks, ci) {
                    let after_c2 = colon2_after(toks, ci);
                    if next_sig(toks, after_c2).is_some_and(|(_, lt)| lt.kind == T::Lt) {
                        probe = after_c2;
                    }
                }
            }
            if mode != PathCtx::Expr || probe != idx {
                if let Some((li, lt)) = next_sig(toks, probe) {
                    if lt.kind == T::Lt {
                        if let Some(close_lt) = find_matching_angle(toks, li) {
                            if alen < 16 {
                                arg_ranges[alen] = (li + 1, close_lt);
                                alen += 1;
                            }
                            idx = close_lt + 1;
                        }
                    }
                }
            }
            if let Some((ci, ct)) = next_sig(toks, idx) {
                if ct.kind == T::Colon && is_colon2(toks, ci) {
                    let after_c2 = colon2_after(toks, ci);
                    if next_sig(toks, after_c2).is_some_and(|(_, nt)| matches!(nt.kind, T::Ident | T::RawIdent | T::SelfType | T::SelfKw | T::Super | T::Crate)) {
                        idx = after_c2;
                        continue;
                    }
                }
            }
            break;
        }
        let resolved = if slen > 0 { self.resolve_raw_path(NodeId(u32::MAX), leading_colon2, &segs[..slen], mode) } else { None };
        let trait_def = match resolved {
            Some(ResTarget::Def { def, kind: ItemKind::Trait, .. }) if def.is_some() => def,
            _ => GlobalDefId::NONE,
        };
        for &(alo, ahi) in &arg_ranges[..alen] {
            self.walk_tok_generic_args(&toks[alo..ahi], trait_def);
        }
        idx
    }

    fn walk_tok_generic_args(&mut self, inner: &[Token], trait_def: GlobalDefId) {
        self.for_each_comma_clause(inner, |this, clause| {
            let Some((fi, ftok)) = next_sig(clause, 0) else {
                return;
            };
            if ftok.kind == T::Lifetime {
                return;
            }
            if matches!(ftok.kind, T::Ident | T::RawIdent) {
                if let Some((ni, nt)) = next_sig(clause, fi + 1) {
                    if (nt.kind == T::Eq && !clause[ni].is_joint()) || (nt.kind == T::Colon && !is_colon2(clause, ni)) {
                        if trait_def.is_some() {
                            if let Some(assoc) = this.map.resolve_trait_assoc_cached(this.db, this.ext_cache, trait_def, norm_ident(ftok.text(this.st.src)), NsMask::TYPE) {
                                let rt = ResTarget::Def { def: assoc.def, local: LocalItemId::NONE, kind: assoc.kind };
                                this.emit_ref(ftok.start, ftok.end, NodeId(u32::MAX), rt, PathCtx::Type, false);
                            }
                        }
                        this.walk_tok_slice_paths(&clause[ni + 1..], PathCtx::Type);
                        return;
                    }
                }
            }
            this.walk_tok_slice_paths(clause, PathCtx::TypeArg);
        });
    }

    pub(super) fn skip_attrs_and_check_cfg(&self, toks: &[Token]) -> Option<usize> {
        let mut attrs = EffectiveAttrs::default();
        let mut idx = 0usize;
        while let Some((hi, htok)) = next_sig(toks, idx) {
            if htok.kind != T::Hash {
                break;
            }
            let Some((bi, btok)) = next_sig(toks, hi + 1) else {
                break;
            };
            if btok.kind != T::LBracket {
                break;
            }
            let close = find_matching_delim(toks, bi)?;
            eval_attr_bracket_tokens(self.st.src, &toks[bi + 1..close], self.db, self.map.krate, &mut attrs);
            idx = close + 1;
        }
        attrs.enabled.then_some(idx)
    }

    pub(super) fn for_each_comma_clause(&mut self, toks: &[Token], mut f: impl FnMut(&mut Self, &[Token])) {
        let (mut start, mut depth, mut i) = (0usize, 0i32, 0usize);
        while i < toks.len() {
            match toks[i].kind {
                T::Minus if toks[i].is_joint() && toks.get(i + 1).map(|t| t.kind) == Some(T::Gt) => i += 1,
                T::LParen | T::LBracket | T::LBrace | T::Lt => depth += 1,
                T::RParen | T::RBracket | T::RBrace | T::Gt => depth = (depth - 1).max(0),
                T::Comma if depth == 0 => {
                    f(self, &toks[start..i]);
                    start = i + 1;
                }
                _ => {}
            }
            i += 1;
        }
        if start < toks.len() {
            f(self, &toks[start..]);
        }
    }
}

pub(super) use super::tok_delim::{colon2_after, find_matching_angle, find_matching_delim, find_top_level_tok, is_colon2, next_sig, next_sig_matching, rfind_sig, skip_vis};
