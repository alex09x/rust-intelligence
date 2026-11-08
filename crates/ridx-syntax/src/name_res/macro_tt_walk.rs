/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Best-effort lexical, path, and Rust-2021 format-capture token walker inside
//! `K::MacroCall > K::TokenTree` arguments (`vec![x]`, `assert_eq!(a, b)`, `println!("{x}")`,
//! `tri!(Foo::bar(x))`, `assert_impl_all!(Ty: Trait)`).

use super::body_walk::FileResolver;
use super::db::ItemKind;
use super::path_ctx_resolve::RawSeg;
use super::path_res::{PathCtx, ResTarget};
use super::tok_slice_scan::{colon2_after, find_matching_angle, find_matching_delim, is_colon2, next_sig, rfind_sig};
use super::type_head_eval::binding_to_res_target;
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::token::{Token, TokenKind as T};
use crate::tree::NodeId;

impl<'a, 's> FileResolver<'a, 's> {
    /// Resolves `K::MacroCall`'s callee path and walks call-site argument tokens inside its `K::TokenTree`.
    pub(super) fn walk_macro_call_node(&mut self, mac_node: NodeId, in_expr_or_body: bool) {
        let mut is_include_like = false;
        if let Some(p) = self.st.child(mac_node, K::Path) {
            if let Some((_, sname)) = self.single_bare_segment_info(p) {
                is_include_like = matches!(sname, "include" | "include_str" | "include_bytes" | "env" | "option_env");
            }
            self.resolve_cst_path(p, PathCtx::MacroCall);
        }
        if is_include_like {
            return;
        }
        let Some(tt) = self.st.child(mac_node, K::TokenTree) else {
            return;
        };
        let toks = self.st.tokens_of(tt);
        let Some((open_i, open_t)) = next_sig(toks, 0) else {
            return;
        };
        let Some(close_i) = rfind_sig(toks) else {
            return;
        };
        if open_i >= close_i || !matches!(open_t.kind, T::LParen | T::LBracket | T::LBrace) {
            return;
        }
        self.walk_macro_tt_slice(&toks[open_i + 1..close_i], open_t.kind == T::LBrace, in_expr_or_body);
    }

    fn walk_macro_tt_slice(&mut self, toks: &[Token], in_braces: bool, in_body: bool) {
        let mut idx = 0usize;
        let mut prev_sig: Option<T> = None;
        while let Some((ti, tok)) = next_sig(toks, idx) {
            match tok.kind {
                T::Dollar => {
                    idx = ti + 1;
                    if let Some((ni, nt)) = next_sig(toks, idx) {
                        if matches!(nt.kind, T::Ident | T::RawIdent | T::Crate) {
                            idx = ni + 1;
                        }
                    }
                    prev_sig = Some(T::Dollar);
                }
                T::LParen | T::LBracket | T::LBrace => {
                    if let Some(close_d) = find_matching_delim(toks, ti) {
                        self.walk_macro_tt_slice(&toks[ti + 1..close_d], tok.kind == T::LBrace, in_body);
                        idx = close_d + 1;
                        prev_sig = Some(toks[close_d].kind);
                    } else {
                        idx = ti + 1;
                        prev_sig = Some(tok.kind);
                    }
                }
                T::Str | T::RawStr if in_body => {
                    self.scan_format_str_captures(tok);
                    idx = ti + 1;
                    prev_sig = Some(tok.kind);
                }
                T::Colon if is_colon2(toks, ti) && prev_sig != Some(T::Dot) => {
                    idx = self.scan_macro_path(toks, ti, true, in_body);
                    prev_sig = Some(T::Ident);
                }
                T::Ident | T::RawIdent | T::SelfType | T::SelfKw | T::Super | T::Crate => {
                    if prev_sig == Some(T::Dot) {
                        idx = ti + 1;
                        prev_sig = Some(tok.kind);
                        continue;
                    }
                    if in_braces && matches!(tok.kind, T::Ident | T::RawIdent) {
                        if let Some((ci, ct)) = next_sig(toks, ti + 1) {
                            if ct.kind == T::Colon && !is_colon2(toks, ci) {
                                idx = ci + 1;
                                prev_sig = Some(T::Colon);
                                continue;
                            }
                        }
                    }
                    idx = self.scan_macro_path(toks, ti, false, in_body);
                    prev_sig = Some(T::Ident);
                }
                _ => {
                    idx = ti + 1;
                    prev_sig = Some(tok.kind);
                }
            }
        }
    }

    fn scan_macro_path(&mut self, toks: &[Token], mut idx: usize, leading_colon2: bool, in_body: bool) -> usize {
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
            if let Some((ci, ct)) = next_sig(toks, idx) {
                if ct.kind == T::Colon && is_colon2(toks, ci) {
                    let after_c2 = colon2_after(toks, ci);
                    if let Some((li, lt)) = next_sig(toks, after_c2) {
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
                } else if ct.kind == T::Lt && !in_body {
                    if let Some(close_lt) = find_matching_angle(toks, ci) {
                        if alen < 16 {
                            arg_ranges[alen] = (ci + 1, close_lt);
                            alen += 1;
                        }
                        idx = close_lt + 1;
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

        if slen == 1 && !leading_colon2 {
            let s0 = segs[0];
            let followed_by_bang = next_sig(toks, idx).is_some_and(|(_, nt)| nt.kind == T::Bang);
            if followed_by_bang {
                self.resolve_raw_path(NodeId(u32::MAX), false, &segs[..1], PathCtx::MacroCall);
            } else {
                self.resolve_macro_bare_ident(s0, in_body);
            }
        } else if slen > 0 {
            let followed_by_bang = next_sig(toks, idx).is_some_and(|(_, nt)| nt.kind == T::Bang);
            let ctx = if followed_by_bang {
                PathCtx::MacroCall
            } else if in_body {
                PathCtx::Expr
            } else {
                PathCtx::Type
            };
            if self.resolve_raw_path(NodeId(u32::MAX), leading_colon2, &segs[..slen], ctx).is_none() && in_body && !followed_by_bang {
                self.resolve_raw_path(NodeId(u32::MAX), leading_colon2, &segs[..slen], PathCtx::Type);
            }
        }

        for &(as_, ae) in &arg_ranges[..alen] {
            self.walk_tok_slice_paths(&toks[as_..ae], PathCtx::Type);
        }
        idx
    }

    fn resolve_macro_bare_ident(&mut self, s0: RawSeg<'_>, in_body: bool) {
        match s0.tok_kind {
            T::SelfKw => {
                if in_body {
                    if let Some(lid) = self.ribs.lookup_local(self.st.src, "self") {
                        self.emit_ref(s0.start, s0.end, NodeId(u32::MAX), ResTarget::Local(lid), PathCtx::Expr, true);
                    }
                }
                return;
            }
            T::SelfType => {
                if let Some(sc) = self.ribs.self_ctx {
                    self.emit_ref(s0.start, s0.end, NodeId(u32::MAX), ResTarget::SelfTy { target: sc.target, trait_def: sc.trait_def }, PathCtx::Type, true);
                }
                return;
            }
            T::Crate | T::Super => return,
            _ => {}
        }
        if in_body {
            if let Some(lid) = self.ribs.lookup_local(self.st.src, s0.name) {
                self.emit_ref(s0.start, s0.end, NodeId(u32::MAX), ResTarget::Local(lid), PathCtx::Expr, true);
                return;
            }
        }
        if let Some((gid, _)) = self.ribs.lookup_generic(self.st.src, s0.name) {
            self.emit_ref(s0.start, s0.end, NodeId(u32::MAX), ResTarget::Generic(gid), PathCtx::Type, true);
            return;
        }
        let pns = self.lookup_bare_all_ns(s0.name);
        let chosen = if in_body { pns.values.or(pns.types) } else { pns.types.or(pns.values) };
        if let Some(b) = chosen {
            if b.kind == ItemKind::Mod {
                return;
            }
            if let Some(rt) = binding_to_res_target(b) {
                self.emit_ref(s0.start, s0.end, NodeId(u32::MAX), rt, if in_body { PathCtx::Expr } else { PathCtx::Type }, true);
            }
        }
    }

    /// Resolves Rust 2021 implicit format-string captures (`"{ident}"`, `"{ident:?}"`) inside string literals.
    fn scan_format_str_captures(&mut self, tok: Token) {
        let raw = tok.text(self.st.src);
        let bytes = raw.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() {
            if bytes[i] == b'{' {
                if i + 1 < bytes.len() && bytes[i + 1] == b'{' {
                    i += 2;
                    continue;
                }
                let id_start = i + 1;
                let mut j = id_start;
                if j + 2 <= bytes.len() && &bytes[j..j + 2] == b"r#" {
                    j += 2;
                }
                let ident_head = j;
                if j < bytes.len() && (bytes[j].is_ascii_alphabetic() || bytes[j] == b'_') {
                    j += 1;
                    while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                        j += 1;
                    }
                    if j < bytes.len() && matches!(bytes[j], b'}' | b':') {
                        let ident = &raw[ident_head..j];
                        if ident != "_" {
                            let bs = tok.start + id_start as u32;
                            let be = tok.start + j as u32;
                            if let Some(lid) = self.ribs.lookup_local(self.st.src, ident) {
                                self.emit_ref(bs, be, NodeId(u32::MAX), ResTarget::Local(lid), PathCtx::Expr, true);
                            } else if let Some(vb) = self.lookup_bare_all_ns(ident).values {
                                if let Some(rt) = binding_to_res_target(vb) {
                                    self.emit_ref(bs, be, NodeId(u32::MAX), rt, PathCtx::Expr, true);
                                }
                            }
                        }
                    }
                }
                i = j + 1;
            } else if bytes[i] == b'}' && i + 1 < bytes.len() && bytes[i + 1] == b'}' {
                i += 2;
            } else {
                i += 1;
            }
        }
    }
}
