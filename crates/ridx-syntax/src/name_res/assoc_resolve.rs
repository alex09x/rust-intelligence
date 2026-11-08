/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Cached inherent, trait-item, supertrait, and in-scope trait associated-item resolution
//! methods on [`CrateDefMap`] backed by [`super::assoc_cache::AssocScopeCache`].

use super::db::{GlobalDefId, ItemDb, ItemKind, ModuleId, NsMask, VisKind};
use super::db_impls::{ImplTargetKey, LocalImplId};
use super::def_map::CrateDefMap;
use super::dirty_overlay::{is_indexed_assoc_visible, is_local_assoc_visible, ResolvedAssoc};
use super::ext_scope::ExtScopeCache;
use super::mod_tree::ModuleTree;
use super::per_ns::{LocalItemId, ResolvedBinding};
use crate::sema_fn::norm_ident;

impl CrateDefMap {
    #[allow(clippy::too_many_arguments)]
    pub fn resolve_inherent_assoc_cached(
        &self,
        db: &dyn ItemDb,
        ext_cache: &mut ExtScopeCache,
        mod_tree: &ModuleTree,
        from_mod: ModuleId,
        target: ImplTargetKey,
        raw_name: &str,
        ns: NsMask,
    ) -> Option<ResolvedAssoc> {
        if target == ImplTargetKey::Unknown {
            return None;
        }
        let name = norm_ident(raw_name);
        for limp in &self.local_impls.impls {
            if limp.target != target || limp.trait_def.is_some() || limp.is_negative {
                continue;
            }
            for ai in self.local_impls.impl_assocs(limp) {
                if self.local_impls.assoc_name(ai) == name && ai.kind.ns_mask(false).0 & ns.0 != 0 && is_local_assoc_visible(mod_tree, from_mod, ai.vis, ai.vis_mod) {
                    return Some(ResolvedAssoc {
                        def: ai.def,
                        owner_impl: limp.id,
                        owner_trait: LocalItemId::NONE,
                        kind: ai.kind,
                        vis: ai.vis,
                        vis_mod: ai.vis_mod,
                        has_self_receiver: ai.has_self_receiver,
                    });
                }
            }
        }
        for c in ext_cache.assoc.inherent_items(db, target) {
            if self.is_dirty_file(c.impl_file) {
                continue;
            }
            if c.name.as_str() == name && c.kind.ns_mask(false).0 & ns.0 != 0 && is_indexed_assoc_visible(self.krate, c.impl_krate, c.vis) {
                return Some(ResolvedAssoc {
                    def: c.def,
                    owner_impl: LocalImplId::NONE,
                    owner_trait: LocalItemId::NONE,
                    kind: c.kind,
                    vis: c.vis,
                    vis_mod: ModuleId::NONE,
                    has_self_receiver: c.has_self_receiver,
                });
            }
        }
        None
    }

    pub fn resolve_trait_assoc_cached(&self, db: &dyn ItemDb, ext_cache: &mut ExtScopeCache, trait_def: GlobalDefId, raw_name: &str, ns: NsMask) -> Option<ResolvedAssoc> {
        if !trait_def.is_some() {
            return None;
        }
        let name = norm_ident(raw_name);
        let mut chain = [GlobalDefId::NONE; 16];
        let mut clen = 0usize;
        if let Some(&(tlid, _, _)) = self.local_impls.trait_assocs.iter().find(|&&(tlid, _, _)| self.items[tlid.0 as usize].index_def == trait_def) {
            chain[0] = trait_def;
            clen = 1;
            let mut head = 0usize;
            while head < clen && clen < 16 {
                let cur_def = chain[head];
                head += 1;
                if let Some(&(cur_lid, _, _)) = self.local_impls.trait_assocs.iter().find(|&&(l, _, _)| self.items[l.0 as usize].index_def == cur_def) {
                    for &(slid, sup_def) in &self.local_impls.local_trait_supers {
                        if slid == cur_lid && sup_def.is_some() && !chain[..clen].contains(&sup_def) && clen < 16 {
                            chain[clen] = sup_def;
                            clen += 1;
                        }
                    }
                } else {
                    for &td in ext_cache.assoc.trait_hierarchy(db, cur_def) {
                        if !chain[..clen].contains(&td) && clen < 16 {
                            chain[clen] = td;
                            clen += 1;
                        }
                    }
                }
            }
            let _ = tlid;
        } else {
            for &td in ext_cache.assoc.trait_hierarchy(db, trait_def) {
                if clen < 16 {
                    chain[clen] = td;
                    clen += 1;
                }
            }
        }
        for &td in &chain[..clen] {
            if let Some(&(tlid, a_start, a_len)) = self.local_impls.trait_assocs.iter().find(|&&(tlid, _, _)| self.items[tlid.0 as usize].index_def == td) {
                let s = a_start as usize;
                for ai in &self.local_impls.assoc_items[s..s + a_len as usize] {
                    if self.local_impls.assoc_name(ai) == name && ai.kind.ns_mask(false).0 & ns.0 != 0 {
                        return Some(ResolvedAssoc {
                            def: ai.def,
                            owner_impl: LocalImplId::NONE,
                            owner_trait: tlid,
                            kind: ai.kind,
                            vis: VisKind::Public,
                            vis_mod: ModuleId::NONE,
                            has_self_receiver: ai.has_self_receiver,
                        });
                    }
                }
                continue;
            }
            for c in ext_cache.assoc.trait_decl_items(db, td) {
                if c.name.as_str() == name && c.kind.ns_mask(false).0 & ns.0 != 0 {
                    return Some(ResolvedAssoc {
                        def: c.def,
                        owner_impl: LocalImplId::NONE,
                        owner_trait: LocalItemId::NONE,
                        kind: c.kind,
                        vis: VisKind::Public,
                        vis_mod: ModuleId::NONE,
                        has_self_receiver: c.has_self_receiver,
                    });
                }
            }
        }
        None
    }

    pub fn resolve_trait_impl_assoc_cached(
        &self,
        db: &dyn ItemDb,
        ext_cache: &mut ExtScopeCache,
        target: ImplTargetKey,
        trait_def: GlobalDefId,
        raw_name: &str,
        ns: NsMask,
    ) -> Option<ResolvedAssoc> {
        if matches!(target, ImplTargetKey::Unknown | ImplTargetKey::Param) {
            return self.resolve_trait_assoc_cached(db, ext_cache, trait_def, raw_name, ns);
        }
        let name = norm_ident(raw_name);
        for limp in &self.local_impls.impls {
            if (limp.target == target || limp.target == ImplTargetKey::Param) && limp.trait_def == trait_def && !limp.is_negative {
                for ai in self.local_impls.impl_assocs(limp) {
                    if self.local_impls.assoc_name(ai) == name && ai.kind.ns_mask(false).0 & ns.0 != 0 {
                        return Some(ResolvedAssoc {
                            def: ai.def,
                            owner_impl: limp.id,
                            owner_trait: LocalItemId::NONE,
                            kind: ai.kind,
                            vis: VisKind::Public,
                            vis_mod: ModuleId::NONE,
                            has_self_receiver: ai.has_self_receiver,
                        });
                    }
                }
            }
        }
        for imp in ext_cache.assoc.target_trait_impls(db, target) {
            if imp.trait_def == trait_def && !self.is_dirty_file(imp.file) {
                if let Some(c) = imp.items.iter().find(|c| c.name.as_str() == name && c.kind.ns_mask(false).0 & ns.0 != 0) {
                    return Some(ResolvedAssoc {
                        def: c.def,
                        owner_impl: LocalImplId::NONE,
                        owner_trait: LocalItemId::NONE,
                        kind: c.kind,
                        vis: VisKind::Public,
                        vis_mod: ModuleId::NONE,
                        has_self_receiver: c.has_self_receiver,
                    });
                }
            }
        }
        for imp in ext_cache.assoc.blanket_trait_impls(db, trait_def) {
            if !self.is_dirty_file(imp.file) {
                if let Some(c) = imp.items.iter().find(|c| c.name.as_str() == name && c.kind.ns_mask(false).0 & ns.0 != 0) {
                    return Some(ResolvedAssoc {
                        def: c.def,
                        owner_impl: LocalImplId::NONE,
                        owner_trait: LocalItemId::NONE,
                        kind: c.kind,
                        vis: VisKind::Public,
                        vis_mod: ModuleId::NONE,
                        has_self_receiver: c.has_self_receiver,
                    });
                }
            }
        }
        self.resolve_trait_assoc_cached(db, ext_cache, trait_def, raw_name, ns)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn resolve_in_scope_trait_assoc_cached(
        &self,
        db: &dyn ItemDb,
        ext_cache: &mut ExtScopeCache,
        mod_tree: &ModuleTree,
        from_mod: ModuleId,
        extra_traits: &[ResolvedBinding],
        target: ImplTargetKey,
        raw_name: &str,
        ns: NsMask,
    ) -> Option<(GlobalDefId, ResolvedAssoc)> {
        if !matches!(target, ImplTargetKey::Unknown | ImplTargetKey::Param) {
            let mut cands = [GlobalDefId::NONE; 32];
            let mut clen = 0usize;
            for limp in &self.local_impls.impls {
                if limp.target == target
                    && limp.trait_def.is_some()
                    && !limp.is_negative
                    && clen < 32
                    && !cands[..clen].contains(&limp.trait_def)
                    && self.is_trait_def_in_scope(mod_tree, from_mod, extra_traits, limp.trait_def)
                {
                    cands[clen] = limp.trait_def;
                    clen += 1;
                }
            }
            for imp in ext_cache.assoc.target_trait_impls(db, target) {
                if !self.is_dirty_file(imp.file)
                    && clen < 32
                    && !cands[..clen].contains(&imp.trait_def)
                    && self.is_trait_def_in_scope(mod_tree, from_mod, extra_traits, imp.trait_def)
                {
                    cands[clen] = imp.trait_def;
                    clen += 1;
                }
            }
            for &tdef in &cands[..clen] {
                if let Some(assoc) = self.resolve_trait_impl_assoc_cached(db, ext_cache, target, tdef, raw_name, ns) {
                    return Some((tdef, assoc));
                }
            }
        }
        for tb in extra_traits.iter().chain(self.scope(from_mod).unnamed_traits.iter()) {
            if tb.def.is_some() && self.resolve_trait_assoc_cached(db, ext_cache, tb.def, raw_name, ns).is_some() {
                if let Some(assoc) = self.resolve_trait_impl_assoc_cached(db, ext_cache, target, tb.def, raw_name, ns) {
                    return Some((tb.def, assoc));
                }
            }
        }
        for &pns in self.scope(from_mod).entries.values() {
            if let Some(tb) = pns.filter_visible(mod_tree, from_mod).types.filter(|b| b.kind == ItemKind::Trait && b.def.is_some()) {
                if self.resolve_trait_assoc_cached(db, ext_cache, tb.def, raw_name, ns).is_some() {
                    if let Some(assoc) = self.resolve_trait_impl_assoc_cached(db, ext_cache, target, tb.def, raw_name, ns) {
                        return Some((tb.def, assoc));
                    }
                }
            }
        }
        if !mod_tree.module(from_mod).has_no_implicit_prelude() {
            for pns in self.prelude.values() {
                if let Some(tb) = pns.types.filter(|b| b.kind == ItemKind::Trait && b.def.is_some()) {
                    if self.resolve_trait_assoc_cached(db, ext_cache, tb.def, raw_name, ns).is_some() {
                        if let Some(assoc) = self.resolve_trait_impl_assoc_cached(db, ext_cache, target, tb.def, raw_name, ns) {
                            return Some((tb.def, assoc));
                        }
                    }
                }
            }
        }
        None
    }
}
