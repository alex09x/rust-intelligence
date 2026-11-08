/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! CST reference-site classifier separating Phase 2 direct CST paths, `MacroCall > TokenTree`
//! call-site arguments (Checkbox `3.6`), dot accesses (Phase 3), and proc-macro / `#[derive]` /
//! declaration-site synthetic spans.

use std::collections::HashSet;

use memmap2::Mmap;
use ridx_format::{DefKind, IndexReader, RefEntry};
use ridx_syntax::{NodeId, SyntaxKind as K, SyntaxTree, TokenKind as T};

use crate::meta::CorpusFile;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SiteKind {
    DirectCst,
    MacroArg,
    Synth,
}

pub fn collect_as_aliases(src: &str, st: &SyntaxTree<'_>, out: &mut HashSet<String>) {
    let mut saw_as = false;
    for tok in &st.tokens {
        if tok.is_trivia() {
            continue;
        }
        if saw_as {
            if matches!(tok.kind, T::Ident | T::RawIdent) {
                let t = tok.text(src);
                out.insert(t.strip_prefix("r#").unwrap_or(t).to_string());
            }
            saw_as = false;
        } else if tok.kind == T::As {
            saw_as = true;
        }
    }
}

pub fn collect_dot_access_spans(st: &SyntaxTree<'_>) -> HashSet<(u32, u32)> {
    let mut set = HashSet::new();
    for i in 0..st.nodes.len() {
        let nid = NodeId(i as u32);
        if matches!(st.kind(nid), K::MethodCallExpr | K::FieldExpr) {
            if let Some(nn) = st.child(nid, K::Name) {
                let r = st.range(nn);
                set.insert((r.start as u32, r.end as u32));
            }
        }
    }
    let mut prev1 = None::<ridx_syntax::Token>;
    let mut prev2 = None::<ridx_syntax::Token>;
    for &tok in &st.tokens {
        if tok.is_trivia() {
            continue;
        }
        if let Some(p1) = prev1 {
            let is_single_dot = p1.kind == T::Dot && !prev2.is_some_and(|p2| p2.kind == T::Dot && p2.end == p1.start);
            if is_single_dot && matches!(tok.kind, T::Ident | T::RawIdent | T::Int) {
                set.insert((tok.start, tok.end));
            }
        }
        prev2 = prev1;
        prev1 = Some(tok);
    }
    set
}

pub fn classify_ref_site(
    reader: &IndexReader<Mmap>,
    cf: &CorpusFile,
    st: &SyntaxTree<'_>,
    unit_aliases: &HashSet<String>,
    r: &RefEntry,
    sb: u32,
    eb: u32,
) -> (SiteKind, (u32, u32), String) {
    let tidx = st.tokens.partition_point(|t| t.end <= sb);
    let Some(&tok) = st.tokens.get(tidx) else {
        return (SiteKind::Synth, (sb, eb), String::new());
    };
    let exact_tok = (tok.start == sb && tok.end == eb) || (tok.kind == T::RawIdent && tok.start + 2 == sb && tok.end == eb);
    if !exact_tok || !matches!(tok.kind, T::Ident | T::RawIdent | T::SelfKw | T::SelfType | T::Super | T::Crate) {
        return (SiteKind::Synth, (sb, eb), String::new());
    }

    let dkind = DefKind::from(r.kind);
    if dkind != DefKind::Local {
        let raw_txt = tok.text(&cf.text);
        let src_ident = raw_txt.strip_prefix("r#").unwrap_or(raw_txt);
        if !matches!(src_ident, "self" | "Self" | "crate" | "super") {
            let dp = reader.get_str(r.target_path_offset());
            let drv_leaf_raw = dp.rsplit("::").next().unwrap_or(dp);
            let drv_leaf = drv_leaf_raw.strip_prefix("r#").unwrap_or(drv_leaf_raw);
            if src_ident != drv_leaf && !unit_aliases.contains(src_ident) {
                return (SiteKind::Synth, (tok.start, tok.end), String::new());
            }
        }
    }

    let mut best_nid = st.root();
    let mut best_len = u32::MAX;
    for i in 0..st.nodes.len() {
        let nid = NodeId(i as u32);
        let nr = st.range(nid);
        if (nr.start as u32) <= tok.start && tok.end <= (nr.end as u32) {
            let l = (nr.end - nr.start) as u32;
            if l <= best_len {
                best_len = l;
                best_nid = nid;
            }
        }
    }

    let mut cur = Some(best_nid);
    let mut in_macro_tt = false;
    while let Some(nid) = cur {
        let k = st.kind(nid);
        if matches!(k, K::Attr | K::MacroRules | K::Name) || (k == K::SelfParam && tok.kind == T::SelfKw) {
            return (SiteKind::Synth, (tok.start, tok.end), String::new());
        }
        if k == K::TokenTree {
            if let Some(pk) = st.parent(nid).map(|p| st.kind(p)) {
                if pk == K::MacroCall {
                    in_macro_tt = true;
                } else if matches!(pk, K::Struct | K::Union | K::Enum) && is_adt_tt_attr_or_decl_tok(st.tokens_of(nid), tok.start, pk == K::Enum, dkind) {
                    return (SiteKind::Synth, (tok.start, tok.end), String::new());
                }
            }
        }
        cur = st.parent(nid);
    }

    let k1 = st.kind(best_nid);
    let p1 = st.parent(best_nid).map(|p| st.kind(p));
    let p2 = st.parent(best_nid).and_then(|p| st.parent(p)).map(|g| st.kind(g));
    let label = match (k1, p1, p2) {
        (a, Some(b), Some(c)) => format!("{c:?}>{b:?}>{a:?}"),
        (a, Some(b), None) => format!("{b:?}>{a:?}"),
        (a, None, _) => format!("{a:?}"),
    };
    let sk = if in_macro_tt { SiteKind::MacroArg } else { SiteKind::DirectCst };
    (sk, (tok.start, tok.end), label)
}

fn is_adt_tt_attr_or_decl_tok(tt_toks: &[ridx_syntax::Token], tok_start: u32, is_enum: bool, dkind: DefKind) -> bool {
    let mut i = 0usize;
    let mut brace_depth = 0u32;
    let mut in_expr_or_ty = false;
    while i < tt_toks.len() {
        let t = tt_toks[i];
        if t.is_trivia() {
            i += 1;
            continue;
        }
        if t.kind == T::Hash {
            let mut j = i + 1;
            while j < tt_toks.len() && tt_toks[j].is_trivia() {
                j += 1;
            }
            if j < tt_toks.len() && tt_toks[j].kind == T::LBracket {
                let attr_start = t.start;
                let mut bdepth = 1u32;
                let mut k = j + 1;
                let mut attr_end = tt_toks[j].end;
                while k < tt_toks.len() && bdepth > 0 {
                    match tt_toks[k].kind {
                        T::LBracket => bdepth += 1,
                        T::RBracket => bdepth -= 1,
                        _ => {}
                    }
                    attr_end = tt_toks[k].end;
                    k += 1;
                }
                if tok_start >= attr_start && tok_start < attr_end {
                    return true;
                }
                i = k;
                continue;
            }
        }
        match t.kind {
            T::LBrace | T::LParen | T::LBracket => {
                brace_depth += 1;
                if brace_depth == 1 {
                    in_expr_or_ty = false;
                }
            }
            T::RBrace | T::RParen | T::RBracket => {
                brace_depth = brace_depth.saturating_sub(1);
                if brace_depth <= 1 {
                    in_expr_or_ty = false;
                }
            }
            T::Comma if brace_depth == 1 => in_expr_or_ty = false,
            T::Colon | T::Eq if brace_depth == 1 => in_expr_or_ty = true,
            _ => {}
        }
        if t.start == tok_start {
            if matches!(dkind, DefKind::Field | DefKind::Local) {
                return true;
            }
            if is_enum && brace_depth == 1 && !in_expr_or_ty && matches!(dkind, DefKind::Variant | DefKind::VariantCtor | DefKind::StructCtor) {
                return true;
            }
            return false;
        }
        i += 1;
    }
    false
}
