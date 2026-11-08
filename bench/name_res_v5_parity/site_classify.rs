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
//! call-site arguments (Checkbox `3.6`), dot accesses (Phase 3), and disjoint synthetic reasons.

use std::collections::HashSet;

use memmap2::Mmap;
use ridx_format::{DefKind, IndexReader, RefEntry};
use ridx_syntax::name_res::{CrateSnapshot, GlobalDefId, ImplTargetKey, ItemDb, ItemKind};
use ridx_syntax::{NodeId, SyntaxKind as K, SyntaxTree, TokenKind as T};

use crate::meta::CorpusFile;
use crate::v5_db::{RidxV5ItemDb, CRATE_ROOT_DEF_IDX, IMPL_SYNTH_BIT, PROC_MACRO_SYNTH_BIT};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SynthReason {
    SpanNotToken = 0,
    AttrNameMacroRules = 1,
    AdtTtAttrOrDecl = 2,
    LeafNeIdent = 3,
}

impl SynthReason {
    pub const ALL: [Self; 4] = [Self::SpanNotToken, Self::AttrNameMacroRules, Self::AdtTtAttrOrDecl, Self::LeafNeIdent];

    #[inline]
    pub fn label(self) -> &'static str {
        match self {
            Self::SpanNotToken => "span != token",
            Self::AttrNameMacroRules => "inside Attr/Name/MacroRules",
            Self::AdtTtAttrOrDecl => "ADT TT attr/decl",
            Self::LeafNeIdent => "driver leaf != source ident",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SiteKind {
    DirectCst,
    MacroArg,
    Synth(SynthReason),
}

pub struct ClassifyOut {
    pub kind: SiteKind,
    pub tok_span: (u32, u32),
    pub label: String,
    pub moved_real_alias: bool,
}

fn def_leaf_name<'a>(db: &'a RidxV5ItemDb<'_>, gid: GlobalDefId) -> &'a str {
    let r_idx = gid.krate.0 as usize;
    if gid.index == CRATE_ROOT_DEF_IDX {
        return db.metas.get(r_idx).map_or("", |m| m.crate_name.trim_start_matches("r#"));
    }
    if (gid.index & PROC_MACRO_SYNTH_BIT) != 0 {
        let sym = (gid.index & !PROC_MACRO_SYNTH_BIT) as usize;
        return db.shards.get(r_idx).and_then(|s| s.synth_macros.get(sym)).map_or("", |s| s.trim_start_matches("r#"));
    }
    if (gid.index & IMPL_SYNTH_BIT) != 0 {
        return "";
    }
    let Some(reader) = db.ws.readers().get(r_idx) else {
        return "";
    };
    reader.definitions().get(gid.index as usize).map_or("", |d| reader.get_str(d.name_offset()).trim_start_matches("r#"))
}

pub fn collect_unit_alias_pairs(r_idx: usize, snap: &CrateSnapshot, db: &RidxV5ItemDb<'_>, out: &mut HashSet<(String, String)>) {
    let dm = &snap.clean_def_map;
    for (ext_alias, uk) in &db.metas[r_idx].extern_units {
        let alias = ext_alias.trim_start_matches("r#");
        if let Some(tr_idx) = db.resolve_unit_to_reader(r_idx, uk) {
            let orig_cname = db.metas[tr_idx].crate_name.trim_start_matches("r#");
            out.insert((alias.to_string(), orig_cname.to_string()));
            out.insert((alias.to_string(), String::new()));
        }
    }
    for ec in &dm.uses.extern_crates {
        if ec.is_underscore {
            continue;
        }
        let bind = dm.uses.extern_bind(ec).trim_start_matches("r#");
        let orig = dm.uses.extern_orig(ec).trim_start_matches("r#");
        if !bind.is_empty() && bind != orig {
            out.insert((bind.to_string(), orig.to_string()));
            out.insert((bind.to_string(), String::new()));
        }
    }
    for (uidx, u) in dm.uses.uses.iter().enumerate() {
        let bind = dm.uses.bind_name(u).trim_start_matches("r#");
        if bind.is_empty() || bind == "_" {
            continue;
        }
        if let Some(&last_seg_range) = dm.uses.seg_spans(u).last() {
            let last_seg = dm.uses.str_span(last_seg_range).trim_start_matches("r#");
            if !last_seg.is_empty() && bind != last_seg {
                out.insert((bind.to_string(), last_seg.to_string()));
                out.insert((bind.to_string(), String::new()));
            }
        }
        let pns = dm.resolved_uses[uidx];
        for b in [pns.types, pns.values, pns.macros].into_iter().flatten() {
            if b.def.is_some() && !b.def.is_overlay() {
                let leaf = def_leaf_name(db, b.def);
                if !leaf.is_empty() && bind != leaf {
                    out.insert((bind.to_string(), leaf.to_string()));
                }
            }
        }
    }
    for it in &dm.items {
        if it.kind == ItemKind::TypeAlias && it.index_def.is_some() && !it.index_def.is_overlay() {
            let alias = dm.item_name(it.id).trim_start_matches("r#");
            if let Some(ImplTargetKey::Adt(adt_def)) = db.type_alias_target(it.index_def) {
                let leaf = def_leaf_name(db, adt_def);
                if !leaf.is_empty() && alias != leaf {
                    out.insert((alias.to_string(), leaf.to_string()));
                }
            }
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

#[allow(clippy::too_many_arguments)]
pub fn classify_ref_site(
    reader: &IndexReader<Mmap>,
    db: &RidxV5ItemDb<'_>,
    cf: &CorpusFile,
    st: &SyntaxTree<'_>,
    unit_alias_pairs: &HashSet<(String, String)>,
    r: &RefEntry,
    sb: u32,
    eb: u32,
) -> ClassifyOut {
    let tidx = st.tokens.partition_point(|t| t.end <= sb);
    let Some(&tok) = st.tokens.get(tidx) else {
        return ClassifyOut { kind: SiteKind::Synth(SynthReason::SpanNotToken), tok_span: (sb, eb), label: String::new(), moved_real_alias: false };
    };
    let exact_tok = (tok.start == sb && tok.end == eb) || (tok.kind == T::RawIdent && tok.start + 2 == sb && tok.end == eb);
    if !exact_tok || !matches!(tok.kind, T::Ident | T::RawIdent | T::SelfKw | T::SelfType | T::Super | T::Crate) {
        return ClassifyOut { kind: SiteKind::Synth(SynthReason::SpanNotToken), tok_span: (sb, eb), label: String::new(), moved_real_alias: false };
    }

    let dkind = DefKind::from(r.kind);
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
            return ClassifyOut { kind: SiteKind::Synth(SynthReason::AttrNameMacroRules), tok_span: (tok.start, tok.end), label: String::new(), moved_real_alias: false };
        }
        if k == K::TokenTree {
            if let Some(pk) = st.parent(nid).map(|p| st.kind(p)) {
                if pk == K::MacroCall {
                    in_macro_tt = true;
                } else if matches!(pk, K::Struct | K::Union | K::Enum) && is_adt_tt_attr_or_decl_tok(st.tokens_of(nid), tok.start, pk == K::Enum, dkind) {
                    return ClassifyOut { kind: SiteKind::Synth(SynthReason::AdtTtAttrOrDecl), tok_span: (tok.start, tok.end), label: String::new(), moved_real_alias: false };
                }
            }
        }
        cur = st.parent(nid);
    }

    let mut moved_real_alias = false;
    if dkind != DefKind::Local {
        let raw_txt = tok.text(&cf.text);
        let src_ident = raw_txt.strip_prefix("r#").unwrap_or(raw_txt);
        if !matches!(src_ident, "self" | "Self" | "crate" | "super") {
            let dp = reader.get_str(r.target_path_offset());
            let drv_leaf_raw = dp.rsplit("::").next().unwrap_or(dp);
            let drv_leaf = drv_leaf_raw.strip_prefix("r#").unwrap_or(drv_leaf_raw);
            if src_ident != drv_leaf {
                let pair = (src_ident.to_string(), drv_leaf.to_string());
                if db.valid_alias_leaf_pairs.contains(&pair) || unit_alias_pairs.contains(&pair) {
                    moved_real_alias = true;
                } else {
                    return ClassifyOut { kind: SiteKind::Synth(SynthReason::LeafNeIdent), tok_span: (tok.start, tok.end), label: String::new(), moved_real_alias: false };
                }
            }
        }
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
    ClassifyOut { kind: sk, tok_span: (tok.start, tok.end), label, moved_real_alias }
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
            if is_enum && brace_depth == 1 && !in_expr_or_ty {
                return true;
            }
            return false;
        }
        i += 1;
    }
    false
}
