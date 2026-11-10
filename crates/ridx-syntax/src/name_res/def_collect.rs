/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Module-tree CST walker for [`super::def_map::CrateDefMap`] (per-item extraction helpers live in
//! [`super::def_collect_items`]).

use super::attr::eval_outer_attrs;
use super::db::{CrateId, FileId, GlobalDefId, IndexedDef, ItemDb, ItemKind, ModuleId, SourceDb, VisKind};
use super::impl_collect::LocalImplArena;
use super::mbe_expand::MAX_MBE_DEPTH;
use super::mbe_parse::MacroRulesDef;
use super::mod_tree::{ModuleTree, MOD_FLAG_MACRO_USE};
use super::per_ns::LocalItemId;
use super::use_tree::UseArena;
use crate::parser::parse;
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::token::Token;
use crate::tree::{NodeId, SyntaxTree};

/// Compact descriptor of one locally defined item (or enum variant) inside [`super::def_map::CrateDefMap`].
#[derive(Clone, Copy, Debug)]
pub struct LocalItemData {
    pub id: LocalItemId,
    pub module: ModuleId,
    pub file: FileId,
    /// Owning `LocalItemId` when `kind == ItemKind::Variant`, otherwise `LocalItemId::NONE`.
    pub parent_enum: LocalItemId,
    /// Matched compiler `GlobalDefId` from `ItemDb` (`GlobalDefId::NONE` for brand-new unindexed items).
    pub index_def: GlobalDefId,
    pub(super) name_start: u32,
    pub(super) name_end: u32,
    pub kind: ItemKind,
    pub vis: VisKind,
    pub vis_mod: ModuleId,
    pub has_value_ctor: bool,
    pub macro_export: bool,
    pub var_start: u32,
    pub var_len: u16,
    pub span_start: u32,
    pub span_end: u32,
}

#[derive(Clone, Debug)]
pub(super) struct CollectedCrate {
    pub mod_index_defs: Vec<GlobalDefId>,
    pub items: Vec<LocalItemData>,
    pub strings: String,
    pub uses: UseArena,
    /// Per-module `#[macro_use]` / local textual macros visible inside that module.
    pub textual_macros: Vec<Vec<(LocalItemId, GlobalDefId)>>,
    pub macro_bodies: Vec<(LocalItemId, u32, u32)>,
    pub local_impls: LocalImplArena,
    pub scan_all_impls: bool,
    pub next_overlay_seq: u32,
}

pub(super) type MacroScopeVec = Vec<(String, usize, LocalItemId, GlobalDefId)>;

pub(super) fn collect_crate_items<'t>(
    db: &dyn ItemDb,
    sources: &dyn SourceDb,
    mod_tree: &ModuleTree,
    dirty_files: &[FileId],
    mut get_tree: impl FnMut(FileId) -> Option<&'t SyntaxTree<'t>>,
) -> CollectedCrate {
    let mod_count = mod_tree.modules.len();
    let mod_index_defs = match_module_index_defs(db, mod_tree);
    let scan_all_impls = {
        let mut ci = Vec::new();
        db.crate_impls(mod_tree.krate, &mut ci);
        ci.is_empty() && db.crate_root_def(mod_tree.krate).is_none()
    };

    let mut coll = Collector {
        db,
        sources,
        krate: mod_tree.krate,
        mod_tree,
        dirty_files,
        scan_all_impls,
        out: CollectedCrate {
            mod_index_defs,
            items: Vec::with_capacity(mod_count * 8),
            strings: String::with_capacity(mod_count * 64),
            uses: UseArena::with_capacity(mod_count * 4),
            textual_macros: vec![Vec::new(); mod_count],
            macro_bodies: Vec::new(),
            local_impls: LocalImplArena::default(),
            scan_all_impls,
            next_overlay_seq: 0,
        },
        macro_defs: Vec::new(),
        in_flight_includes: Vec::new(),
        scratch_idx_defs: Vec::new(),
        scratch_var_defs: Vec::new(),
        scratch_toks: Vec::with_capacity(64),
        scratch_prefix: Vec::with_capacity(16),
    };

    let mut macro_env = MacroScopeVec::new();
    coll.walk_module(ModuleId::ROOT, &mut get_tree, None, &mut macro_env);
    assign_overlay_item_ids(&mut coll.out);
    coll.out
}

pub(super) fn assign_overlay_item_ids(out: &mut CollectedCrate) {
    let mut seq = out.next_overlay_seq;
    for it in &mut out.items {
        if it.module.is_some() && !it.index_def.is_some() {
            it.index_def = GlobalDefId::new(CrateId::LOCAL_OVERLAY, seq);
            seq += 1;
        }
    }
    out.next_overlay_seq = seq;
}

fn match_module_index_defs(db: &dyn ItemDb, tree: &ModuleTree) -> Vec<GlobalDefId> {
    let n = tree.modules.len();
    let mut out = vec![GlobalDefId::NONE; n];
    if n == 0 {
        return out;
    }
    let Some(root_def) = db.crate_root_def(tree.krate) else { return out };
    out[ModuleId::ROOT.0 as usize] = root_def;
    let mut scratch = Vec::new();
    for idx in 0..n {
        let m_id = ModuleId(idx as u32);
        let p_def = out[idx];
        if !p_def.is_some() || tree.children(m_id).is_empty() {
            continue;
        }
        scratch.clear();
        db.module_defs(p_def, &mut scratch);
        for edge in tree.children(m_id) {
            let cname = tree.child_name(edge);
            if let Some(found) = scratch.iter().find(|d| d.kind == ItemKind::Mod && d.name == cname) {
                out[edge.child.0 as usize] = found.id;
            }
        }
    }
    out
}

pub(super) struct Collector<'a> {
    pub(super) db: &'a dyn ItemDb,
    pub(super) sources: &'a dyn SourceDb,
    pub(super) krate: CrateId,
    pub(super) mod_tree: &'a ModuleTree,
    pub(super) dirty_files: &'a [FileId],
    pub(super) scan_all_impls: bool,
    pub(super) out: CollectedCrate,
    pub(super) macro_defs: Vec<(String, String, Option<MacroRulesDef>)>,
    pub(super) in_flight_includes: Vec<FileId>,
    pub(super) scratch_idx_defs: Vec<IndexedDef<'a>>,
    pub(super) scratch_var_defs: Vec<IndexedDef<'a>>,
    pub(super) scratch_toks: Vec<Token>,
    pub(super) scratch_prefix: Vec<(u32, u32)>,
}

impl<'a> Collector<'a> {
    #[inline]
    pub(super) fn push_str(&mut self, s: &str) -> (u32, u32) {
        let start = self.out.strings.len() as u32;
        self.out.strings.push_str(s);
        (start, self.out.strings.len() as u32)
    }

    #[inline]
    pub(super) fn should_scan_impls(&self, file: FileId) -> bool {
        self.scan_all_impls || self.dirty_files.contains(&file)
    }

    fn walk_module<'t>(
        &mut self,
        mod_id: ModuleId,
        get_tree: &mut impl FnMut(FileId) -> Option<&'t SyntaxTree<'t>>,
        parent_file_tree: Option<(FileId, &SyntaxTree<'_>)>,
        macro_env: &mut MacroScopeVec,
    ) {
        let m = *self.mod_tree.module(mod_id);
        if !m.file.is_some() || m.is_unresolved() {
            return;
        }
        // Snapshot inherited textual `#[macro_use]` macros active upon entering `mod_id`.
        for &(_, _, lid, gdef) in macro_env.iter() {
            self.out.textual_macros[mod_id.0 as usize].push((lid, gdef));
        }
        let local_tree;
        let st: &SyntaxTree<'_> = if let Some(t) = get_tree(m.file) {
            t
        } else if let Some((pf, pt)) = parent_file_tree.filter(|&(pf, _)| pf == m.file) {
            let _ = pf;
            pt
        } else {
            local_tree = parse(self.sources.file_text(m.file));
            &local_tree
        };
        let mod_def = self.out.mod_index_defs[mod_id.0 as usize];
        self.scratch_idx_defs.clear();
        if mod_def.is_some() {
            self.db.module_defs(mod_def, &mut self.scratch_idx_defs);
        }
        let item_mark = self.out.items.len();
        if m.is_inline() {
            if m.inline_node != u32::MAX {
                if let Some(il) = st.child(NodeId(m.inline_node), K::ItemList) {
                    self.scan_items(mod_id, m.file, st, il, macro_env, 0);
                }
            }
        } else {
            self.scan_items(mod_id, m.file, st, st.root(), macro_env, 0);
        }

        if mod_def.is_some() && !self.dirty_files.contains(&m.file) {
            let start_mark = if m.is_inline() && m.inline_node == u32::MAX { 0 } else { item_mark };
            self.supplement_indexed_defs(mod_id, m.file, start_mark);
        }

        let child_len = self.mod_tree.children(mod_id).len();
        for cidx in 0..child_len {
            let child_id = self.mod_tree.children(mod_id)[cidx].child;
            let env_mark = macro_env.len();
            self.walk_module(child_id, get_tree, Some((m.file, st)), macro_env);
            if (self.mod_tree.module(child_id).flags & MOD_FLAG_MACRO_USE) != 0 {
                for &(_, _, lid, gdef) in &macro_env[env_mark..] {
                    self.out.textual_macros[mod_id.0 as usize].push((lid, gdef));
                }
            } else {
                macro_env.truncate(env_mark);
            }
        }
    }

    pub(super) fn scan_items(&mut self, mod_id: ModuleId, file: FileId, st: &SyntaxTree<'_>, container: NodeId, macro_env: &mut MacroScopeVec, mbe_depth: u32) {
        for node in st.children(container) {
            match st.kind(node) {
                K::Use => self.out.uses.lower_use_into(st, node, self.mod_tree, mod_id, self.db, self.krate, &mut self.scratch_toks, &mut self.scratch_prefix),
                K::ExternCrate => self.out.uses.lower_extern_crate(st, node, self.mod_tree, mod_id, self.db, self.krate),
                K::Struct => {
                    self.collect_named_item(mod_id, file, st, node, ItemKind::Struct);
                }
                K::Enum => self.collect_enum_item(mod_id, file, st, node),
                K::Union => {
                    self.collect_named_item(mod_id, file, st, node, ItemKind::Union);
                }
                K::Trait => self.collect_trait_item(mod_id, file, st, node, macro_env),
                K::Fn => {
                    self.collect_named_item(mod_id, file, st, node, ItemKind::Fn);
                }
                K::TypeAlias => self.collect_type_alias_item(mod_id, file, st, node),
                K::Impl if self.should_scan_impls(file) => {
                    self.out.local_impls.collect_impl_node(st, node, self.mod_tree, mod_id, file, self.db, self.sources, self.krate, &mut self.macro_defs, macro_env);
                }
                K::Const => {
                    self.collect_named_item(mod_id, file, st, node, ItemKind::Const);
                }
                K::Static => {
                    self.collect_named_item(mod_id, file, st, node, ItemKind::Static);
                }
                K::Module if mbe_depth > 0 => {
                    self.collect_mbe_inline_module(mod_id, file, st, node, macro_env, mbe_depth);
                }
                K::ExternBlock => {
                    let attrs = eval_outer_attrs(st, node, self.db, self.krate);
                    if attrs.enabled {
                        if let Some(il) = st.child(node, K::ItemList) {
                            self.scan_items(mod_id, file, st, il, macro_env, mbe_depth);
                        }
                    }
                }
                K::MacroRules => self.collect_macro_rules(mod_id, file, st, node, macro_env),
                K::MacroCall if mbe_depth < MAX_MBE_DEPTH => {
                    self.try_expand_scope_macro(mod_id, file, st, node, macro_env, mbe_depth);
                }
                _ => {}
            }
        }
    }

    fn collect_mbe_inline_module(&mut self, parent_mod: ModuleId, file: FileId, st: &SyntaxTree<'_>, mod_node: NodeId, macro_env: &mut MacroScopeVec, mbe_depth: u32) {
        let Some(item_list) = st.child(mod_node, K::ItemList) else { return };
        let attrs = eval_outer_attrs(st, mod_node, self.db, self.krate);
        if !attrs.enabled {
            return;
        }
        let Some(name_node) = st.child(mod_node, K::Name) else { return };
        let mod_name = norm_ident(st.text(name_node));
        let Some(edge) = self.mod_tree.children(parent_mod).iter().find(|e| self.mod_tree.child_name(e) == mod_name) else {
            return;
        };
        let child_id = edge.child;
        let saved_idx_defs = std::mem::take(&mut self.scratch_idx_defs);
        let child_def = self.out.mod_index_defs[child_id.0 as usize];
        if child_def.is_some() {
            self.db.module_defs(child_def, &mut self.scratch_idx_defs);
        }
        let env_mark = macro_env.len();
        self.scan_items(child_id, file, st, item_list, macro_env, mbe_depth);
        if !attrs.macro_use {
            macro_env.truncate(env_mark);
        }
        self.scratch_idx_defs = saved_idx_defs;
    }
}
