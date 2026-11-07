/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! CST walking for `impl` headers, associated item lists, and item-position macro expansions
//! inside `impl` / `trait` blocks.

use super::attr::{eval_outer_attrs, extract_visibility};
use super::db::{CrateId, FileId, GlobalDefId, ItemDb, ItemKind, ModuleId, VisKind};
use super::db_impls::{ImplTargetKey, LocalImplId};
use super::def_collect::MacroScopeVec;
use super::impl_collect::{LocalAssocItem, LocalImplArena, LocalImplData};
use super::mbe_expand::MAX_MBE_DEPTH;
use super::mbe_parse::MacroRulesDef;
use super::mod_macro_scope::has_scope_item_kw;
use super::mod_tree::ModuleTree;
use super::per_ns::{compute_vis_mod, LocalItemId};
use super::use_tree::PathAnchor;
use crate::parser::parse;
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::token::TokenKind as T;
use crate::tree::{NodeId, SyntaxTree};

impl LocalImplArena {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn collect_impl_node(
        &mut self,
        st: &SyntaxTree<'_>,
        impl_node: NodeId,
        mod_tree: &ModuleTree,
        mod_id: ModuleId,
        file: FileId,
        db: &dyn ItemDb,
        krate: CrateId,
        macro_defs: &mut Vec<(String, String, Option<MacroRulesDef>)>,
        macro_env: &MacroScopeVec,
    ) {
        let attrs = eval_outer_attrs(st, impl_node, db, krate);
        if !attrs.enabled {
            return;
        }
        let (gp_start, gp_len) = self.push_generic_type_params(st, impl_node);
        let mut ty_nodes = [NodeId(0); 2];
        let mut ty_cnt = 0usize;
        for ch in st.children(impl_node) {
            if st.kind(ch).is_type() && ty_cnt < 2 {
                ty_nodes[ty_cnt] = ch;
                ty_cnt += 1;
            }
        }
        if ty_cnt == 0 {
            return;
        }
        let (has_trait, trait_anchor, trait_seg_start, trait_seg_len, self_ty_node) = if ty_cnt == 2 {
            let (ta, ts, tl) = match st.child(ty_nodes[0], K::Path) {
                Some(p) => self.push_cst_path(st, p),
                None => (PathAnchor::Bare, 0, 0),
            };
            (true, ta, ts, tl, ty_nodes[1])
        } else {
            (false, PathAnchor::Bare, 0, 0, ty_nodes[0])
        };
        let is_negative = has_trait && header_has_negative_bang(st, impl_node, ty_nodes[0]);
        let raw_head = self.extract_type_head(st, self_ty_node);
        let impl_id = LocalImplId(self.impls.len() as u32);
        let assoc_start = self.assoc_items.len() as u32;
        if let Some(il) = st.child(impl_node, K::ItemList) {
            self.collect_assoc_list(st, il, mod_tree, mod_id, file, impl_id, LocalItemId::NONE, has_trait, db, krate, macro_defs, macro_env, 0);
        }
        let assoc_len = (self.assoc_items.len() as u32 - assoc_start).min(u16::MAX as u32) as u16;
        let r = st.range(impl_node);
        self.impls.push(LocalImplData {
            id: impl_id,
            module: mod_id,
            file,
            index_def: GlobalDefId::NONE,
            is_negative,
            has_trait,
            raw_head,
            trait_anchor,
            trait_seg_start,
            trait_seg_len,
            gp_start,
            gp_len,
            assoc_start,
            assoc_len,
            target: ImplTargetKey::Unknown,
            trait_def: GlobalDefId::NONE,
            span_start: r.start as u32,
            span_end: r.end as u32,
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn collect_assoc_list(
        &mut self,
        st: &SyntaxTree<'_>,
        item_list: NodeId,
        mod_tree: &ModuleTree,
        mod_id: ModuleId,
        file: FileId,
        owner_impl: LocalImplId,
        owner_trait: LocalItemId,
        is_trait_impl: bool,
        db: &dyn ItemDb,
        krate: CrateId,
        macro_defs: &mut Vec<(String, String, Option<MacroRulesDef>)>,
        macro_env: &MacroScopeVec,
        mbe_depth: u32,
    ) {
        for ch in st.children(item_list) {
            let kind = match st.kind(ch) {
                K::Fn => ItemKind::Fn,
                K::Const => ItemKind::Const,
                K::TypeAlias => ItemKind::TypeAlias,
                K::MacroCall if mbe_depth < MAX_MBE_DEPTH => {
                    if let Some(exp) = expand_assoc_macro(st, ch, mod_tree, mod_id, db, krate, macro_defs, macro_env) {
                        let exp_st = parse(&exp);
                        self.collect_assoc_list(
                            &exp_st,
                            exp_st.root(),
                            mod_tree,
                            mod_id,
                            file,
                            owner_impl,
                            owner_trait,
                            is_trait_impl,
                            db,
                            krate,
                            macro_defs,
                            macro_env,
                            mbe_depth + 1,
                        );
                    }
                    continue;
                }
                _ => continue,
            };
            let attrs = eval_outer_attrs(st, ch, db, krate);
            if !attrs.enabled {
                continue;
            }
            let Some(nn) = st.child(ch, K::Name) else { continue };
            let name = norm_ident(st.text(nn));
            if name.is_empty() {
                continue;
            }
            let (raw_vis, in_path) = extract_visibility(st, ch);
            let (vis, vis_mod) = if owner_trait.is_some() || is_trait_impl { (VisKind::Public, ModuleId::NONE) } else { compute_vis_mod(mod_tree, mod_id, raw_vis, in_path) };
            let has_self_receiver = kind == ItemKind::Fn && st.child(ch, K::ParamList).is_some_and(|pl| st.child(pl, K::SelfParam).is_some());
            let (name_start, name_end) = self.push_str(name);
            let nr = st.range(nn);
            self.assoc_items.push(LocalAssocItem {
                def: GlobalDefId::NONE,
                module: mod_id,
                file,
                owner_impl,
                owner_trait,
                name_start,
                name_end,
                kind,
                vis,
                vis_mod,
                has_self_receiver,
                span_start: nr.start as u32,
                span_end: nr.end as u32,
            });
        }
    }
}

fn header_has_negative_bang(st: &SyntaxTree<'_>, impl_node: NodeId, first_ty: NodeId) -> bool {
    let start_tok = st.child(impl_node, K::GenericParamList).map_or(st.node(impl_node).first, |g| st.node(g).end);
    let end_tok = st.node(first_ty).first.max(start_tok);
    st.tokens[start_tok as usize..end_tok as usize].iter().any(|t| t.kind == T::Bang)
}

#[allow(clippy::too_many_arguments)]
fn expand_assoc_macro(
    st: &SyntaxTree<'_>,
    call_node: NodeId,
    mod_tree: &ModuleTree,
    mod_id: ModuleId,
    db: &dyn ItemDb,
    krate: CrateId,
    macro_defs: &mut [(String, String, Option<MacroRulesDef>)],
    macro_env: &MacroScopeVec,
) -> Option<String> {
    let (path_node, tt_node) = (st.child(call_node, K::Path)?, st.child(call_node, K::TokenTree)?);
    let tt = st.text(tt_node);
    let inner = if tt.len() >= 2 { &tt[1..tt.len() - 1] } else { tt };
    if !has_scope_item_kw(inner) {
        return None;
    }
    if !eval_outer_attrs(st, call_node, db, krate).enabled {
        return None;
    }
    let raw_call = st.text(path_node).trim();
    if !raw_call.contains("::") {
        let short = norm_ident(raw_call);
        if let Some(&(_, idx, _, _)) = macro_env.iter().rfind(|(n, _, _, _)| n == short) {
            let (_, body, cache) = &mut macro_defs[idx];
            if cache.is_none() {
                *cache = MacroRulesDef::parse_body(body);
            }
            return cache.as_ref()?.expand(inner, "crate");
        }
    }
    db.macro_rules_body(krate, mod_tree.crate_path(mod_id), raw_call).and_then(MacroRulesDef::parse_body).and_then(|d| d.expand(inner, "crate"))
}
