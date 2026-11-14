/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! CST and [`CrateDefMap`] macro lookup, nested body `macro_rules!` expansion, and span map splicing
//! for [`super::local_defs_macros`].

use crate::name_res::mbe_expand::ExpandedSpanMap;
use crate::name_res::{CrateDefMap, CrateId, ItemDb, MacroRulesDef, ModuleId, PathAnchor};
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::tree::{NodeId, SyntaxTree};

use super::local_defs_macros::{EmptyItemDb, MAX_BODY_MBE_DEPTH, WRAP_PREFIX, WRAP_SUFFIX};

pub(super) fn find_cst_macro_rules_before<'s>(st: &'s SyntaxTree<'s>, at_node: NodeId, target_name: &str) -> Option<&'s str> {
    for anc in st.ancestors(at_node) {
        if !matches!(st.kind(anc), K::BlockExpr | K::ItemList | K::SourceFile) {
            continue;
        }
        let mut found: Option<&'s str> = None;
        for ch in st.children(anc) {
            if st.kind(anc) == K::BlockExpr && ch.0 >= at_node.0 {
                break;
            }
            if st.kind(ch) == K::MacroRules {
                if let Some(nn) = st.child(ch, K::Name) {
                    if norm_ident(st.text(nn)) == target_name {
                        if let Some(tt) = st.child(ch, K::TokenTree) {
                            let r = st.range(tt);
                            found = st.src.get(r.start..r.end);
                        }
                    }
                }
            }
        }
        if found.is_some() {
            return found;
        }
    }
    None
}

pub(super) fn lookup_macro_rules<'a>(def_map: Option<&'a CrateDefMap>, item_db: Option<&'a dyn ItemDb>, cur_mod: ModuleId, raw_path: &str) -> Option<(&'a str, &'a str)> {
    let trimmed = raw_path.trim();
    let rooted = trimmed.starts_with("::");
    let clean = trimmed.trim_start_matches("::");
    let mut seg_buf = [""; 8];
    let mut slen = 0usize;
    for part in clean.split("::") {
        let s = norm_ident(part.trim());
        if !s.is_empty() && slen < seg_buf.len() {
            seg_buf[slen] = s;
            slen += 1;
        }
    }
    if slen == 0 {
        return None;
    }
    if let Some(dm) = def_map {
        let idb: &dyn ItemDb = item_db.unwrap_or(&EmptyItemDb);
        let binding = if slen == 1 && !rooted {
            dm.resolve_name_in_module(&dm.mod_tree, cur_mod, seg_buf[0]).macros.or_else(|| dm.scope(ModuleId::ROOT).get(seg_buf[0]).macros)
        } else {
            let (anchor, start) = if rooted {
                (PathAnchor::ExternRoot, 0)
            } else {
                match seg_buf[0] {
                    "crate" => (PathAnchor::CrateRoot, 1),
                    "self" => (PathAnchor::SelfMod, 1),
                    "super" => {
                        let mut n = 0usize;
                        while n < slen && seg_buf[n] == "super" {
                            n += 1;
                        }
                        (PathAnchor::Super(n as u16), n)
                    }
                    _ => (PathAnchor::Bare, 0),
                }
            };
            (start < slen).then(|| dm.resolve_path(idb, &dm.mod_tree, cur_mod, anchor, &seg_buf[start..slen]).macros).flatten()
        };
        if let Some(b) = binding {
            if b.local.is_some() {
                if let Some(body) = dm.local_macro_body(b.local) {
                    return Some((body, "crate"));
                }
            }
            if b.def.is_some() {
                if (b.def.index & 0x4000_0000) != 0 {
                    return None;
                }
                if let Some(idb_real) = item_db {
                    if let Some(hit) = idb_real.macro_rules_by_def(dm.krate, b.def) {
                        return Some(hit);
                    }
                }
            }
        }
        if let Some(idb_real) = item_db {
            let mod_path = dm.mod_tree.crate_path(cur_mod);
            return idb_real.macro_rules_with_crate(dm.krate, mod_path, trimmed);
        }
    } else if let Some(idb_real) = item_db {
        return idb_real.macro_rules_with_crate(CrateId::NONE, "crate", trimmed);
    }
    None
}

pub(super) fn expand_nested_macros(
    exp_src: &mut String,
    spans: &mut Vec<ExpandedSpanMap>,
    orig_st: &SyntaxTree<'_>,
    orig_node: NodeId,
    def_map: Option<&CrateDefMap>,
    item_db: Option<&dyn ItemDb>,
    cur_mod: ModuleId,
) {
    for _ in 1..MAX_BODY_MBE_DEPTH {
        if exp_src.len() > 32_768 || !exp_src.contains('!') {
            break;
        }
        let mut probe = String::with_capacity(WRAP_PREFIX.len() + exp_src.len() + WRAP_SUFFIX.len());
        probe.push_str(WRAP_PREFIX);
        probe.push_str(exp_src);
        probe.push_str(WRAP_SUFFIX);
        let pst = crate::parse(&probe);
        let mut replaced = false;
        for ch in pst.descendants(pst.root()) {
            if pst.kind(ch) != K::MacroCall {
                continue;
            }
            let (Some(pnode), Some(tt)) = (pst.child(ch, K::Path), pst.child(ch, K::TokenTree)) else { continue };
            if super::infer_macro_std::classify_std_macro_path(&pst, pnode).is_some() {
                continue;
            }
            let rpath = pst.text(pnode).trim();
            let tail = rpath.rsplit("::").next().map_or("", |s| norm_ident(s.trim()));
            let resolved = (!rpath.contains("::"))
                .then(|| find_cst_macro_rules_before(orig_st, orig_node, tail).map(|b| (b, "crate")))
                .flatten()
                .or_else(|| lookup_macro_rules(def_map, item_db, cur_mod, rpath));
            let Some((bstr, dc)) = resolved else { continue };
            let cr = pst.range(ch);
            let tr = pst.range(tt);
            let body_end = WRAP_PREFIX.len() + exp_src.len();
            if cr.start < WRAP_PREFIX.len()
                || cr.end > body_end
                || tr.start < cr.start
                || tr.end > cr.end
                || tr.end.saturating_sub(tr.start) < 2
                || !matches!(probe.as_bytes().get(tr.start), Some(b'(' | b'[' | b'{'))
                || !matches!(probe.as_bytes().get(tr.end - 1), Some(b')' | b']' | b'}'))
            {
                continue;
            }
            let (mc_s, mc_e) = (cr.start - WRAP_PREFIX.len(), cr.end - WRAP_PREFIX.len());
            let (in_s, in_e) = (tr.start + 1 - WRAP_PREFIX.len(), tr.end - 1 - WRAP_PREFIX.len());
            if mc_s > mc_e || !exp_src.is_char_boundary(mc_s) || !exp_src.is_char_boundary(mc_e) {
                continue;
            }
            let Some(inner_src) = exp_src.get(in_s..in_e) else { continue };
            let Some(mdef) = MacroRulesDef::parse_body(bstr) else { continue };
            let mut sub_spans = Vec::new();
            let Some(sub_exp) = mdef.expand_with_spans(inner_src, in_s as u32, dc, &mut sub_spans) else { continue };
            splice_span_maps(spans, mc_s as u32, mc_e as u32, sub_exp.len() as u32, &sub_spans);
            exp_src.replace_range(mc_s..mc_e, &sub_exp);
            replaced = true;
            break;
        }
        if !replaced {
            break;
        }
    }
}

fn splice_span_maps(spans: &mut Vec<ExpandedSpanMap>, mc_s: u32, mc_e: u32, new_len: u32, sub_spans: &[ExpandedSpanMap]) {
    let old = core::mem::take(spans);
    let delta = new_len as i64 - (mc_e - mc_s) as i64;
    for s in &old {
        if s.exp_end <= mc_s {
            spans.push(*s);
        }
    }
    for cs in sub_spans {
        let len = cs.exp_end - cs.exp_start;
        if let Some(p) = old.iter().find(|p| cs.orig_start >= p.exp_start && cs.orig_start + len <= p.exp_end) {
            spans.push(ExpandedSpanMap { exp_start: mc_s + cs.exp_start, exp_end: mc_s + cs.exp_end, orig_start: p.orig_start + (cs.orig_start - p.exp_start) });
        }
    }
    for s in &old {
        if s.exp_start >= mc_e {
            spans.push(ExpandedSpanMap { exp_start: (s.exp_start as i64 + delta) as u32, exp_end: (s.exp_end as i64 + delta) as u32, orig_start: s.orig_start });
        }
    }
}

pub(super) fn expand_include_helper_macro(
    st: &SyntaxTree<'_>,
    at_node: NodeId,
    def_map: Option<&CrateDefMap>,
    item_db: Option<&dyn ItemDb>,
    cur_mod: ModuleId,
    mpath: &str,
    margs: &str,
) -> Option<String> {
    let tail = mpath.rsplit("::").next().map_or("", |s| norm_ident(s.trim()));
    let (body_str, dc) = (!mpath.contains("::"))
        .then(|| find_cst_macro_rules_before(st, at_node, tail).map(|b| (b, "crate")))
        .flatten()
        .or_else(|| lookup_macro_rules(def_map, item_db, cur_mod, mpath))?;
    MacroRulesDef::parse_body(body_str)?.expand(margs, dc)
}
