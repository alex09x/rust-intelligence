/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Compiler-indexed clean [`CrateSnapshot`] constructor (`build_from_index` + per-file inline-module
//! binding) avoiding upfront multi-file disk reads and CST parses on clean `.ridx` `v5` crates.

use std::collections::HashMap;
use std::sync::Arc;

use super::attr::{eval_inner_attrs, EffectiveAttrs};
use super::crate_snapshot::{apply_bind_indexed_file, bind_inline_submodules, CrateSnapshot};
use super::db::{CrateId, FileId, GlobalDefId, ItemDb, ItemKind, ModuleId, VisKind};
use super::def_map::{finalize_in_scope_trait_tables, finalize_module_trait_table, finalize_nonleaf_trait_tables, local_mod_binding, CrateDefMap, ModuleScope};
use super::ext_scope::ExtScopeCache;
use super::ident_key::IdentKey;
use super::impl_collect::LocalImplArena;
use super::mod_tree::{ModChild, ModuleData, ModuleTree};
use super::per_ns::{BindingOrigin, PerNs};
use super::snapshot_indexed_pop::{expand_indexed_globs, indexed_vis_mod, populate_mod_items, ModPopBufs};
use super::use_tree::UseArena;
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::tree::{NodeId, SyntaxTree};

#[inline]
fn blank_mod(id: ModuleId, parent: ModuleId, ns: u32, ne: u32, cs: u32, ce: u32, vis: VisKind) -> ModuleData {
    ModuleData {
        id,
        parent,
        file: FileId::NONE,
        decl_file: FileId::NONE,
        inline_node: u32::MAX,
        decl_start: 0,
        decl_end: 0,
        name_start: ns,
        name_end: ne,
        crate_path_start: cs,
        crate_path_end: ce,
        dir_start: 0,
        dir_end: 0,
        children_start: 0,
        children_len: 0,
        vis,
        flags: 0,
    }
}

impl CrateSnapshot {
    /// Builds a clean [`CrateSnapshot`] directly from compiler-indexed `ItemDb` module definitions
    /// and `UseEntry` tables without reading or parsing untouched `.rs` files in `krate`.
    pub fn build_from_index(db: &dyn ItemDb, krate: CrateId) -> Option<Self> {
        Self::build_from_index_inner(db, krate, ExtScopeCache::new())
    }

    pub(super) fn build_from_index_inner(db: &dyn ItemDb, krate: CrateId, mut ext_cache: ExtScopeCache) -> Option<Self> {
        let root_def = db.crate_root_def(krate)?;
        let edition = db.crate_edition(krate);
        let mut strings = String::with_capacity(256);
        strings.push_str("crate");
        let mut modules = vec![blank_mod(ModuleId::ROOT, ModuleId::NONE, 0, 0, 0, 5, VisKind::Public)];
        let mut mod_index_defs = vec![root_def];
        let mut gid_to_mod: HashMap<GlobalDefId, ModuleId> = HashMap::new();
        gid_to_mod.insert(root_def, ModuleId::ROOT);
        let mut raw_edges: Vec<(ModuleId, ModChild)> = Vec::new();
        let mut defs = Vec::new();

        let mut idx = 0usize;
        while idx < modules.len() {
            let pid = ModuleId(idx as u32);
            let p_gid = mod_index_defs[idx];
            defs.clear();
            db.module_defs(p_gid, &mut defs);
            let child_mods: Vec<(GlobalDefId, String, VisKind)> = defs
                .iter()
                .filter(|d| d.kind == ItemKind::Mod && !d.name.is_empty())
                .map(|d| (d.id, norm_ident(d.name).to_string(), if d.vis == VisKind::Restricted { VisKind::Crate } else { d.vis }))
                .collect();
            for (cgid, cname, cvis) in child_mods {
                let cid = ModuleId(modules.len() as u32);
                let ns = strings.len() as u32;
                strings.push_str(&cname);
                let ne = strings.len() as u32;
                let pm = modules[idx];
                let p_path = strings[pm.crate_path_start as usize..pm.crate_path_end as usize].to_string();
                let cs = strings.len() as u32;
                strings.push_str(&p_path);
                strings.push_str("::");
                strings.push_str(&cname);
                let ce = strings.len() as u32;
                modules.push(blank_mod(cid, pid, ns, ne, cs, ce, cvis));
                mod_index_defs.push(cgid);
                gid_to_mod.insert(cgid, cid);
                raw_edges.push((pid, ModChild { child: cid, name_start: ns, name_end: ne, vis: cvis }));
            }
            idx += 1;
        }

        let mod_count = modules.len();
        let mut children = Vec::new();
        if !raw_edges.is_empty() {
            let mut offsets = vec![0u32; mod_count];
            for &(p, _) in &raw_edges {
                offsets[p.0 as usize] += 1;
            }
            let mut running = 0u32;
            for (m, slot) in offsets.iter_mut().enumerate() {
                let c = *slot;
                modules[m].children_start = running;
                modules[m].children_len = c;
                *slot = running;
                running += c;
            }
            children.resize(running as usize, ModChild { child: ModuleId::NONE, name_start: 0, name_end: 0, vis: VisKind::Private });
            for (p, edge) in raw_edges {
                let slot = &mut offsets[p.0 as usize];
                children[*slot as usize] = edge;
                *slot += 1;
            }
        }

        let mod_tree = ModuleTree { krate, edition, no_std: false, no_core: false, modules, children, file_to_modules: Vec::new(), strings, diagnostics: Vec::new() };
        let mut scopes: Vec<ModuleScope> = mod_index_defs.iter().map(|&index_def| ModuleScope { index_def, ..ModuleScope::default() }).collect();

        let mut extern_prelude: HashMap<IdentKey, (CrateId, GlobalDefId)> = HashMap::new();
        let mut raw_ext = Vec::new();
        db.extern_crates(krate, &mut raw_ext);
        for (ename, cid) in raw_ext {
            let rdef = db.crate_root_def(cid).unwrap_or(GlobalDefId::new(cid, 0));
            extern_prelude.insert(IdentKey::new(norm_ident(ename)), (cid, rdef));
        }
        for builtin_ext in ["core", "std", "alloc", "proc_macro", "test"] {
            if let Some(cid) = db.extern_crate(krate, builtin_ext) {
                let rdef = db.crate_root_def(cid).unwrap_or(GlobalDefId::new(cid, 0));
                extern_prelude.entry(IdentKey::new(builtin_ext)).or_insert((cid, rdef));
            }
        }

        let prelude_mod = ext_cache.resolve_prelude_mod(db, &mod_tree);
        let mut prelude: HashMap<IdentKey, PerNs> = HashMap::new();
        if let Some(pmod) = prelude_mod {
            for (k, &pns) in &ext_cache.module_scope(db, pmod).entries {
                prelude.insert(k.clone(), pns.rebind(VisKind::Public, ModuleId::NONE, BindingOrigin::Prelude));
            }
        }

        let mut uses = Vec::new();
        let mut globs: Vec<(usize, GlobalDefId, VisKind, ModuleId)> = Vec::new();
        for midx in 0..mod_count {
            let mod_id = ModuleId(midx as u32);
            for edge in mod_tree.children(mod_id) {
                let cname = IdentKey::new(mod_tree.child_name(edge));
                let (v, vmod) = indexed_vis_mod(&mod_tree, mod_id, edge.vis);
                let cdef = scopes[edge.child.0 as usize].index_def;
                scopes[midx].entries.entry(cname).or_insert(PerNs::EMPTY).merge_explicit(local_mod_binding(edge.child, cdef, v, vmod));
            }
            let is_leaf = midx != 0 && mod_tree.children(mod_id).is_empty();
            let mut bufs = ModPopBufs { scopes: &mut scopes, extern_prelude: &mut extern_prelude, globs: &mut globs, defs: &mut defs, uses: &mut uses };
            populate_mod_items(db, &mod_tree, krate, midx, is_leaf, false, &gid_to_mod, &mut bufs);
        }
        expand_indexed_globs(db, &mod_tree, &gid_to_mod, &globs, &mut ext_cache, &mut scopes);

        let (empty_traits, mod_tree_arc): (Arc<[GlobalDefId]>, _) = (Arc::from([]), Arc::new(mod_tree.clone()));
        let mut map = CrateDefMap {
            krate,
            edition,
            prelude_mod,
            prelude,
            prelude_traits: Arc::clone(&empty_traits),
            prelude_traits_sorted: empty_traits,
            extern_prelude,
            items: Vec::new(),
            item_strings: String::new(),
            uses: UseArena::default(),
            scopes,
            resolved_uses: Vec::new(),
            dirty_files: Vec::new(),
            indexed_clean: true,
            mod_tree: mod_tree_arc,
            macro_bodies: Vec::new(),
            local_impls: LocalImplArena::default(),
            local_alias_targets: HashMap::new(),
            diagnostics: Vec::new(),
        };
        finalize_nonleaf_trait_tables(&mut map, &mod_tree);
        Some(Self::from_parts(mod_tree, map, ext_cache))
    }

    /// Binds `file` and any inline `mod sub { ... }` declarations inside `st` onto an index-built [`ModuleTree`].
    pub fn bind_indexed_file(&mut self, db: &dyn ItemDb, file: FileId, st: &SyntaxTree<'_>, file_root_mod_path: &str) {
        self.bind_indexed_file_at(db, file, st, file_root_mod_path, None);
    }

    /// Binds `file` (and any inline modules needed for `byte_offset`) onto an index-built [`ModuleTree`].
    pub fn bind_indexed_file_at(&mut self, db: &dyn ItemDb, file: FileId, st: &SyntaxTree<'_>, file_root_mod_path: &str, byte_offset: Option<u32>) {
        let root_mid = self.mod_tree.modules_for_file(file).next().unwrap_or_else(|| self.mod_tree.find_by_path(file_root_mod_path).unwrap_or(ModuleId::ROOT));
        if self.mod_tree.modules_for_file(file).next().is_none() {
            let mut inner = EffectiveAttrs::default();
            eval_inner_attrs(st, st.root(), db, self.mod_tree.krate, &mut inner);
            apply_bind_indexed_file(&mut self.mod_tree, db, file, st, root_mid, inner.no_implicit_prelude);
            let mt = Arc::make_mut(&mut self.clean_def_map.mod_tree);
            mt.modules.clone_from(&self.mod_tree.modules);
            mt.file_to_modules.clone_from(&self.mod_tree.file_to_modules);
            if inner.no_implicit_prelude && root_mid == ModuleId::ROOT {
                finalize_in_scope_trait_tables(&mut self.clean_def_map, &self.mod_tree);
            }
        } else if self.mod_tree.children(root_mid).iter().any(|e| self.mod_tree.modules[e.child.0 as usize].file == FileId::NONE)
            && st.children(st.root()).any(|c| st.kind(c) == K::Module)
        {
            bind_inline_submodules(&mut self.mod_tree, db, file, st, st.root(), root_mid);
            let mods = &self.mod_tree.modules;
            self.mod_tree.file_to_modules.sort_by_key(|&(f, mid)| (f.0, mods[mid.0 as usize].is_inline() as u8, mods[mid.0 as usize].decl_start));
            let mt = Arc::make_mut(&mut self.clean_def_map.mod_tree);
            mt.modules.clone_from(&self.mod_tree.modules);
            mt.file_to_modules.clone_from(&self.mod_tree.file_to_modules);
        }
        let (empty_gid, mut dummy_ep) = (HashMap::new(), HashMap::new());
        let (mut globs, mut defs, mut uses, mut named_buf) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let mut ext_cache = self.ext_cache.borrow_mut();
        for &(f, mid) in &self.mod_tree.file_to_modules {
            let midx = mid.0 as usize;
            if f != file || self.clean_def_map.scopes[midx].visible_traits_hash != 0 {
                continue;
            }
            let md = &self.mod_tree.modules[midx];
            if md.is_inline() && byte_offset.is_some_and(|off| off < md.decl_start || off > md.decl_end) {
                continue;
            }
            if md.is_inline()
                && byte_offset.is_none()
                && !st.nodes.iter().enumerate().any(|(i, n)| {
                    n.kind == K::Module
                        && st.range(NodeId(i as u32)).start as u32 == md.decl_start
                        && st.child(NodeId(i as u32), K::ItemList).is_some_and(|il| st.children(il).next().is_some())
                })
            {
                continue;
            }
            globs.clear();
            let mut bufs = ModPopBufs { scopes: &mut self.clean_def_map.scopes, extern_prelude: &mut dummy_ep, globs: &mut globs, defs: &mut defs, uses: &mut uses };
            populate_mod_items(db, &self.mod_tree, self.mod_tree.krate, midx, false, true, &empty_gid, &mut bufs);
            expand_indexed_globs(db, &self.mod_tree, &empty_gid, &globs, &mut ext_cache, &mut self.clean_def_map.scopes);
            finalize_module_trait_table(&mut self.clean_def_map.scopes[midx], &self.mod_tree, mid, &mut named_buf);
        }
    }
}
