/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Recursive token-slice `use`-tree flattener (`{self, *, as _, #[cfg(...)]}`) into [`UseArena`].

use super::attr::{eval_attr_bracket_tokens, eval_outer_attrs, extract_visibility, EffectiveAttrs};
use super::db::{CrateId, ItemDb, ModuleId, VisKind};
use super::mod_tree::ModuleTree;
use super::per_ns::compute_vis_mod;
use super::use_tree::{FlatUse, FlatUseKind, PathAnchor, UseArena};
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::token::{Token, TokenKind as T};
use crate::tree::{NodeId, SyntaxTree};

struct LowerCtx<'s, 'd> {
    src: &'s str,
    db: &'d dyn ItemDb,
    krate: CrateId,
    module: ModuleId,
    vis: VisKind,
    vis_mod: ModuleId,
}

impl UseArena {
    /// Evaluates attributes + visibility and flattens a `K::Use` node into [`Self::uses`].
    pub fn lower_use(&mut self, st: &SyntaxTree<'_>, node: NodeId, mod_tree: &ModuleTree, module: ModuleId, db: &dyn ItemDb, krate: CrateId) {
        let mut scratch_toks = Vec::new();
        let mut scratch_prefix = Vec::new();
        self.lower_use_into(st, node, mod_tree, module, db, krate, &mut scratch_toks, &mut scratch_prefix);
    }

    /// Zero-alloc variant reusing caller-owned scratch buffers across all `use` items in a crate.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn lower_use_into(
        &mut self,
        st: &SyntaxTree<'_>,
        node: NodeId,
        mod_tree: &ModuleTree,
        module: ModuleId,
        db: &dyn ItemDb,
        krate: CrateId,
        scratch_toks: &mut Vec<Token>,
        scratch_prefix: &mut Vec<(u32, u32)>,
    ) {
        let attrs = eval_outer_attrs(st, node, db, krate);
        if !attrs.enabled {
            return;
        }
        let (raw_vis, in_path) = extract_visibility(st, node);
        let (vis, vis_mod) = compute_vis_mod(mod_tree, module, raw_vis, in_path);
        let body = body_tokens_after_headers(st, node);
        let Some(use_pos) = body.iter().position(|t| t.kind == T::Use) else { return };
        scratch_toks.clear();
        scratch_toks.extend(body[use_pos + 1..].iter().copied().filter(|t| !t.is_trivia() && t.kind != T::Semi));
        if scratch_toks.is_empty() {
            return;
        }
        scratch_prefix.clear();
        let ctx = LowerCtx { src: st.src, db, krate, module, vis, vis_mod };
        self.parse_subtree(&ctx, scratch_toks, None, scratch_prefix);
    }

    fn parse_subtree(&mut self, ctx: &LowerCtx<'_, '_>, mut toks: &[Token], inherited_anchor: Option<PathAnchor>, prefix: &mut Vec<(u32, u32)>) {
        let mut branch_attrs = EffectiveAttrs::default();
        while toks.first().map(|t| t.kind) == Some(T::Hash) && toks.get(1).map(|t| t.kind) == Some(T::LBracket) {
            let Some(close) = matching_delim(toks, 1, T::LBracket, T::RBracket) else { return };
            eval_attr_bracket_tokens(ctx.src, &toks[2..close], ctx.db, ctx.krate, &mut branch_attrs);
            toks = &toks[close + 1..];
        }
        if !branch_attrs.enabled || toks.is_empty() {
            return;
        }

        let span_start = toks.first().map_or(0, |t| t.start);
        let span_end = toks.last().map_or(span_start, |t| t.end);
        let base_prefix_len = prefix.len();
        let mut i = 0usize;
        let mut anchor = inherited_anchor.unwrap_or(PathAnchor::Bare);

        if inherited_anchor.is_none() {
            if is_colon_colon(toks, 0) {
                anchor = PathAnchor::ExternRoot;
                i = 2;
            } else if toks.first().map(|t| t.kind) == Some(T::Crate) {
                anchor = PathAnchor::CrateRoot;
                i = 1;
                if is_colon_colon(toks, i) {
                    i += 2;
                }
            } else if toks.first().map(|t| t.kind) == Some(T::SelfKw) {
                anchor = PathAnchor::SelfMod;
                i = 1;
                if is_colon_colon(toks, i) {
                    i += 2;
                }
            } else if toks.first().map(|t| t.kind) == Some(T::Super) {
                let mut sup = 0u16;
                while toks.get(i).map(|t| t.kind) == Some(T::Super) {
                    sup += 1;
                    i += 1;
                    if is_colon_colon(toks, i) {
                        i += 2;
                    } else {
                        break;
                    }
                }
                anchor = PathAnchor::Super(sup);
            }
        } else if toks.first().map(|t| t.kind) == Some(T::SelfKw) {
            i = 1;
            if is_colon_colon(toks, i) {
                i += 2;
            } else {
                self.emit_terminal_leaf(ctx, anchor, prefix, &toks[i..], span_start, span_end);
                prefix.truncate(base_prefix_len);
                return;
            }
        } else if toks.first().map(|t| t.kind) == Some(T::Super) {
            while toks.get(i).map(|t| t.kind) == Some(T::Super) {
                if prefix.pop().is_none() {
                    anchor = match anchor {
                        PathAnchor::SelfMod | PathAnchor::Bare => PathAnchor::Super(1),
                        PathAnchor::Super(n) => PathAnchor::Super(n + 1),
                        other => other,
                    };
                }
                i += 1;
                if is_colon_colon(toks, i) {
                    i += 2;
                } else {
                    break;
                }
            }
        }

        loop {
            if i >= toks.len() {
                self.emit_terminal_leaf(ctx, anchor, prefix, &[], span_start, span_end);
                break;
            }
            match toks[i].kind {
                T::Star => {
                    self.emit_leaf(ctx, anchor, prefix, "", FlatUseKind::Glob, span_start, span_end);
                    break;
                }
                T::LBrace => {
                    let close = matching_delim(toks, i, T::LBrace, T::RBrace).unwrap_or(toks.len().saturating_sub(1));
                    let inner = if close > i { &toks[i + 1..close] } else { &toks[i + 1..] };
                    let mut depth = 0u32;
                    let mut start = 0usize;
                    for (bi, bt) in inner.iter().enumerate() {
                        match bt.kind {
                            T::LBrace | T::LParen | T::LBracket => depth += 1,
                            T::RBrace | T::RParen | T::RBracket => depth = depth.saturating_sub(1),
                            T::Comma if depth == 0 => {
                                if start < bi {
                                    self.parse_subtree(ctx, &inner[start..bi], Some(anchor), prefix);
                                }
                                start = bi + 1;
                            }
                            _ => {}
                        }
                    }
                    if start < inner.len() {
                        self.parse_subtree(ctx, &inner[start..], Some(anchor), prefix);
                    }
                    break;
                }
                T::As => {
                    self.emit_terminal_leaf(ctx, anchor, prefix, &toks[i..], span_start, span_end);
                    break;
                }
                tk if matches!(tk, T::Ident | T::RawIdent) || tk.is_keyword() => {
                    let tok = toks[i];
                    let raw = tok.text(ctx.src);
                    let off = if raw.starts_with("r#") { 2u32 } else { 0u32 };
                    prefix.push((tok.start + off, tok.end));
                    i += 1;
                    if is_colon_colon(toks, i) {
                        i += 2;
                    } else {
                        self.emit_terminal_leaf(ctx, anchor, prefix, &toks[i..], span_start, span_end);
                        break;
                    }
                }
                _ => break,
            }
        }
        prefix.truncate(base_prefix_len);
    }

    fn emit_terminal_leaf(&mut self, ctx: &LowerCtx<'_, '_>, anchor: PathAnchor, prefix: &[(u32, u32)], tail: &[Token], span_start: u32, span_end: u32) {
        let (kind, bind_opt) = parse_alias_tail(ctx.src, tail);
        let bind = match bind_opt.or_else(|| prefix.last().map(|&(s, e)| &ctx.src[s as usize..e as usize])) {
            Some(b) => b,
            None => return,
        };
        self.emit_leaf(ctx, anchor, prefix, bind, kind, span_start, span_end);
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_leaf(&mut self, ctx: &LowerCtx<'_, '_>, anchor: PathAnchor, prefix: &[(u32, u32)], bind: &str, kind: FlatUseKind, span_start: u32, span_end: u32) {
        let seg_start = self.segs.len() as u32;
        for &(s, e) in prefix {
            let sp = self.push_str(&ctx.src[s as usize..e as usize]);
            self.segs.push(sp);
        }
        let seg_len = (self.segs.len() as u32 - seg_start) as u16;
        let (bind_start, bind_end) = self.push_str(bind);
        self.uses.push(FlatUse { module: ctx.module, anchor, seg_start, seg_len, bind_start, bind_end, vis: ctx.vis, vis_mod: ctx.vis_mod, kind, span_start, span_end });
    }
}

pub(super) fn body_tokens_after_headers<'a>(st: &'a SyntaxTree<'_>, item: NodeId) -> &'a [Token] {
    let mut first_tok = st.node(item).first;
    for child in st.children(item) {
        if matches!(st.kind(child), K::Attr | K::Visibility) {
            first_tok = st.node(child).end.max(first_tok);
        } else {
            break;
        }
    }
    let end_tok = st.node(item).end.max(first_tok);
    &st.tokens[first_tok as usize..end_tok as usize]
}

#[inline]
fn is_colon_colon(toks: &[Token], idx: usize) -> bool {
    toks.get(idx).map(|t| t.kind) == Some(T::Colon) && toks.get(idx + 1).map(|t| t.kind) == Some(T::Colon)
}

fn parse_alias_tail<'s>(src: &'s str, tail: &[Token]) -> (FlatUseKind, Option<&'s str>) {
    if tail.first().map(|t| t.kind) == Some(T::As) {
        if let Some(t) = tail.get(1) {
            if t.kind == T::Underscore {
                return (FlatUseKind::Underscore, Some("_"));
            }
            if matches!(t.kind, T::Ident | T::RawIdent) {
                return (FlatUseKind::Named, Some(norm_ident(t.text(src))));
            }
        }
    }
    (FlatUseKind::Named, None)
}

fn matching_delim(toks: &[Token], open_idx: usize, open_k: T, close_k: T) -> Option<usize> {
    let mut depth = 0u32;
    for (off, t) in toks[open_idx..].iter().enumerate() {
        if t.kind == open_k {
            depth += 1;
        } else if t.kind == close_k {
            depth -= 1;
            if depth == 0 {
                return Some(open_idx + off);
            }
        }
    }
    None
}
