/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Incremental dirty-file splicing over [`CollectedCrate`]: reuses pre-collected items, `use`s,
//! and `impl`s of all unchanged `.rs` files and re-scans ONLY modules residing in `dirty_files`.

use std::collections::HashMap;

use super::db::{FileId, ItemDb, ItemKind, ModuleId, SourceDb};
use super::db_impls::LocalImplId;
use super::def_collect::{assign_overlay_item_ids, CollectedCrate, Collector, MacroScopeVec};
use super::impl_collect::LocalImplArena;
use super::mod_tree::ModuleTree;
use super::per_ns::LocalItemId;
use super::use_tree::UseArena;
use crate::parser::parse;
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::tree::{NodeId, SyntaxTree};

impl CollectedCrate {
    pub(super) fn splice_dirty_files<'t>(
        &self,
        db: &dyn ItemDb,
        sources: &dyn SourceDb,
        mod_tree: &ModuleTree,
        dirty_files: &[FileId],
        get_tree: &mut impl FnMut(FileId) -> Option<&'t SyntaxTree<'t>>,
    ) -> (CollectedCrate, u32) {
        let mod_count = mod_tree.modules.len();
        let mut old_to_new = vec![LocalItemId::NONE; self.items.len()];
        let mut items = Vec::with_capacity(self.items.len() + 16);
        for it in &self.items {
            if dirty_files.contains(&it.file) {
                continue;
            }
            let new_id = LocalItemId(items.len() as u32);
            old_to_new[it.id.0 as usize] = new_id;
            let mut copy = *it;
            copy.id = new_id;
            items.push(copy);
        }
        for it in &mut items {
            if it.parent_enum.is_some() {
                it.parent_enum = old_to_new[it.parent_enum.0 as usize];
            }
            if it.kind == ItemKind::Enum && it.var_len > 0 {
                it.var_start = it.id.0 + 1;
            }
        }

        let mut uses = UseArena {
            strings: self.uses.strings.clone(),
            segs: self.uses.segs.clone(),
            uses: Vec::with_capacity(self.uses.uses.len() + 8),
            extern_crates: Vec::with_capacity(self.uses.extern_crates.len()),
        };
        for u in &self.uses.uses {
            let f = mod_tree.module(u.module).file;
            if !dirty_files.contains(&f) {
                uses.uses.push(*u);
            }
        }
        for ec in &self.uses.extern_crates {
            let f = mod_tree.module(ec.module).file;
            if !dirty_files.contains(&f) {
                uses.extern_crates.push(*ec);
            }
        }

        let mut textual_macros = vec![Vec::new(); mod_count];
        for (midx, tlist) in self.textual_macros.iter().enumerate() {
            let f = mod_tree.module(ModuleId(midx as u32)).file;
            if dirty_files.contains(&f) {
                continue;
            }
            for &(lid, gdef) in tlist {
                let mapped = old_to_new.get(lid.0 as usize).copied().unwrap_or(LocalItemId::NONE);
                if mapped.is_some() {
                    textual_macros[midx].push((mapped, gdef));
                }
            }
        }

        let local_impls = copy_clean_local_impls(&self.local_impls, dirty_files, &old_to_new);

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
        let seq = coll.out.next_overlay_seq;
        (coll.out, seq)
    }
}

fn copy_clean_local_impls(src: &LocalImplArena, dirty_files: &[FileId], old_to_new_item: &[LocalItemId]) -> LocalImplArena {
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
        let mapped = old_to_new_item.get(tlid.0 as usize).copied().unwrap_or(LocalItemId::NONE);
        if !mapped.is_some() {
            continue;
        }
        let new_s = dst.assoc_items.len() as u32;
        for ai in &src.assoc_items[a_s as usize..a_s as usize + a_l as usize] {
            let mut c = *ai;
            c.owner_trait = mapped;
            dst.assoc_items.push(c);
        }
        dst.trait_assocs.push((mapped, new_s, a_l));
    }
    for rts in &src.raw_trait_supers {
        let mapped = old_to_new_item.get(rts.trait_item.0 as usize).copied().unwrap_or(LocalItemId::NONE);
        if mapped.is_some() {
            let mut c = *rts;
            c.trait_item = mapped;
            dst.raw_trait_supers.push(c);
        }
    }
    for ta in &src.type_aliases {
        let mapped = old_to_new_item.get(ta.item_id.0 as usize).copied().unwrap_or(LocalItemId::NONE);
        if mapped.is_some() {
            let mut tc = *ta;
            tc.item_id = mapped;
            dst.type_aliases.push(tc);
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
