/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Crate-wide item and import resolution table (`CrateDefMap`) — `use` trees, globs, re-exports,
//! `extern crate`, and `std`/`core` edition preludes resolved into [`GlobalDefId`]s.

use std::collections::HashMap;

use super::db::{CrateId, Edition, FileId, GlobalDefId, ItemDb, ItemKind, ModuleId, NsMask, SourceDb, VisKind};
use super::db_impls::ImplTargetKey;
use super::def_collect::{collect_crate_items, CollectedCrate, LocalItemData};
use super::def_solver::ImportSolver;
use super::ext_scope::ExtScopeCache;
use super::ident_key::IdentKey;
use super::impl_collect::LocalImplArena;
use super::impl_resolve::resolve_local_impls_and_assocs;
use super::mod_tree::ModuleTree;
use super::per_ns::{BindingOrigin, LocalItemId, PerNs, PrimTy, ResolvedBinding};
use super::use_tree::{PathAnchor, UseArena};
use crate::sema_fn::norm_ident;
use crate::tree::SyntaxTree;

/// Resolved symbol table of one module inside [`CrateDefMap`].
#[derive(Debug, Default)]
pub struct ModuleScope {
    pub index_def: GlobalDefId,
    pub entries: HashMap<IdentKey, PerNs>,
    /// Anonymous trait imports (`use Trait as _;`) visible for method lookup in this module.
    pub unnamed_traits: Vec<ResolvedBinding>,
}

impl ModuleScope {
    #[inline]
    pub fn get(&self, name: &str) -> PerNs {
        self.entries.get(name).copied().unwrap_or(PerNs::EMPTY)
    }
}

/// Non-fatal import-resolution diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UseDiag {
    UnresolvedUse { module: ModuleId, use_index: u32 },
    UnresolvedExternCrate { module: ModuleId, name: String },
}

/// Complete crate-level definition & import map built without invoking `rustc`.
#[derive(Debug)]
pub struct CrateDefMap {
    pub krate: CrateId,
    pub edition: Edition,
    pub prelude_mod: Option<GlobalDefId>,
    pub prelude: HashMap<IdentKey, PerNs>,
    /// Crate-wide extern crate prelude (`name -> (CrateId, root GlobalDefId)`).
    pub extern_prelude: HashMap<IdentKey, (CrateId, GlobalDefId)>,
    pub items: Vec<LocalItemData>,
    pub(super) item_strings: String,
    pub uses: UseArena,
    pub scopes: Vec<ModuleScope>,
    /// Per-`FlatUse` resolved `PerNs` (`resolved_uses[i]` corresponds to `uses.uses[i]`).
    pub resolved_uses: Vec<PerNs>,
    pub dirty_files: Vec<FileId>,
    pub local_impls: LocalImplArena,
    pub(super) local_alias_targets: HashMap<GlobalDefId, ImplTargetKey>,
    pub diagnostics: Vec<UseDiag>,
}

impl CrateDefMap {
    /// Builds the `CrateDefMap` for `mod_tree` with no dirty-file overrides.
    pub fn build(db: &dyn ItemDb, sources: &dyn SourceDb, mod_tree: &ModuleTree) -> Self {
        Self::build_cached(db, sources, mod_tree, &[], |_| None)
    }

    /// Builds the `CrateDefMap` reusing pre-parsed [`SyntaxTree`]s and honoring `dirty_files`.
    pub fn build_cached<'t>(
        db: &dyn ItemDb,
        sources: &dyn SourceDb,
        mod_tree: &ModuleTree,
        dirty_files: &[FileId],
        get_tree: impl FnMut(FileId) -> Option<&'t SyntaxTree<'t>>,
    ) -> Self {
        let collected = collect_crate_items(db, sources, mod_tree, dirty_files, get_tree);
        let mut ext_cache = ExtScopeCache::new();
        Self::from_collected(db, mod_tree, collected, dirty_files, &mut ext_cache)
    }

    pub(super) fn from_collected(db: &dyn ItemDb, mod_tree: &ModuleTree, collected: CollectedCrate, dirty_files: &[FileId], ext_cache: &mut ExtScopeCache) -> Self {
        let next_seq = collected.next_overlay_seq;
        let has_local_impls = !collected.local_impls.impls.is_empty() || !collected.local_impls.trait_assocs.is_empty() || !collected.local_impls.type_aliases.is_empty();
        let mut map = ImportSolver::solve(db, mod_tree, collected, dirty_files, ext_cache);
        if has_local_impls {
            resolve_local_impls_and_assocs(&mut map, db, mod_tree, ext_cache, next_seq);
        }
        map
    }

    #[inline]
    pub fn scope(&self, module: ModuleId) -> &ModuleScope {
        &self.scopes[module.0 as usize]
    }

    #[inline]
    pub fn item(&self, id: LocalItemId) -> &LocalItemData {
        &self.items[id.0 as usize]
    }

    #[inline]
    pub fn item_name(&self, id: LocalItemId) -> &str {
        let it = self.item(id);
        &self.item_strings[it.name_start as usize..it.name_end as usize]
    }

    /// Resolves an unqualified single-segment identifier `raw_name` inside `module` across all
    /// namespaces (`module scope -> edition prelude -> extern crate prelude (2018+) -> primitives`).
    pub fn resolve_name_in_module(&self, mod_tree: &ModuleTree, module: ModuleId, raw_name: &str) -> PerNs {
        let name = norm_ident(raw_name);
        let mut res = self.scope(module).get(name).filter_visible(mod_tree, module);

        if !mod_tree.module(module).has_no_implicit_prelude() {
            if let Some(&pre) = self.prelude.get(name) {
                res = res.or_else_ns(pre);
            }
        }
        if self.edition.has_uniform_paths() && res.types.is_none() {
            if let Some(&(ext_krate, ext_root)) = self.extern_prelude.get(name) {
                res.types = Some(extern_crate_binding(ext_krate, self.krate, ext_root));
            }
        }
        if res.types.is_none() {
            if let Some(p) = PrimTy::from_name(name) {
                res.types = Some(ResolvedBinding::primitive(p, VisKind::Public, ModuleId::NONE, BindingOrigin::Builtin));
            }
        }
        res
    }

    /// Resolves a multi-segment path `(anchor, segments)` from `module` across local modules,
    /// re-exports, globs, external dependency crates, enum variants, and builtin primitives.
    pub fn resolve_path(&self, db: &dyn ItemDb, mod_tree: &ModuleTree, module: ModuleId, anchor: PathAnchor, segments: &[&str]) -> PerNs {
        let mut ext_cache = ExtScopeCache::new();
        self.resolve_path_with_cache(db, mod_tree, &mut ext_cache, module, anchor, segments)
    }

    /// Same as [`Self::resolve_path`], reusing a caller-owned [`ExtScopeCache`] across bulk queries.
    pub fn resolve_path_with_cache(&self, db: &dyn ItemDb, mod_tree: &ModuleTree, ext_cache: &mut ExtScopeCache, module: ModuleId, anchor: PathAnchor, segments: &[&str]) -> PerNs {
        ImportSolver::resolve_arbitrary_path(self, db, mod_tree, ext_cache, module, anchor, segments)
    }

    /// Collects all traits in scope inside `module` (explicit/glob named traits, `use Trait as _;`,
    /// and standard prelude traits when implicit prelude is active).
    pub fn traits_in_scope(&self, mod_tree: &ModuleTree, module: ModuleId) -> Vec<ResolvedBinding> {
        let sc = self.scope(module);
        let mut out: Vec<ResolvedBinding> = Vec::with_capacity(sc.unnamed_traits.len() + 16);
        out.extend(sc.unnamed_traits.iter().copied());
        for pns in sc.entries.values() {
            if let Some(t) = pns.types {
                if !t.ambiguous && t.kind == ItemKind::Trait && !out.iter().any(|x| x.same_target(t)) {
                    out.push(t);
                }
            }
        }
        if !mod_tree.module(module).has_no_implicit_prelude() {
            for pns in self.prelude.values() {
                if let Some(t) = pns.types {
                    if t.kind == ItemKind::Trait && !out.iter().any(|x| x.same_target(t)) {
                        out.push(t);
                    }
                }
            }
        }
        out
    }
}

#[inline]
pub(super) fn extern_crate_binding(target_krate: CrateId, self_krate: CrateId, root_def: GlobalDefId) -> ResolvedBinding {
    ResolvedBinding {
        def: root_def,
        local: LocalItemId::NONE,
        local_mod: if target_krate == self_krate { ModuleId::ROOT } else { ModuleId::NONE },
        vis_mod: ModuleId::NONE,
        kind: ItemKind::Mod,
        vis: VisKind::Public,
        origin: BindingOrigin::ExternPrelude,
        prim_ty: 0,
        ambiguous: false,
    }
}

#[inline]
pub(super) fn local_mod_binding(mod_id: ModuleId, index_def: GlobalDefId, vis: VisKind, vis_mod: ModuleId) -> PerNs {
    let b = ResolvedBinding {
        def: index_def,
        local: LocalItemId::NONE,
        local_mod: mod_id,
        vis_mod,
        kind: ItemKind::Mod,
        vis,
        origin: BindingOrigin::Explicit,
        prim_ty: 0,
        ambiguous: false,
    };
    PerNs::from_binding(b, NsMask::TYPE)
}
