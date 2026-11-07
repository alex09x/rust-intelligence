/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Per-item CST extraction, enum-variant parsing, and macro-scope expansion for [`Collector`].

use super::attr::{eval_attr_bracket_tokens, eval_outer_attrs, extract_visibility, EffectiveAttrs};
use super::db::{CrateId, FileId, GlobalDefId, ItemDb, ItemKind, ModuleId, VisKind};
use super::db_impls::LocalImplId;
use super::def_collect::{Collector, LocalItemData, MacroScopeVec};
use super::impl_collect::LocalTypeAlias;
use super::mbe_parse::MacroRulesDef;
use super::mod_macro_scope::has_scope_item_kw;
use super::per_ns::{compute_vis_mod, LocalItemId};
use crate::parser::parse;
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::token::{Token, TokenKind as T};
use crate::tree::{NodeId, SyntaxTree};

impl<'a> Collector<'a> {
    pub(super) fn collect_named_item(&mut self, mod_id: ModuleId, file: FileId, st: &SyntaxTree<'_>, node: NodeId, kind: ItemKind) -> LocalItemId {
        let attrs = eval_outer_attrs(st, node, self.db, self.krate);
        if !attrs.enabled {
            return LocalItemId::NONE;
        }
        let Some(name_node) = st.child(node, K::Name) else { return LocalItemId::NONE };
        let name = norm_ident(st.text(name_node));
        if name.is_empty() || name == "_" {
            return LocalItemId::NONE;
        }
        let (raw_vis, in_path) = extract_visibility(st, node);
        let (vis, vis_mod) = compute_vis_mod(self.mod_tree, mod_id, raw_vis, in_path);
        let has_value_ctor = kind == ItemKind::Struct && struct_has_value_ctor(st, node);
        let index_def = self.find_idx_def(name, kind);
        let nr = st.range(name_node);
        self.push_item(mod_id, file, LocalItemId::NONE, index_def, name, kind, vis, vis_mod, has_value_ctor, false, nr.start as u32, nr.end as u32)
    }

    pub(super) fn collect_trait_item(&mut self, mod_id: ModuleId, file: FileId, st: &SyntaxTree<'_>, node: NodeId, macro_env: &MacroScopeVec) {
        let lid = self.collect_named_item(mod_id, file, st, node, ItemKind::Trait);
        if !lid.is_some() || !self.should_scan_impls(file) {
            return;
        }
        let a_start = self.out.local_impls.assoc_items.len() as u32;
        if let Some(il) = st.child(node, K::ItemList) {
            self.out.local_impls.collect_assoc_list(st, il, self.mod_tree, mod_id, file, LocalImplId::NONE, lid, false, self.db, self.krate, &mut self.macro_defs, macro_env, 0);
        }
        let a_len = (self.out.local_impls.assoc_items.len() as u32 - a_start).min(u16::MAX as u32) as u16;
        self.out.local_impls.trait_assocs.push((lid, a_start, a_len));
    }

    pub(super) fn collect_type_alias_item(&mut self, mod_id: ModuleId, file: FileId, st: &SyntaxTree<'_>, node: NodeId) {
        let lid = self.collect_named_item(mod_id, file, st, node, ItemKind::TypeAlias);
        if !lid.is_some() || !self.should_scan_impls(file) {
            return;
        }
        if let Some(ty_node) = st.children(node).find(|&c| st.kind(c).is_type()) {
            let (gp_start, gp_len) = self.out.local_impls.push_generic_type_params(st, node);
            let raw_head = self.out.local_impls.extract_type_head(st, ty_node);
            self.out.local_impls.type_aliases.push(LocalTypeAlias { item_id: lid, module: mod_id, gp_start, gp_len, raw_head });
        }
    }

    pub(super) fn collect_enum_item(&mut self, mod_id: ModuleId, file: FileId, st: &SyntaxTree<'_>, node: NodeId) {
        let attrs = eval_outer_attrs(st, node, self.db, self.krate);
        if !attrs.enabled {
            return;
        }
        let Some(name_node) = st.child(node, K::Name) else { return };
        let name = norm_ident(st.text(name_node));
        if name.is_empty() {
            return;
        }
        let (raw_vis, in_path) = extract_visibility(st, node);
        let (vis, vis_mod) = compute_vis_mod(self.mod_tree, mod_id, raw_vis, in_path);
        let enum_idx_def = self.find_idx_def(name, ItemKind::Enum);
        let nr = st.range(name_node);
        let enum_local = self.push_item(mod_id, file, LocalItemId::NONE, enum_idx_def, name, ItemKind::Enum, vis, vis_mod, false, false, nr.start as u32, nr.end as u32);
        let Some(tt_node) = st.child(node, K::TokenTree) else { return };
        self.scratch_var_defs.clear();
        if enum_idx_def.is_some() {
            self.db.enum_variants(enum_idx_def, &mut self.scratch_var_defs);
        }
        let toks = st.tokens_of(tt_node);
        let var_start = self.out.items.len() as u32;
        for (vname, has_ctor, vstart, vend) in parse_enum_variants(st.src, toks, self.db, self.krate) {
            let v_def = self.scratch_var_defs.iter().find(|d| d.kind == ItemKind::Variant && d.name == vname).map_or(GlobalDefId::NONE, |d| d.id);
            self.push_item(mod_id, file, enum_local, v_def, vname, ItemKind::Variant, vis, vis_mod, has_ctor, false, vstart, vend);
        }
        let var_len = (self.out.items.len() as u32).saturating_sub(var_start).min(u16::MAX as u32) as u16;
        self.out.items[enum_local.0 as usize].var_start = var_start;
        self.out.items[enum_local.0 as usize].var_len = var_len;
    }

    pub(super) fn collect_macro_rules(&mut self, mod_id: ModuleId, file: FileId, st: &SyntaxTree<'_>, node: NodeId, macro_env: &mut MacroScopeVec) {
        let attrs = eval_outer_attrs(st, node, self.db, self.krate);
        if !attrs.enabled {
            return;
        }
        let Some(name_node) = st.child(node, K::Name) else { return };
        let name = norm_ident(st.text(name_node));
        if name.is_empty() {
            return;
        }
        let (raw_vis, in_path) = extract_visibility(st, node);
        let (vis, vis_mod) = if attrs.macro_export { (VisKind::Public, ModuleId::NONE) } else { compute_vis_mod(self.mod_tree, mod_id, raw_vis, in_path) };
        let idx_def = self.find_idx_def(name, ItemKind::Macro);
        let nr = st.range(name_node);
        let lid = self.push_item(mod_id, file, LocalItemId::NONE, idx_def, name, ItemKind::Macro, vis, vis_mod, false, attrs.macro_export, nr.start as u32, nr.end as u32);
        self.out.textual_macros[mod_id.0 as usize].push((lid, idx_def));
        if let Some(tt_node) = st.child(node, K::TokenTree) {
            let def_idx = self.macro_defs.len();
            self.macro_defs.push((name.to_string(), st.text(tt_node).to_string(), None));
            macro_env.push((name.to_string(), def_idx, lid, idx_def));
        }
    }

    pub(super) fn try_expand_scope_macro(&mut self, mod_id: ModuleId, file: FileId, st: &SyntaxTree<'_>, call_node: NodeId, macro_env: &mut MacroScopeVec, mbe_depth: u32) {
        let (Some(path_node), Some(tt_node)) = (st.child(call_node, K::Path), st.child(call_node, K::TokenTree)) else {
            return;
        };
        let tt_text = st.text(tt_node);
        let inner = if tt_text.len() >= 2 { &tt_text[1..tt_text.len() - 1] } else { tt_text };
        if !has_scope_item_kw(inner) {
            return;
        }
        let attrs = eval_outer_attrs(st, call_node, self.db, self.krate);
        if !attrs.enabled {
            return;
        }
        let raw_call = st.text(path_node).trim();
        let expanded = if !raw_call.contains("::") {
            let short = norm_ident(raw_call);
            if let Some(&(_, idx, _, _)) = macro_env.iter().rfind(|(n, _, _, _)| n == short) {
                let (_, body, cache) = &mut self.macro_defs[idx];
                if cache.is_none() {
                    *cache = MacroRulesDef::parse_body(body);
                }
                cache.as_ref().and_then(|d| d.expand(inner, "crate"))
            } else {
                self.db.macro_rules_body(self.krate, self.mod_tree.crate_path(mod_id), raw_call).and_then(MacroRulesDef::parse_body).and_then(|d| d.expand(inner, "crate"))
            }
        } else {
            self.db.macro_rules_body(self.krate, self.mod_tree.crate_path(mod_id), raw_call).and_then(MacroRulesDef::parse_body).and_then(|d| d.expand(inner, "crate"))
        };
        if let Some(exp) = expanded {
            let exp_st = parse(&exp);
            self.scan_items(mod_id, file, &exp_st, exp_st.root(), macro_env, mbe_depth + 1);
        }
    }

    pub(super) fn supplement_indexed_defs(&mut self, mod_id: ModuleId, file: FileId, item_mark: usize) {
        if self.scratch_idx_defs.is_empty() {
            return;
        }
        let extras: Vec<(GlobalDefId, String, ItemKind, VisKind, bool)> = self
            .scratch_idx_defs
            .iter()
            .filter(|d| !matches!(d.kind, ItemKind::Mod | ItemKind::Use | ItemKind::Impl | ItemKind::Field))
            .filter(|d| {
                let df = self.db.def_file(d.id);
                !(df.is_some() && self.dirty_files.contains(&df))
            })
            .filter(|d| {
                !self.out.items[item_mark..].iter().any(|it| {
                    it.module == mod_id
                        && it.parent_enum == LocalItemId::NONE
                        && &self.out.strings[it.name_start as usize..it.name_end as usize] == d.name
                        && it.kind.ns_mask(it.has_value_ctor).0 & d.kind.ns_mask(d.has_value_ctor).0 != 0
                })
            })
            .map(|d| (d.id, d.name.to_string(), d.kind, d.vis, d.has_value_ctor))
            .collect();
        for (id, name, kind, vis, has_ctor) in extras {
            let (_, vis_mod) = compute_vis_mod(self.mod_tree, mod_id, vis, None);
            self.push_item(mod_id, file, LocalItemId::NONE, id, &name, kind, vis, vis_mod, has_ctor, false, 0, 0);
        }
    }

    fn find_idx_def(&self, name: &str, kind: ItemKind) -> GlobalDefId {
        self.scratch_idx_defs.iter().find(|d| d.name == name && d.kind == kind).map_or(GlobalDefId::NONE, |d| d.id)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn push_item(
        &mut self,
        module: ModuleId,
        file: FileId,
        parent_enum: LocalItemId,
        index_def: GlobalDefId,
        name: &str,
        kind: ItemKind,
        vis: VisKind,
        vis_mod: ModuleId,
        has_value_ctor: bool,
        macro_export: bool,
        span_start: u32,
        span_end: u32,
    ) -> LocalItemId {
        let id = LocalItemId(self.out.items.len() as u32);
        let (name_start, name_end) = self.push_str(name);
        self.out.items.push(LocalItemData {
            id,
            module,
            file,
            parent_enum,
            index_def,
            name_start,
            name_end,
            kind,
            vis,
            vis_mod,
            has_value_ctor,
            macro_export,
            var_start: 0,
            var_len: 0,
            span_start,
            span_end,
        });
        id
    }
}

fn struct_has_value_ctor(st: &SyntaxTree<'_>, struct_node: NodeId) -> bool {
    match st.child(struct_node, K::TokenTree) {
        None => true,
        Some(tt) => !st.text(tt).starts_with('{'),
    }
}

fn parse_enum_variants<'s>(src: &'s str, toks: &[Token], db: &dyn ItemDb, krate: CrateId) -> Vec<(&'s str, bool, u32, u32)> {
    let sig: Vec<Token> = toks.iter().copied().filter(|t| !t.is_trivia()).collect();
    if sig.len() < 2 || sig[0].kind != T::LBrace {
        return Vec::new();
    }
    let body = &sig[1..sig.len().saturating_sub(1)];
    let mut out = Vec::new();
    let mut depth = 0u32;
    let mut start = 0usize;
    for (i, t) in body.iter().enumerate() {
        match t.kind {
            T::LBrace | T::LParen | T::LBracket => depth += 1,
            T::RBrace | T::RParen | T::RBracket => depth = depth.saturating_sub(1),
            T::Comma if depth == 0 => {
                push_variant_slice(src, &body[start..i], db, krate, &mut out);
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < body.len() {
        push_variant_slice(src, &body[start..], db, krate, &mut out);
    }
    out
}

fn push_variant_slice<'s>(src: &'s str, mut slice: &[Token], db: &dyn ItemDb, krate: CrateId, out: &mut Vec<(&'s str, bool, u32, u32)>) {
    let mut attrs = EffectiveAttrs::default();
    while slice.first().map(|t| t.kind) == Some(T::Hash) && slice.get(1).map(|t| t.kind) == Some(T::LBracket) {
        let mut d = 0u32;
        let mut close = None;
        for (off, t) in slice[1..].iter().enumerate() {
            if t.kind == T::LBracket {
                d += 1;
            } else if t.kind == T::RBracket {
                d -= 1;
                if d == 0 {
                    close = Some(1 + off);
                    break;
                }
            }
        }
        let Some(c) = close else { return };
        eval_attr_bracket_tokens(src, &slice[2..c], db, krate, &mut attrs);
        slice = &slice[c + 1..];
    }
    if !attrs.enabled {
        return;
    }
    let Some(&name_tok) = slice.first() else { return };
    if !matches!(name_tok.kind, T::Ident | T::RawIdent) {
        return;
    }
    let has_ctor = slice.get(1).map(|t| t.kind) != Some(T::LBrace);
    out.push((norm_ident(name_tok.text(src)), has_ctor, name_tok.start, name_tok.end));
}
