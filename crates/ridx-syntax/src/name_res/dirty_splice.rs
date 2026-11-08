/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Incremental dirty-file splicing over [`CollectedCrate`]: tombstones dirty-file items in place
//! (keeping every clean [`LocalItemId`] identical) and re-scans ONLY modules residing in `dirty_files`.

use std::collections::HashMap;

use super::db::{FileId, GlobalDefId, ItemDb, ItemKind, ModuleId, SourceDb};
use super::db_impls::LocalImplId;
use super::def_collect::{assign_overlay_item_ids, CollectedCrate, Collector, LocalItemData, MacroScopeVec};
use super::impl_collect::LocalImplArena;
use super::mod_tree::ModuleTree;
use super::per_ns::LocalItemId;
use super::use_tree::UseArena;
use crate::parser::parse;
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::tree::{NodeId, SyntaxTree};

pub(super) struct SplicedCrate {
    pub(super) collected: CollectedCrate,
    /// Maps retained clean `FlatUse` slot `0..clean_use_map.len()` to its index in `clean_collected.uses.uses`.
    pub(super) clean_use_map: Vec<u32>,
    pub(super) next_overlay_seq: u32,
}

impl CollectedCrate {
    pub(super) fn splice_dirty_files<'t>(
        &self,
        db: &dyn ItemDb,
        sources: &dyn SourceDb,
        mod_tree: &ModuleTree,
        dirty_files: &[FileId],
        get_tree: &mut impl FnMut(FileId) -> Option<&'t SyntaxTree<'t>>,
    ) -> SplicedCrate {
        let mod_count = mod_tree.modules.len();
        let mut items = Vec::with_capacity(self.items.len() + 16);
        for it in &self.items {
            if dirty_files.contains(&it.file) {
                let mut dead = *it;
                dead.file = FileId::NONE;
                dead.module = ModuleId::NONE;
                dead.index_def = GlobalDefId::NONE;
                dead.parent_enum = LocalItemId::NONE;
                dead.var_len = 0;
                dead.macro_export = false;
                items.push(dead);
            } else {
                items.push(*it);
            }
        }

        let mut uses = UseArena {
            strings: self.uses.strings.clone(),
            segs: self.uses.segs.clone(),
            uses: Vec::with_capacity(self.uses.uses.len() + 8),
            extern_crates: Vec::with_capacity(self.uses.extern_crates.len()),
        };
        let mut clean_use_map = Vec::with_capacity(self.uses.uses.len());
        for (old_idx, u) in self.uses.uses.iter().enumerate() {
            let f = mod_tree.module(u.module).file;
            if !dirty_files.contains(&f) {
                uses.uses.push(*u);
                clean_use_map.push(old_idx as u32);
            }
        }
        for ec in &self.uses.extern_crates {
            let f = mod_tree.module(ec.module).file;
            if !dirty_files.contains(&f) {
                uses.extern_crates.push(*ec);
            }
        }

        let clean_items_len = self.items.len();
        let mut textual_macros = vec![Vec::new(); mod_count];
        let mut pending_dirty_textual: Vec<(usize, ModuleId, &str)> = Vec::new();
        for (midx, tlist) in self.textual_macros.iter().enumerate() {
            for &(lid, gdef) in tlist {
                let src_it = &self.items[lid.0 as usize];
                if !dirty_files.contains(&src_it.file) {
                    textual_macros[midx].push((lid, gdef));
                } else if src_it.module.is_some() {
                    let mname = &self.strings[src_it.name_start as usize..src_it.name_end as usize];
                    pending_dirty_textual.push((midx, src_it.module, mname));
                }
            }
        }

        let local_impls = copy_clean_local_impls(&self.local_impls, &self.items, dirty_files);

        let mut owned_dirty: HashMap<FileId, SyntaxTree<'_>> = HashMap::new();
        for &df in dirty_files {
            if df.is_some() && get_tree(df).is_none() {
                owned_dirty.entry(df).or_insert_with(|| parse(sources.file_text(df)));
            }
        }

        let mut coll = Collector {
            db,
            krate: mod_tree.krate,
            mod_tree,
            dirty_files,
            scan_all_impls: self.scan_all_impls,
            out: CollectedCrate {
                mod_index_defs: self.mod_index_defs.clone(),
                items,
                strings: self.strings.clone(),
                uses,
                textual_macros,
                local_impls,
                scan_all_impls: self.scan_all_impls,
                next_overlay_seq: self.next_overlay_seq,
            },
            macro_defs: Vec::new(),
            scratch_idx_defs: Vec::new(),
            scratch_var_defs: Vec::new(),
            scratch_toks: Vec::with_capacity(64),
            scratch_prefix: Vec::with_capacity(16),
        };

        let mut macro_env = MacroScopeVec::new();
        for m in &mod_tree.modules {
            if !m.file.is_some() || m.is_unresolved() || !dirty_files.contains(&m.file) {
                continue;
            }
            let st: &SyntaxTree<'_> = match get_tree(m.file) {
                Some(t) => t,
                None => &owned_dirty[&m.file],
            };
            let Some(container) = find_module_container_in_file(mod_tree, m.id, st) else {
                continue;
            };
            let mod_def = coll.out.mod_index_defs[m.id.0 as usize];
            coll.scratch_idx_defs.clear();
            if mod_def.is_some() {
                db.module_defs(mod_def, &mut coll.scratch_idx_defs);
            }
            macro_env.clear();
            coll.scan_items(m.id, m.file, st, container, &mut macro_env, 0);
        }

        assign_overlay_item_ids(&mut coll.out);
        for (midx, src_mod, mname) in pending_dirty_textual {
            if let Some(found) = coll.out.items[clean_items_len..]
                .iter()
                .rev()
                .find(|it| it.module == src_mod && it.kind == ItemKind::Macro && &coll.out.strings[it.name_start as usize..it.name_end as usize] == mname)
            {
                coll.out.textual_macros[midx].push((found.id, found.index_def));
            }
        }
        let seq = coll.out.next_overlay_seq;
        SplicedCrate { collected: coll.out, clean_use_map, next_overlay_seq: seq }
    }
}

fn copy_clean_local_impls(src: &LocalImplArena, clean_items: &[LocalItemData], dirty_files: &[FileId]) -> LocalImplArena {
    if src.impls.is_empty() && src.trait_assocs.is_empty() && src.type_aliases.is_empty() {
        return LocalImplArena::default();
    }
    let mut dst = LocalImplArena {
        strings: src.strings.clone(),
        segs: src.segs.clone(),
        generic_params: src.generic_params.clone(),
        impls: Vec::with_capacity(src.impls.len() + 4),
        assoc_items: Vec::with_capacity(src.assoc_items.len() + 8),
        trait_assocs: Vec::with_capacity(src.trait_assocs.len()),
        raw_trait_supers: Vec::with_capacity(src.raw_trait_supers.len()),
        local_trait_supers: Vec::new(),
        type_aliases: Vec::with_capacity(src.type_aliases.len()),
    };
    for imp in &src.impls {
        if dirty_files.contains(&imp.file) {
            continue;
        }
        let new_id = LocalImplId(dst.impls.len() as u32);
        let a_start = dst.assoc_items.len() as u32;
        for ai in src.impl_assocs(imp) {
            let mut c = *ai;
            c.owner_impl = new_id;
            dst.assoc_items.push(c);
        }
        let mut ic = *imp;
        ic.id = new_id;
        ic.assoc_start = a_start;
        dst.impls.push(ic);
    }
    for &(tlid, a_s, a_l) in &src.trait_assocs {
        if dirty_files.contains(&clean_items[tlid.0 as usize].file) {
            continue;
        }
        let new_s = dst.assoc_items.len() as u32;
        for ai in &src.assoc_items[a_s as usize..a_s as usize + a_l as usize] {
            dst.assoc_items.push(*ai);
        }
        dst.trait_assocs.push((tlid, new_s, a_l));
    }
    for rts in &src.raw_trait_supers {
        if !dirty_files.contains(&clean_items[rts.trait_item.0 as usize].file) {
            dst.raw_trait_supers.push(*rts);
        }
    }
    for ta in &src.type_aliases {
        if !dirty_files.contains(&clean_items[ta.item_id.0 as usize].file) {
            dst.type_aliases.push(*ta);
        }
    }
    dst
}

pub(super) fn find_module_container_in_file(tree: &ModuleTree, mod_id: ModuleId, st: &SyntaxTree<'_>) -> Option<NodeId> {
    let m = tree.module(mod_id);
    if !m.is_inline() {
        return Some(st.root());
    }
    let parent_cont = find_module_container_in_file(tree, m.parent, st)?;
    let target_name = tree.name(mod_id);
    for ch in st.children(parent_cont) {
        if st.kind(ch) == K::Module {
            if let Some(nn) = st.child(ch, K::Name) {
                if norm_ident(st.text(nn)) == target_name {
                    return st.child(ch, K::ItemList);
                }
            }
        }
    }
    None
}
