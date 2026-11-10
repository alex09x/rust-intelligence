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

use super::attr::{eval_inner_attrs, eval_outer_attrs, EffectiveAttrs};
use super::crate_snapshot::CrateSnapshot;
use super::db::{CrateId, FileId, GlobalDefId, ItemDb, ItemKind, ModuleId, VisKind};
use super::def_map::{finalize_in_scope_trait_tables, local_mod_binding, CrateDefMap, ModuleScope};
use super::ext_scope::{indexed_use_to_per_ns, ExtScopeCache};
use super::ident_key::IdentKey;
use super::impl_collect::LocalImplArena;
use super::mod_tree::{ModChild, ModuleData, ModuleTree, MOD_FLAG_INLINE, MOD_FLAG_NO_IMPLICIT_PRELUDE};
use super::per_ns::{compute_vis_mod, is_binding_visible, BindingOrigin, LocalItemId, PerNs, ResolvedBinding};
use super::use_tree::UseArena;
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::tree::{NodeId, SyntaxTree};

#[inline]
fn indexed_vis_mod(tree: &ModuleTree, mod_id: ModuleId, vis: VisKind) -> (VisKind, ModuleId) {
    if vis == VisKind::Restricted {
        (VisKind::Crate, ModuleId::ROOT)
    } else {
        compute_vis_mod(tree, mod_id, vis, None)
    }
}

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
        let mut ext_cache = ExtScopeCache::new();
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
            defs.clear();
            db.module_defs(mod_index_defs[midx], &mut defs);
            for d in &defs {
                if d.name.is_empty() || matches!(d.kind, ItemKind::Mod | ItemKind::Use | ItemKind::Impl | ItemKind::Field) {
                    continue;
                }
                let (v, vmod) = indexed_vis_mod(&mod_tree, mod_id, d.vis);
                let b = ResolvedBinding {
                    def: d.id,
                    local: LocalItemId::NONE,
                    local_mod: ModuleId::NONE,
                    vis_mod: vmod,
                    kind: d.kind,
                    vis: v,
                    origin: BindingOrigin::Explicit,
                    prim_ty: 0,
                    ambiguous: false,
                };
                scopes[midx].entries.entry(IdentKey::new(norm_ident(d.name))).or_insert(PerNs::EMPTY).merge_explicit(PerNs::from_binding(b, d.kind.ns_mask(d.has_value_ctor)));
            }
            uses.clear();
            db.module_uses(mod_index_defs[midx], &mut uses);
            for u in &uses {
                if u.is_prelude {
                    continue;
                }
                let (uvis, vmod) = indexed_vis_mod(&mod_tree, mod_id, u.vis);
                if u.is_glob {
                    if u.type_def.is_some() && u.type_def != mod_index_defs[midx] {
                        globs.push((midx, u.type_def, uvis, vmod));
                    }
                    continue;
                }
                if u.is_underscore {
                    if u.type_def.is_some() && db.def_kind(u.type_def) == Some(ItemKind::Trait) {
                        let tb = ResolvedBinding {
                            def: u.type_def,
                            local: LocalItemId::NONE,
                            local_mod: ModuleId::NONE,
                            vis_mod: vmod,
                            kind: ItemKind::Trait,
                            vis: uvis,
                            origin: BindingOrigin::Explicit,
                            prim_ty: 0,
                            ambiguous: false,
                        };
                        let list = &mut scopes[midx].unnamed_traits;
                        if !list.iter().any(|x| x.same_target(tb)) {
                            list.push(tb);
                        }
                    }
                    continue;
                }
                if u.name.is_empty() {
                    continue;
                }
                let mut pns = indexed_use_to_per_ns(db, u).rebind(uvis, vmod, BindingOrigin::Explicit);
                if let Some(tb) = pns.types.as_mut().filter(|b| b.kind == ItemKind::Mod) {
                    if let Some(&lm) = gid_to_mod.get(&tb.def) {
                        tb.local_mod = lm;
                    } else if midx == 0 && tb.def.krate != krate && db.crate_root_def(tb.def.krate) == Some(tb.def) {
                        extern_prelude.entry(IdentKey::new(norm_ident(u.name))).or_insert((tb.def.krate, tb.def));
                    }
                }
                if !pns.is_empty() {
                    scopes[midx].entries.entry(IdentKey::new(norm_ident(u.name))).or_insert(PerNs::EMPTY).merge_explicit(pns);
                }
            }
        }

        let mut local_globs: Vec<(usize, usize, VisKind, ModuleId)> = Vec::new();
        for &(midx, gdef, gvis, gvmod) in &globs {
            if let Some(&src_m) = gid_to_mod.get(&gdef) {
                local_globs.push((midx, src_m.0 as usize, gvis, gvmod));
            } else {
                let ext_s = ext_cache.module_scope(db, gdef);
                scopes[midx].entries.reserve(ext_s.entries.len());
                for (k, &v) in &ext_s.entries {
                    scopes[midx].entries.entry(k.clone()).or_insert(PerNs::EMPTY).merge_glob(v.rebind(gvis, gvmod, BindingOrigin::Glob));
                }
            }
        }
        for _ in 0..4 {
            let mut changed = false;
            for &(dst, src, gvis, gvmod) in &local_globs {
                if dst == src {
                    continue;
                }
                let dst_id = ModuleId(dst as u32);
                let pairs: Vec<(IdentKey, PerNs)> = scopes[src]
                    .entries
                    .iter()
                    .map(|(k, &v)| {
                        let fv = PerNs {
                            types: v.types.filter(|&b| is_binding_visible(&mod_tree, b, dst_id)),
                            values: v.values.filter(|&b| is_binding_visible(&mod_tree, b, dst_id)),
                            macros: v.macros.filter(|&b| is_binding_visible(&mod_tree, b, dst_id)),
                        };
                        (k.clone(), fv.rebind(gvis, gvmod, BindingOrigin::Glob))
                    })
                    .filter(|(_, fv)| !fv.is_empty())
                    .collect();
                for (k, pns) in pairs {
                    changed |= scopes[dst].entries.entry(k).or_insert(PerNs::EMPTY).merge_glob(pns);
                }
            }
            if !changed {
                break;
            }
        }

        let mut map = CrateDefMap {
            krate,
            edition,
            prelude_mod,
            prelude,
            prelude_traits: Arc::from([]),
            prelude_traits_sorted: Arc::from([]),
            extern_prelude,
            items: Vec::new(),
            item_strings: String::new(),
            uses: UseArena::default(),
            scopes,
            resolved_uses: Vec::new(),
            dirty_files: Vec::new(),
            mod_tree: Arc::new(mod_tree.clone()),
            macro_bodies: Vec::new(),
            local_impls: LocalImplArena::default(),
            local_alias_targets: HashMap::new(),
            diagnostics: Vec::new(),
        };
        finalize_in_scope_trait_tables(&mut map, &mod_tree);
        Some(Self::from_parts(mod_tree, map, ext_cache))
    }

    /// Binds `file` and any inline `mod sub { ... }` declarations inside `st` onto an index-built [`ModuleTree`].
    pub fn bind_indexed_file(&mut self, db: &dyn ItemDb, file: FileId, st: &SyntaxTree<'_>, file_root_mod_path: &str) {
        if self.mod_tree.modules_for_file(file).next().is_some() {
            return;
        }
        let root_mid = self.mod_tree.find_by_path(file_root_mod_path).unwrap_or(ModuleId::ROOT);
        let mut inner = EffectiveAttrs::default();
        eval_inner_attrs(st, st.root(), db, self.mod_tree.krate, &mut inner);
        {
            let m = &mut self.mod_tree.modules[root_mid.0 as usize];
            m.file = file;
            m.decl_end = st.src.len() as u32;
            if inner.no_implicit_prelude {
                m.flags |= MOD_FLAG_NO_IMPLICIT_PRELUDE;
            }
        }
        self.mod_tree.file_to_modules.push((file, root_mid));
        bind_inline_submodules(&mut self.mod_tree, db, file, st, st.root(), root_mid);
        let mods = &self.mod_tree.modules;
        self.mod_tree.file_to_modules.sort_by_key(|&(f, mid)| {
            let md = &mods[mid.0 as usize];
            (f.0, md.is_inline() as u8, md.decl_start)
        });
        if inner.no_implicit_prelude {
            finalize_in_scope_trait_tables(&mut self.clean_def_map, &self.mod_tree);
        }
        self.clean_def_map.mod_tree = Arc::new(self.mod_tree.clone());
    }
}

fn bind_inline_submodules(tree: &mut ModuleTree, db: &dyn ItemDb, file: FileId, st: &SyntaxTree<'_>, container: NodeId, parent_mid: ModuleId) {
    let parent_no_pre = tree.modules[parent_mid.0 as usize].flags & MOD_FLAG_NO_IMPLICIT_PRELUDE;
    for ch in st.children(container) {
        if st.kind(ch) != K::Module {
            continue;
        }
        let Some(il) = st.child(ch, K::ItemList) else { continue };
        let outer = eval_outer_attrs(st, ch, db, tree.krate);
        if !outer.enabled {
            continue;
        }
        let Some(nn) = st.child(ch, K::Name) else { continue };
        let Some(child_mid) = tree.find_child(parent_mid, norm_ident(st.text(nn))) else { continue };
        let mut inner = EffectiveAttrs::default();
        eval_inner_attrs(st, il, db, tree.krate, &mut inner);
        if !inner.enabled {
            continue;
        }
        let r = st.range(ch);
        let cm = &mut tree.modules[child_mid.0 as usize];
        cm.file = file;
        cm.decl_file = file;
        cm.inline_node = ch.0;
        cm.decl_start = r.start as u32;
        cm.decl_end = r.end as u32;
        cm.flags |= MOD_FLAG_INLINE | parent_no_pre | if outer.no_implicit_prelude || inner.no_implicit_prelude { MOD_FLAG_NO_IMPLICIT_PRELUDE } else { 0 };
        tree.file_to_modules.push((file, child_mid));
        bind_inline_submodules(tree, db, file, st, il, child_mid);
    }
}
