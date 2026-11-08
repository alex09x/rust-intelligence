/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Merged dirty-file + indexed snapshot queries (`impl` blocks, inherent & trait associated items).

use super::db::{CrateId, FileId, GlobalDefId, IndexedDef, ItemDb, ItemKind, ModuleId, NsMask, VisKind};
use super::db_impls::{ImplTargetKey, IndexedImpl, LocalImplId};
use super::def_map::CrateDefMap;
use super::mod_tree::ModuleTree;
use super::per_ns::{is_descendant_or_eq, LocalItemId, ResolvedBinding};
use crate::sema_fn::norm_ident;

/// Produces a stable [`GlobalDefId`] for both indexed defs and unindexed local CST items.
#[inline]
pub(super) fn effective_local_or_def(def: GlobalDefId, local: LocalItemId) -> GlobalDefId {
    if def.is_some() {
        def
    } else if local.is_some() {
        GlobalDefId::new(CrateId::LOCAL_OVERLAY, 0x8000_0000 | local.0)
    } else {
        GlobalDefId::NONE
    }
}

/// Unified view of either a clean indexed `impl` or a dirty/local CST `impl`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MergedImplRef {
    pub id: GlobalDefId,
    pub local_impl: LocalImplId,
    pub target: ImplTargetKey,
    pub trait_def: GlobalDefId,
    pub is_negative: bool,
}

/// Resolved inherent or trait associated item (`T::f`, `T::CONST`, `Trait::AssocTy`, `<T as Trait>::f`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedAssoc {
    pub def: GlobalDefId,
    pub owner_impl: LocalImplId,
    pub owner_trait: LocalItemId,
    pub kind: ItemKind,
    pub vis: VisKind,
    pub vis_mod: ModuleId,
    pub has_self_receiver: bool,
}

impl CrateDefMap {
    /// Returns `true` if `file` is overridden by a dirty buffer in this `CrateDefMap`.
    #[inline]
    pub fn is_dirty_file(&self, file: FileId) -> bool {
        file.is_some() && self.dirty_files.contains(&file)
    }

    /// Resolves a `TypeAlias` [`GlobalDefId`] (either dirty/local or indexed) to its [`ImplTargetKey`].
    pub fn resolve_alias_target(&self, db: &dyn ItemDb, alias_def: GlobalDefId) -> Option<ImplTargetKey> {
        self.local_alias_targets.get(&alias_def).copied().or_else(|| db.type_alias_target(alias_def))
    }

    /// Appends all active `impl` blocks matching `target` (clean indexed minus `dirty_files` + dirty/local CST).
    pub fn merged_impls_for_target(&self, db: &dyn ItemDb, target: ImplTargetKey, include_blanket_param: bool, out: &mut Vec<MergedImplRef>) {
        if target == ImplTargetKey::Unknown {
            return;
        }
        let mut raw_idx: Vec<IndexedImpl> = Vec::new();
        db.impls_for_target(target, &mut raw_idx);
        if include_blanket_param && target != ImplTargetKey::Param {
            db.impls_for_target(ImplTargetKey::Param, &mut raw_idx);
        }
        for imp in raw_idx {
            if imp.file.is_some() && self.dirty_files.contains(&imp.file) {
                continue;
            }
            out.push(MergedImplRef { id: imp.id, local_impl: LocalImplId::NONE, target: imp.target, trait_def: imp.trait_def, is_negative: imp.is_negative });
        }
        for limp in &self.local_impls.impls {
            if limp.target == target || (include_blanket_param && limp.target == ImplTargetKey::Param) {
                out.push(MergedImplRef { id: limp.index_def, local_impl: limp.id, target: limp.target, trait_def: limp.trait_def, is_negative: limp.is_negative });
            }
        }
    }

    /// Resolves an inherent associated item (`T::name`) across merged clean-indexed + dirty-local `impl` blocks.
    pub fn resolve_inherent_assoc(&self, db: &dyn ItemDb, mod_tree: &ModuleTree, from_mod: ModuleId, target: ImplTargetKey, raw_name: &str, ns: NsMask) -> Option<ResolvedAssoc> {
        let name = norm_ident(raw_name);
        let mut impls = Vec::new();
        self.merged_impls_for_target(db, target, false, &mut impls);
        let mut scratch: Vec<IndexedDef<'_>> = Vec::new();
        for mimp in impls {
            if mimp.trait_def.is_some() || mimp.is_negative {
                continue;
            }
            if mimp.local_impl.is_some() {
                let limp = &self.local_impls.impls[mimp.local_impl.0 as usize];
                for ai in self.local_impls.impl_assocs(limp) {
                    if self.local_impls.assoc_name(ai) == name && ai.kind.ns_mask(false).0 & ns.0 != 0 && is_local_assoc_visible(mod_tree, from_mod, ai.vis, ai.vis_mod) {
                        return Some(ResolvedAssoc {
                            def: ai.def,
                            owner_impl: mimp.local_impl,
                            owner_trait: LocalItemId::NONE,
                            kind: ai.kind,
                            vis: ai.vis,
                            vis_mod: ai.vis_mod,
                            has_self_receiver: ai.has_self_receiver,
                        });
                    }
                }
            } else {
                scratch.clear();
                db.assoc_items(mimp.id, &mut scratch);
                for d in &scratch {
                    if d.name == name && d.kind.ns_mask(false).0 & ns.0 != 0 && is_indexed_assoc_visible(self.krate, mimp.id.krate, d.vis) {
                        return Some(ResolvedAssoc {
                            def: d.id,
                            owner_impl: LocalImplId::NONE,
                            owner_trait: LocalItemId::NONE,
                            kind: d.kind,
                            vis: d.vis,
                            vis_mod: ModuleId::NONE,
                            has_self_receiver: d.has_value_ctor,
                        });
                    }
                }
            }
        }
        None
    }

    /// Resolves a trait associated item (`Trait::name`) honoring dirty-file trait overrides.
    pub fn resolve_trait_assoc(&self, db: &dyn ItemDb, trait_def: GlobalDefId, raw_name: &str, ns: NsMask) -> Option<ResolvedAssoc> {
        if !trait_def.is_some() {
            return None;
        }
        let name = norm_ident(raw_name);
        if let Some(&(tlid, a_start, a_len)) = self.local_impls.trait_assocs.iter().find(|&&(tlid, _, _)| self.items[tlid.0 as usize].index_def == trait_def) {
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
            return None;
        }
        let mut scratch: Vec<IndexedDef<'_>> = Vec::new();
        db.assoc_items(trait_def, &mut scratch);
        scratch.iter().find(|d| d.name == name && d.kind.ns_mask(false).0 & ns.0 != 0).map(|d| ResolvedAssoc {
            def: d.id,
            owner_impl: LocalImplId::NONE,
            owner_trait: LocalItemId::NONE,
            kind: d.kind,
            vis: VisKind::Public,
            vis_mod: ModuleId::NONE,
            has_self_receiver: d.has_value_ctor,
        })
    }

    /// Resolves `<T as Trait>::name` — checks the trait declaration first as a fast O(1) guard,
    /// then prefers a specialized item on `(target, trait_def)` `impl` when `target` is concrete.
    pub fn resolve_trait_impl_assoc(&self, db: &dyn ItemDb, target: ImplTargetKey, trait_def: GlobalDefId, raw_name: &str, ns: NsMask) -> Option<ResolvedAssoc> {
        if matches!(target, ImplTargetKey::Unknown | ImplTargetKey::Param) {
            return self.resolve_trait_assoc(db, trait_def, raw_name, ns);
        }
        let name = norm_ident(raw_name);
        let mut impls = Vec::new();
        self.merged_impls_for_target(db, target, true, &mut impls);
        let mut scratch: Vec<IndexedDef<'_>> = Vec::new();
        for mimp in impls {
            if mimp.trait_def != trait_def || mimp.is_negative {
                continue;
            }
            if mimp.local_impl.is_some() {
                let limp = &self.local_impls.impls[mimp.local_impl.0 as usize];
                for ai in self.local_impls.impl_assocs(limp) {
                    if self.local_impls.assoc_name(ai) == name && ai.kind.ns_mask(false).0 & ns.0 != 0 {
                        return Some(ResolvedAssoc {
                            def: ai.def,
                            owner_impl: mimp.local_impl,
                            owner_trait: LocalItemId::NONE,
                            kind: ai.kind,
                            vis: VisKind::Public,
                            vis_mod: ModuleId::NONE,
                            has_self_receiver: ai.has_self_receiver,
                        });
                    }
                }
            } else {
                scratch.clear();
                db.assoc_items(mimp.id, &mut scratch);
                if let Some(d) = scratch.iter().find(|d| d.name == name && d.kind.ns_mask(false).0 & ns.0 != 0) {
                    return Some(ResolvedAssoc {
                        def: d.id,
                        owner_impl: LocalImplId::NONE,
                        owner_trait: LocalItemId::NONE,
                        kind: d.kind,
                        vis: VisKind::Public,
                        vis_mod: ModuleId::NONE,
                        has_self_receiver: d.has_value_ctor,
                    });
                }
            }
        }
        self.resolve_trait_assoc(db, trait_def, raw_name, ns)
    }

    /// Resolves `Target::name` through traits visible in `from_mod` (plus block-local `extra_traits`).
    /// Prefers concrete `impl Trait for Target` items first so competing trait names disambiguate
    /// toward the trait actually implemented by `Target`, then falls back to blanket/declaration items.
    #[allow(clippy::too_many_arguments)]
    pub fn resolve_in_scope_trait_assoc(
        &self,
        db: &dyn ItemDb,
        mod_tree: &ModuleTree,
        from_mod: ModuleId,
        extra_traits: &[ResolvedBinding],
        target: ImplTargetKey,
        raw_name: &str,
        ns: NsMask,
    ) -> Option<(GlobalDefId, ResolvedAssoc)> {
        if target != ImplTargetKey::Unknown {
            let mut impls = Vec::new();
            self.merged_impls_for_target(db, target, false, &mut impls);
            for mimp in &impls {
                if !mimp.trait_def.is_some() || mimp.is_negative {
                    continue;
                }
                if self.is_trait_def_in_scope(mod_tree, from_mod, extra_traits, mimp.trait_def) {
                    if let Some(assoc) = self.resolve_trait_impl_assoc(db, target, mimp.trait_def, raw_name, ns) {
                        return Some((mimp.trait_def, assoc));
                    }
                }
            }
        }
        for tb in extra_traits {
            if tb.def.is_some() && self.resolve_trait_assoc(db, tb.def, raw_name, ns).is_some() {
                if let Some(assoc) = self.resolve_trait_impl_assoc(db, target, tb.def, raw_name, ns) {
                    return Some((tb.def, assoc));
                }
            }
        }
        for &tdef in &*self.scope(from_mod).visible_traits {
            if self.resolve_trait_assoc(db, tdef, raw_name, ns).is_some() {
                if let Some(assoc) = self.resolve_trait_impl_assoc(db, target, tdef, raw_name, ns) {
                    return Some((tdef, assoc));
                }
            }
        }
        if !mod_tree.module(from_mod).has_no_implicit_prelude() {
            for &tdef in &*self.prelude_traits {
                if self.resolve_trait_assoc(db, tdef, raw_name, ns).is_some() {
                    if let Some(assoc) = self.resolve_trait_impl_assoc(db, target, tdef, raw_name, ns) {
                        return Some((tdef, assoc));
                    }
                }
            }
        }
        None
    }

    #[inline]
    pub(super) fn is_trait_def_in_scope(&self, mod_tree: &ModuleTree, from_mod: ModuleId, extra_traits: &[ResolvedBinding], trait_def: GlobalDefId) -> bool {
        if extra_traits.iter().any(|b| b.def == trait_def) {
            return true;
        }
        if self.scope(from_mod).visible_traits_sorted.binary_search(&trait_def).is_ok() {
            return true;
        }
        !mod_tree.module(from_mod).has_no_implicit_prelude() && self.prelude_traits_sorted.binary_search(&trait_def).is_ok()
    }
}

pub(super) fn is_local_assoc_visible(tree: &ModuleTree, from_mod: ModuleId, vis: VisKind, vis_mod: ModuleId) -> bool {
    match vis {
        VisKind::Public | VisKind::Crate => true,
        VisKind::Private | VisKind::Super | VisKind::Restricted => is_descendant_or_eq(tree, from_mod, vis_mod),
    }
}

pub(super) fn is_indexed_assoc_visible(cur_krate: CrateId, owner_krate: CrateId, vis: VisKind) -> bool {
    match vis {
        VisKind::Public => true,
        VisKind::Crate => cur_krate == owner_krate,
        VisKind::Private | VisKind::Super | VisKind::Restricted => cur_krate == owner_krate,
    }
}
