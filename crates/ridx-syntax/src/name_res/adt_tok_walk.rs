/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Zero-allocation token-slice path walker for `TokenTree`-backed CST item regions:
//! `struct` / `union` / `enum` field & variant lists, `fn(...)` pointer parameter lists,
//! and nested `use` trees.

use super::attr::eval_outer_attrs;
use super::body_walk::FileResolver;
use super::path_ctx_resolve::RawSeg;
use super::path_res::PathCtx;
use super::tok_slice_scan::{colon2_after, find_matching_delim, is_colon2, next_sig, next_sig_matching, rfind_sig, skip_vis};
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::token::{Token, TokenKind as T};
use crate::tree::NodeId;

impl<'a, 's> FileResolver<'a, 's> {
    /// Resolves field types inside a `K::Struct` or `K::Union` `TokenTree` (`{ a: Ty }` or `(Ty1, Ty2)`).
    pub(super) fn walk_struct_or_union_tt(&mut self, tt_node: NodeId) {
        let toks = self.st.tokens_of(tt_node);
        let Some((open_idx, open_tok)) = next_sig(toks, 0) else {
            return;
        };
        let Some(close_idx) = rfind_sig(toks) else {
            return;
        };
        if open_idx >= close_idx {
            return;
        }
        let inner = &toks[open_idx + 1..close_idx];
        match open_tok.kind {
            T::LBrace => self.walk_record_field_defs(inner),
            T::LParen => self.walk_tuple_field_defs(inner),
            _ => {}
        }
    }

    /// Resolves variant field types and discriminant expressions inside a `K::Enum` `TokenTree`.
    pub(super) fn walk_enum_tt(&mut self, tt_node: NodeId) {
        let toks = self.st.tokens_of(tt_node);
        let Some((open_idx, open_tok)) = next_sig(toks, 0) else {
            return;
        };
        let Some(close_idx) = rfind_sig(toks) else {
            return;
        };
        if open_tok.kind != T::LBrace || open_idx >= close_idx {
            return;
        }
        let inner = &toks[open_idx + 1..close_idx];
        self.for_each_comma_clause(inner, |this, clause| {
            let Some(mut idx) = this.skip_attrs_and_check_cfg(clause) else {
                return;
            };
            let Some((ni, ntok)) = next_sig(clause, idx) else {
                return;
            };
            if !matches!(ntok.kind, T::Ident | T::RawIdent) {
                return;
            }
            idx = ni + 1;
            if let Some((pi, ptok)) = next_sig(clause, idx) {
                if matches!(ptok.kind, T::LParen | T::LBrace) {
                    if let Some(end_p) = find_matching_delim(clause, pi) {
                        let body = &clause[pi + 1..end_p];
                        if ptok.kind == T::LParen {
                            this.walk_tuple_field_defs(body);
                        } else {
                            this.walk_record_field_defs(body);
                        }
                        idx = end_p + 1;
                    }
                }
            }
            if let Some((eq_i, eq_t)) = next_sig(clause, idx) {
                if eq_t.kind == T::Eq {
                    this.walk_tok_slice_paths(&clause[eq_i + 1..], PathCtx::Expr);
                }
            }
        });
    }

    /// Resolves parameter types inside a `K::FnPtrType` `( ... )` `TokenTree`.
    pub(super) fn walk_fn_ptr_tt(&mut self, tt_node: NodeId) {
        let toks = self.st.tokens_of(tt_node);
        let Some((open_idx, open_tok)) = next_sig(toks, 0) else {
            return;
        };
        let Some(close_idx) = rfind_sig(toks) else {
            return;
        };
        if open_tok.kind != T::LParen || open_idx >= close_idx {
            return;
        }
        let inner = &toks[open_idx + 1..close_idx];
        self.for_each_comma_clause(inner, |this, clause| {
            let Some(mut idx) = this.skip_attrs_and_check_cfg(clause) else {
                return;
            };
            let mut probe = idx;
            if let Some((mi, mt)) = next_sig(clause, probe) {
                if mt.kind == T::Mut {
                    probe = mi + 1;
                }
            }
            if let Some((ni, nt)) = next_sig(clause, probe) {
                if matches!(nt.kind, T::Ident | T::RawIdent | T::Underscore) {
                    if let Some((ci, ct)) = next_sig(clause, ni + 1) {
                        if ct.kind == T::Colon && !is_colon2(clause, ci) {
                            idx = ci + 1;
                        }
                    }
                }
            }
            this.walk_tok_slice_paths(&clause[idx..], PathCtx::Type);
        });
    }

    /// Resolves segments inside a `K::Use` statement (`use a::b::{c, d as e, self};`).
    pub(super) fn walk_use_item(&mut self, use_node: NodeId) {
        if !eval_outer_attrs(self.st, use_node, self.db, self.map.krate).enabled {
            return;
        }
        let start_tok = self
            .st
            .children(use_node)
            .filter(|&c| matches!(self.st.kind(c), K::Attr | K::Visibility))
            .last()
            .map_or(self.st.node(use_node).first as usize, |c| self.st.node(c).end as usize);
        let end_tok = (self.st.node(use_node).end as usize).max(start_tok);
        let toks = &self.st.tokens[start_tok..end_tok];
        let Some((u_idx, _)) = next_sig_matching(toks, 0, T::Use) else {
            return;
        };
        let mut prefix = [RawSeg::EMPTY; 16];
        self.walk_use_subtree(&toks[u_idx + 1..], false, &mut prefix, 0);
    }

    fn walk_use_subtree(&mut self, toks: &[Token], mut leading_colon2: bool, prefix: &mut [RawSeg<'s>; 16], mut plen: usize) {
        let Some(mut idx) = self.skip_attrs_and_check_cfg(toks) else {
            return;
        };
        if plen == 0 {
            if let Some((ci, ct)) = next_sig(toks, idx) {
                if ct.kind == T::Colon && is_colon2(toks, ci) {
                    leading_colon2 = true;
                    idx = colon2_after(toks, ci);
                }
            }
        }
        while let Some((ti, tok)) = next_sig(toks, idx) {
            match tok.kind {
                T::Semi | T::Star => return,
                T::LBrace => {
                    if let Some(close_b) = find_matching_delim(toks, ti) {
                        let inner = &toks[ti + 1..close_b];
                        let (saved_len, saved_c2) = (plen, leading_colon2);
                        let (mut start, mut depth) = (0usize, 0u32);
                        for (off, bt) in inner.iter().enumerate() {
                            match bt.kind {
                                T::LBrace | T::LParen | T::LBracket => depth += 1,
                                T::RBrace | T::RParen | T::RBracket => depth = depth.saturating_sub(1),
                                T::Comma if depth == 0 => {
                                    self.walk_use_subtree(&inner[start..off], saved_c2, prefix, saved_len);
                                    start = off + 1;
                                }
                                _ => {}
                            }
                        }
                        if start < inner.len() {
                            self.walk_use_subtree(&inner[start..], saved_c2, prefix, saved_len);
                        }
                    }
                    return;
                }
                T::Ident | T::RawIdent | T::Crate | T::SelfKw | T::Super => {
                    if tok.kind == T::SelfKw && plen > 0 && next_sig(toks, ti + 1).is_none_or(|(_, nt)| matches!(nt.kind, T::As | T::Semi | T::Comma)) {
                        let last_idx = plen - 1;
                        let saved = prefix[last_idx];
                        prefix[last_idx] = RawSeg { start: tok.start, end: tok.end, ..saved };
                        self.resolve_raw_path(NodeId(u32::MAX), leading_colon2, &prefix[..plen], PathCtx::UseTree);
                        prefix[last_idx] = saved;
                        return;
                    }
                    if plen < 16 {
                        prefix[plen] = RawSeg { start: tok.start, end: tok.end, tok_kind: tok.kind, name: norm_ident(tok.text(self.st.src)), seg_node: NodeId(u32::MAX) };
                        plen += 1;
                    }
                    idx = ti + 1;
                    if let Some((ni, ntok)) = next_sig(toks, idx) {
                        if ntok.kind == T::Colon && is_colon2(toks, ni) {
                            idx = colon2_after(toks, ni);
                            continue;
                        }
                        if ntok.kind == T::As {
                            if plen > 0 {
                                self.resolve_raw_path(NodeId(u32::MAX), leading_colon2, &prefix[..plen], PathCtx::UseTree);
                            }
                            return;
                        }
                    }
                    if plen > 0 {
                        self.resolve_raw_path(NodeId(u32::MAX), leading_colon2, &prefix[..plen], PathCtx::UseTree);
                    }
                    return;
                }
                _ => return,
            }
        }
    }

    pub(super) fn walk_record_field_defs(&mut self, inner: &[Token]) {
        self.for_each_comma_clause(inner, |this, clause| {
            let Some(mut idx) = this.skip_attrs_and_check_cfg(clause) else {
                return;
            };
            idx = skip_vis(clause, idx);
            let mut depth = 0u32;
            for i in idx..clause.len() {
                match clause[i].kind {
                    T::LParen | T::LBracket | T::LBrace => depth += 1,
                    T::RParen | T::RBracket | T::RBrace => depth = depth.saturating_sub(1),
                    T::Colon if depth == 0 && !is_colon2(clause, i) => {
                        this.walk_tok_slice_paths(&clause[i + 1..], PathCtx::Type);
                        return;
                    }
                    _ => {}
                }
            }
        });
    }

    pub(super) fn walk_tuple_field_defs(&mut self, inner: &[Token]) {
        self.for_each_comma_clause(inner, |this, clause| {
            let Some(mut idx) = this.skip_attrs_and_check_cfg(clause) else {
                return;
            };
            idx = skip_vis(clause, idx);
            this.walk_tok_slice_paths(&clause[idx..], PathCtx::Type);
        });
    }
}
