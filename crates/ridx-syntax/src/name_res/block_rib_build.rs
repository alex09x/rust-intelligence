/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Lazy builder (`RibStack::push_block_rib`) for inner item & `use` symbol tables inside block expressions.

use super::attr::eval_outer_attrs;
use super::body_ribs::{norm_byte_span, BlockItemRib, RibStack};
use super::db::{FileId, GlobalDefId, ItemDb, ItemKind, ModuleId, NsMask, VisKind};
use super::def_collect_items::{parse_enum_variants, struct_has_value_ctor};
use super::def_map::{extern_crate_binding, CrateDefMap};
use super::def_path::{apply_glob_to_entries, resolve_segments};
use super::ext_scope::ExtScopeCache;
use super::ident_key::IdentKey;
use super::mod_tree::ModuleTree;
use super::per_ns::{BindingOrigin, LocalItemId, PerNs, ResolvedBinding};
use super::use_tree::{FlatUse, FlatUseKind, PathAnchor, UseArena};
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::tree::{NodeId, SyntaxTree};

impl RibStack {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn push_block_rib(
        &mut self,
        map: &CrateDefMap,
        db: &dyn ItemDb,
        mod_tree: &ModuleTree,
        ext_cache: &mut ExtScopeCache,
        file: FileId,
        module: ModuleId,
        st: &SyntaxTree<'_>,
        container: NodeId,
    ) {
        let mut rib = self.block_rib_pool.pop().unwrap_or_default();
        rib.clear();
        let mut use_arena = std::mem::take(&mut self.use_arena_scratch);
        use_arena.clear();
        for ch in st.children(container) {
            let ck = st.kind(ch);
            match ck {
                K::Use => use_arena.lower_use(st, ch, mod_tree, module, db, map.krate),
                K::ExternCrate => use_arena.lower_extern_crate(st, ch, mod_tree, module, db, map.krate),
                K::Fn | K::Struct | K::Union | K::Trait | K::TypeAlias | K::Const | K::Static | K::MacroRules => {
                    if !eval_outer_attrs(st, ch, db, map.krate).enabled {
                        continue;
                    }
                    let Some(nn) = st.child(ch, K::Name) else {
                        continue;
                    };
                    let name = norm_ident(st.text(nn));
                    if name.is_empty() || name == "_" {
                        continue;
                    }
                    let kind = match ck {
                        K::Fn => ItemKind::Fn,
                        K::Struct => ItemKind::Struct,
                        K::Union => ItemKind::Union,
                        K::Trait => ItemKind::Trait,
                        K::TypeAlias => ItemKind::TypeAlias,
                        K::Const => ItemKind::Const,
                        K::Static => ItemKind::Static,
                        _ => ItemKind::Macro,
                    };
                    let (ns, ne) = norm_byte_span(st.src, st.range(nn));
                    let mut def = db.def_at_byte_span(map.krate, file, ns, ne);
                    if !def.is_some() {
                        def = self.alloc_synthetic_def();
                    }
                    let has_ctor = ck == K::Struct && struct_has_value_ctor(st, ch);
                    let b = private_block_binding(def, module, kind);
                    rib.entries.entry(IdentKey::new(name)).or_insert(PerNs::EMPTY).merge_explicit(PerNs::from_binding(b, kind.ns_mask(has_ctor)));
                }
                K::Enum => {
                    if !eval_outer_attrs(st, ch, db, map.krate).enabled {
                        continue;
                    }
                    let Some(nn) = st.child(ch, K::Name) else {
                        continue;
                    };
                    let ename = norm_ident(st.text(nn));
                    if ename.is_empty() {
                        continue;
                    }
                    let (ns, ne) = norm_byte_span(st.src, st.range(nn));
                    let mut edef = db.def_at_byte_span(map.krate, file, ns, ne);
                    if !edef.is_some() {
                        edef = self.alloc_synthetic_def();
                    }
                    let eb = private_block_binding(edef, module, ItemKind::Enum);
                    rib.entries.entry(IdentKey::new(ename)).or_insert(PerNs::EMPTY).merge_explicit(PerNs::from_binding(eb, NsMask::TYPE));
                    if let Some(tt) = st.child(ch, K::TokenTree) {
                        for (vname, has_ctor, vs, ve) in parse_enum_variants(st.src, st.tokens_of(tt), db, map.krate) {
                            let (vns, vne) = if st.src.get(vs as usize..vs as usize + 2) == Some("r#") { (vs + 2, ve) } else { (vs, ve) };
                            let mut vdef = db.def_at_byte_span(map.krate, file, vns, vne);
                            if !vdef.is_some() {
                                vdef = self.alloc_synthetic_def();
                            }
                            let vb = private_block_binding(vdef, module, ItemKind::Variant);
                            rib.enum_variants.push((edef, IdentKey::new(vname), vb, ItemKind::Variant.ns_mask(has_ctor)));
                        }
                    }
                }
                _ => {}
            }
        }
        for ec in &use_arena.extern_crates {
            let orig = if ec.is_self { db.crate_name(map.krate) } else { use_arena.extern_orig(ec) };
            let target_cid = if ec.is_self { Some(map.krate) } else { db.extern_crate(map.krate, orig) };
            if let Some(cid) = target_cid {
                if !ec.is_underscore {
                    let rdef = db.crate_root_def(cid).unwrap_or(GlobalDefId::new(cid, 0));
                    let b = extern_crate_binding(cid, map.krate, rdef);
                    rib.entries.entry(IdentKey::new(use_arena.extern_bind(ec))).or_insert(PerNs::EMPTY).merge_explicit(PerNs::from_binding(b, NsMask::TYPE));
                }
            }
        }
        for u in &use_arena.uses {
            let pns = resolve_block_flat_use(map, db, mod_tree, ext_cache, &rib, &use_arena, u);
            match u.kind {
                FlatUseKind::Named if !pns.is_empty() => {
                    rib.entries.entry(IdentKey::new(use_arena.bind_name(u))).or_insert(PerNs::EMPTY).merge_explicit(pns);
                }
                FlatUseKind::Underscore => {
                    if let Some(tb) = pns.types.filter(|b| b.kind == ItemKind::Trait) {
                        rib.unnamed_traits.push(tb);
                    }
                }
                FlatUseKind::Glob => {
                    apply_glob_to_entries(map, db, mod_tree, ext_cache, u.module, pns.types, &mut rib);
                }
                _ => {}
            }
        }
        use_arena.clear();
        self.use_arena_scratch = use_arena;
        self.block_ribs.push(rib);
    }
}

#[inline]
fn private_block_binding(def: GlobalDefId, vis_mod: ModuleId, kind: ItemKind) -> ResolvedBinding {
    ResolvedBinding {
        def,
        local: LocalItemId::NONE,
        local_mod: ModuleId::NONE,
        vis_mod,
        kind,
        vis: VisKind::Private,
        origin: BindingOrigin::Explicit,
        prim_ty: 0,
        ambiguous: false,
    }
}

fn resolve_block_flat_use(map: &CrateDefMap, db: &dyn ItemDb, mod_tree: &ModuleTree, ext_cache: &mut ExtScopeCache, rib: &BlockItemRib, arena: &UseArena, u: &FlatUse) -> PerNs {
    let spans = arena.seg_spans(u);
    if u.anchor == PathAnchor::Bare && spans.len() == 1 {
        if let Some(&pns) = rib.entries.get(arena.str_span(spans[0])) {
            if !pns.is_empty() {
                return pns;
            }
        }
    }
    if u.anchor == PathAnchor::Bare && spans.len() == 2 {
        let (first, second) = (arena.str_span(spans[0]), arena.str_span(spans[1]));
        if let Some(eb) = rib.entries.get(first).and_then(|p| p.types).filter(|b| b.kind == ItemKind::Enum) {
            for (edef, vname, vb, mask) in &rib.enum_variants {
                if *edef == eb.def && vname.as_str() == second {
                    return PerNs::from_binding(*vb, *mask);
                }
            }
        }
    }
    if spans.len() <= 16 {
        let mut segs: [&str; 16] = [""; 16];
        for (i, &sp) in spans.iter().enumerate() {
            segs[i] = arena.str_span(sp);
        }
        return resolve_segments(map, db, mod_tree, ext_cache, u.module, u.anchor, &segs[..spans.len()], true);
    }
    let vec_segs: Vec<&str> = spans.iter().map(|&sp| arena.str_span(sp)).collect();
    resolve_segments(map, db, mod_tree, ext_cache, u.module, u.anchor, &vec_segs, true)
}
