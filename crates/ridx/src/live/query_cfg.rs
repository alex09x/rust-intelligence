/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `cfg` gates at the cursor: whether the code there is compiled, and by which unit.

use std::path::Path;

use ridx_syntax::name_res::{eval_attr_bracket_tokens, eval_inner_attrs, eval_outer_attrs, CrateId, EffectiveAttrs};
use ridx_syntax::{NodeId, SyntaxKind as K, SyntaxTree, TokenKind as T};

use super::v5_db::RidxV5ItemDb;

fn check_node_cfg_at(st: &SyntaxTree<'_>, nid: NodeId, b: usize, db: &RidxV5ItemDb<'_>, krate: CrateId) -> bool {
    let r = st.range(nid);
    if b < r.start || b > r.end {
        return true;
    }
    if matches!(st.kind(nid), K::SourceFile | K::ItemList | K::BlockExpr) {
        let mut inner_eff = EffectiveAttrs::default();
        eval_inner_attrs(st, nid, db, krate, &mut inner_eff);
        if !inner_eff.enabled {
            return false;
        }
    }
    if st.children(nid).next().is_some_and(|c| st.kind(c) == K::Attr) && !eval_outer_attrs(st, nid, db, krate).enabled {
        return false;
    }
    let mut pattrs = [NodeId(0); 8];
    let mut plen = 0usize;
    for ch in st.children(nid) {
        if st.kind(ch) == K::Attr {
            let toks = st.tokens_of(ch);
            let mut sig = toks.iter().filter(|t| !t.is_trivia());
            let is_inner = matches!(sig.next().map(|t| t.kind), Some(T::Hash)) && matches!(sig.next().map(|t| t.kind), Some(T::Bang));
            if !is_inner && plen < pattrs.len() {
                pattrs[plen] = ch;
                plen += 1;
            }
            continue;
        }
        let cr = st.range(ch);
        if cr.start > b {
            break;
        }
        if b <= cr.end {
            for &pa in &pattrs[..plen] {
                let toks = st.tokens_of(pa);
                if let Some(open) = toks.iter().position(|t| t.kind == T::LBracket) {
                    let close = toks.iter().rposition(|t| t.kind == T::RBracket).filter(|&c| c > open).unwrap_or(toks.len());
                    let mut eff = EffectiveAttrs::default();
                    eval_attr_bracket_tokens(st.src, &toks[open + 1..close], db, krate, &mut eff);
                    if !eff.enabled {
                        return false;
                    }
                }
            }
            if !check_node_cfg_at(st, ch, b, db, krate) {
                return false;
            }
        }
        plen = 0;
    }
    true
}

#[inline]
fn cursor_cfg_enabled_in_unit(st: &SyntaxTree<'_>, off: u32, db: &RidxV5ItemDb<'_>, krate: CrateId) -> bool {
    check_node_cfg_at(st, st.root(), off as usize, db, krate)
}

/// Whether the code at `off` is compiled by no unit that compiles `canon` (the cursor sits in inactive `cfg` code).
pub(crate) fn is_inside_inactive_target_cfg(st: &SyntaxTree<'_>, off: u32, db: &RidxV5ItemDb<'_>, primary_r_idx: usize, canon: &Path) -> bool {
    unit_enabling_cursor(st, off, db, primary_r_idx, canon).is_none()
}

/// The unit whose `cfg` compiles the code at `off`: `primary_r_idx` when it does, otherwise another workspace unit
/// that compiles the same file (a test harness for `#[cfg(test)]` code), or `None` when no unit does.
pub(crate) fn unit_enabling_cursor(st: &SyntaxTree<'_>, off: u32, db: &RidxV5ItemDb<'_>, primary_r_idx: usize, canon: &Path) -> Option<usize> {
    if !st.src.contains("cfg") || cursor_cfg_enabled_in_unit(st, off, db, CrateId(primary_r_idx as u32)) {
        return Some(primary_r_idx);
    }
    if let Some(units) = db.units_by_file.get(canon) {
        for &(u, _) in units {
            let meta = &db.metas[u];
            if u != primary_r_idx && !meta.is_dependency && meta.target_kind != "custom-build" && cursor_cfg_enabled_in_unit(st, off, db, CrateId(u as u32)) {
                return Some(u);
            }
        }
        return None;
    }
    let primary_meta = &db.metas[primary_r_idx];
    for u in 0..db.ws.crate_count() {
        if u == primary_r_idx {
            continue;
        }
        let meta = &db.metas[u];
        if meta.is_dependency || meta.target_kind == "custom-build" {
            continue;
        }
        let shares_target = super::query_unit::reader_stores(db.ws, u, canon)
            || (meta.source_root.is_some() && meta.source_root == primary_meta.source_root && meta.crate_name == primary_meta.crate_name);
        if shares_target && cursor_cfg_enabled_in_unit(st, off, db, CrateId(u as u32)) {
            return Some(u);
        }
    }
    None
}
