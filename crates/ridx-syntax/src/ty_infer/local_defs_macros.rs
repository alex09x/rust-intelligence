/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Pre-expansion and module-scoped name resolution of body `macro_rules!`, `include!`, and
//! `include_bytes!` inside [`super::local_defs::LocalCstDefs`] so steady-state `infer_fn_into`
//! runs with `0` heap allocations (`Checkbox 3.6.2`).

use crate::name_res::include_eval::eval_include_path_str;
use crate::name_res::mbe_expand::ExpandedSpanMap;
use crate::name_res::{CrateDefMap, CrateId, FileId, FileNameRes, ItemDb, MacroRulesDef, ModuleId};
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::token::Token;
use crate::tree::{Node, NodeId, SyntaxTree};

use super::local_defs::LocalCstDefs;
use super::local_defs_macro_lookup::{expand_include_helper_macro, expand_nested_macros, find_cst_macro_rules_before, lookup_macro_rules};

pub(super) const WRAP_PREFIX: &str = "fn __ridx_mac__() {\n";
pub(super) const WRAP_SUFFIX: &str = "\n}";
pub(super) const MAX_BODY_MBE_DEPTH: u8 = 8;

#[derive(Clone, Debug)]
pub(super) struct PreExpandedMacro {
    pub mac_node: NodeId,
    pub cache_key: u64,
    pub base_inner_s: u32,
    pub src: String,
    pub tokens: Vec<Token>,
    pub nodes: Vec<Node>,
    pub fn_node: NodeId,
    pub block_node: NodeId,
    pub file_res: FileNameRes,
    pub spans: Vec<ExpandedSpanMap>,
}

fn macro_cache_key(krate: CrateId, file: FileId, cur_mod: ModuleId, in_ty_ctx: bool, raw_path: &str, inner_src: &str, body_str: &str) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for b in krate.0.to_le_bytes().into_iter().chain(file.0.to_le_bytes()).chain(cur_mod.0.to_le_bytes()).chain([in_ty_ctx as u8]) {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    for s in [raw_path.as_bytes(), &[0xff], inner_src.as_bytes(), &[0xff], body_str.as_bytes()] {
        for &b in s {
            h = (h ^ b as u64).wrapping_mul(0x100000001b3);
        }
    }
    h
}

pub(super) struct EmptyItemDb;
impl ItemDb for EmptyItemDb {
    fn cfg_flag(&self, _: CrateId, _: &str) -> bool {
        false
    }
    fn cfg_key_value(&self, _: CrateId, _: &str, _: &str) -> bool {
        false
    }
}

impl LocalCstDefs {
    #[inline]
    pub fn swap_macro_caches(&mut self, other: &mut Self) {
        core::mem::swap(&mut self.macro_cache, &mut other.macro_cache);
        core::mem::swap(&mut self.failed_macro_keys, &mut other.failed_macro_keys);
    }

    #[inline]
    pub(super) fn find_body_macro(&self, mac_node: NodeId) -> Option<&PreExpandedMacro> {
        let idx = self.body_macros.binary_search_by_key(&mac_node.0, |m| m.mac_node.0).ok()?;
        self.body_macros.get(idx)
    }

    #[inline]
    pub(super) fn include_bytes_len(&self, mac_node: NodeId) -> Option<u64> {
        let idx = self.include_bytes_lens.binary_search_by_key(&mac_node.0, |&(n, _)| n).ok()?;
        Some(self.include_bytes_lens[idx].1)
    }

    pub(super) fn finalize_macro_tables(&mut self) {
        self.body_macros.sort_unstable_by_key(|m| m.mac_node.0);
        self.body_macros.dedup_by_key(|m| m.mac_node.0);
        self.include_bytes_lens.sort_unstable_by_key(|&(n, _)| n);
        self.include_bytes_lens.dedup_by_key(|p| p.0);
    }

    pub(super) fn scan_body_macros(&mut self, st: &SyntaxTree<'_>, subtree: NodeId, def_map: Option<&CrateDefMap>, item_db: Option<&dyn ItemDb>, file: FileId) {
        if !st.nodes[subtree.0 as usize..st.node(subtree).subtree_end as usize].iter().any(|n| n.kind == K::MacroCall) {
            return;
        }
        let krate = def_map.map_or(CrateId::NONE, |dm| dm.krate);
        for node in st.descendants(subtree) {
            if st.kind(node) != K::MacroCall || self.body_macros.iter().any(|m| m.mac_node == node) {
                continue;
            }
            let (Some(path_node), Some(tt_node)) = (st.child(node, K::Path), st.child(node, K::TokenTree)) else { continue };
            let raw_path = st.text(path_node).trim();
            let Some(tail) = raw_path.rsplit("::").next().map(|s| norm_ident(s.trim())).filter(|s| !s.is_empty()) else { continue };
            let cur_mod = def_map.and_then(|dm| dm.mod_tree.module_at_offset(file, st.range(node).start as u32)).unwrap_or(ModuleId::ROOT);
            if tail == "include_bytes" {
                let idb_ref: &dyn ItemDb = item_db.unwrap_or(&EmptyItemDb);
                let mut resolve_mac = |mp: &str, ma: &str| expand_include_helper_macro(st, node, def_map, item_db, cur_mod, mp, ma);
                if let Some(pstr) = eval_include_path_str(st, node, idb_ref, krate, &mut resolve_mac) {
                    if let Some(len) = item_db.and_then(|idb| idb.include_bytes_len(file, &pstr)) {
                        self.include_bytes_lens.push((node.0, len));
                    }
                }
                continue;
            }
            if tail == "include" {
                let idb_ref: &dyn ItemDb = item_db.unwrap_or(&EmptyItemDb);
                let mut resolve_mac = |mp: &str, ma: &str| expand_include_helper_macro(st, node, def_map, item_db, cur_mod, mp, ma);
                if let Some(pstr) = eval_include_path_str(st, node, idb_ref, krate, &mut resolve_mac) {
                    if let Some((_, inc_text)) = item_db.and_then(|idb| idb.resolve_include_file(file, &pstr)) {
                        if let Some(pre) = build_wrapped_pre(node, 0, 0, inc_text, Vec::new(), st, def_map, item_db, file, false) {
                            self.body_macros.push(pre);
                        }
                    }
                }
                continue;
            }
            let local_hit = (!raw_path.contains("::")).then(|| find_cst_macro_rules_before(st, node, tail)).flatten();
            if local_hit.is_none() && super::infer_macro_std::classify_std_macro_path(st, path_node).is_some() {
                continue;
            }
            let tt_r = st.range(tt_node);
            if tt_r.end.saturating_sub(tt_r.start) < 2
                || !matches!(st.src.as_bytes().get(tt_r.start), Some(b'(' | b'[' | b'{'))
                || !matches!(st.src.as_bytes().get(tt_r.end - 1), Some(b')' | b']' | b'}'))
            {
                continue;
            }
            let (inner_s, inner_e) = (tt_r.start + 1, tt_r.end - 1);
            let Some(inner_src) = st.src.get(inner_s..inner_e) else { continue };
            let in_ty_ctx = st.parent(node).is_some_and(|p| matches!(st.kind(p), K::TypeArg | K::RetType | K::Param | K::TupleType | K::RefType | K::SliceType | K::ArrayType));
            let ckey = macro_cache_key(krate, file, cur_mod, in_ty_ctx, raw_path, inner_src, local_hit.unwrap_or(""));
            let lkey = macro_cache_key(krate, FileId(u32::MAX), cur_mod, false, raw_path, "", local_hit.unwrap_or(""));
            if self.failed_macro_keys.contains(&lkey) || self.failed_macro_keys.contains(&ckey) {
                continue;
            }
            if let Some(pos) = self.macro_cache.iter().position(|m| m.cache_key == ckey) {
                let mut cached = self.macro_cache.swap_remove(pos);
                let delta = inner_s as i64 - cached.base_inner_s as i64;
                cached.mac_node = node;
                cached.base_inner_s = inner_s as u32;
                for sp in &mut cached.spans {
                    sp.orig_start = (sp.orig_start as i64 + delta) as u32;
                }
                self.body_macros.push(cached);
                continue;
            }
            let Some((body_str, dc)) = local_hit.map(|b| (b, "crate")).or_else(|| lookup_macro_rules(def_map, item_db, cur_mod, raw_path)) else {
                self.failed_macro_keys.push(lkey);
                continue;
            };
            let mut spans = Vec::new();
            let Some(mut exp_src) = MacroRulesDef::parse_body(body_str).and_then(|m| m.expand_with_spans(inner_src, inner_s as u32, dc, &mut spans)) else {
                self.failed_macro_keys.push(ckey);
                continue;
            };
            expand_nested_macros(&mut exp_src, &mut spans, st, node, def_map, item_db, cur_mod);
            let pfx_len = if in_ty_ctx { "fn __ridx_m() {\nlet _: ".len() } else { WRAP_PREFIX.len() } as u32;
            for sp in &mut spans {
                sp.exp_start += pfx_len;
                sp.exp_end += pfx_len;
            }
            if let Some(pre) = build_wrapped_pre(node, ckey, inner_s as u32, &exp_src, spans, st, def_map, item_db, file, in_ty_ctx) {
                self.body_macros.push(pre);
            } else {
                self.failed_macro_keys.push(ckey);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn build_wrapped_pre(
    mac_node: NodeId,
    cache_key: u64,
    base_inner_s: u32,
    inner_code: &str,
    spans: Vec<ExpandedSpanMap>,
    orig_st: &SyntaxTree<'_>,
    def_map: Option<&CrateDefMap>,
    item_db: Option<&dyn ItemDb>,
    file: FileId,
    in_ty_ctx: bool,
) -> Option<PreExpandedMacro> {
    if inner_code.len() > 65_536 {
        return None;
    }
    let (pfx, sfx) = if in_ty_ctx { ("fn __ridx_m() {\nlet _: ", ";\n}") } else { (WRAP_PREFIX, WRAP_SUFFIX) };
    let mut wrapped = String::with_capacity(pfx.len() + inner_code.len() + sfx.len());
    wrapped.push_str(pfx);
    wrapped.push_str(inner_code);
    wrapped.push_str(sfx);
    let exp_st = crate::parse(&wrapped);
    let fn_node = exp_st.children(exp_st.root()).find(|&c| exp_st.kind(c) == K::Fn)?;
    let block_node = exp_st.child(fn_node, K::BlockExpr)?;
    let mut file_res = FileNameRes::new();
    if let Some(dm) = def_map {
        let cur_mod = dm.mod_tree.module_at_offset(file, orig_st.range(mac_node).start as u32).unwrap_or(ModuleId::ROOT);
        let idb: &dyn ItemDb = item_db.unwrap_or(&EmptyItemDb);
        dm.resolve_module_snippet_into(idb, cur_mod, file, &exp_st, &mut file_res);
    }
    let SyntaxTree { tokens, nodes, .. } = exp_st;
    Some(PreExpandedMacro { mac_node, cache_key, base_inner_s, src: wrapped, tokens, nodes, fn_node, block_node, file_res, spans })
}

pub(super) fn count_cst_generics(st: &SyntaxTree<'_>, owner: NodeId) -> u16 {
    st.child(owner, K::GenericParamList).map_or(0, |gpl| st.children(gpl).filter(|&c| matches!(st.kind(c), K::TypeParam | K::ConstParam)).count() as u16)
}
