/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Zero-allocation memoized associated-item, trait-impl, blanket-impl, and supertrait lookup tables
//! stored inside [`super::ext_scope::ExtScopeCache`].

use std::collections::HashMap;

use super::db::{CrateId, FileId, GlobalDefId, IndexedDef, ItemDb, ItemKind, NsMask, VisKind};
use super::db_impls::{ImplTargetKey, IndexedImpl};
use super::ident_key::IdentKey;

#[derive(Clone, Debug)]
pub struct CachedAssocItem {
    pub name: IdentKey,
    pub def: GlobalDefId,
    pub impl_file: FileId,
    pub impl_krate: CrateId,
    pub kind: ItemKind,
    pub vis: VisKind,
    pub has_self_receiver: bool,
}

#[derive(Clone, Debug)]
pub struct CachedTraitImpl {
    pub trait_def: GlobalDefId,
    pub file: FileId,
    pub items: Box<[CachedAssocItem]>,
}

type ModTraitAssocMap = HashMap<IdentKey, Box<[(GlobalDefId, NsMask)]>>;

#[derive(Clone, Debug)]
struct CachedModTraitScope {
    visible_traits: Box<[GlobalDefId]>,
    include_prelude: bool,
    by_name: ModTraitAssocMap,
}

#[derive(Clone, Debug, Default)]
pub struct AssocScopeCache {
    inherent_by_target: HashMap<ImplTargetKey, Box<[CachedAssocItem]>>,
    trait_decl_by_def: HashMap<GlobalDefId, Box<[CachedAssocItem]>>,
    supertraits_by_def: HashMap<GlobalDefId, Box<[GlobalDefId]>>,
    hierarchy_decl_by_def: HashMap<GlobalDefId, Box<[CachedAssocItem]>>,
    trait_impls_by_target: HashMap<ImplTargetKey, Box<[CachedTraitImpl]>>,
    blanket_by_trait: Option<HashMap<GlobalDefId, Box<[CachedTraitImpl]>>>,
    clean_mod_trait_items: HashMap<u64, CachedModTraitScope>,
}

impl AssocScopeCache {
    pub fn inherent_items(&mut self, db: &dyn ItemDb, target: ImplTargetKey) -> &[CachedAssocItem] {
        self.inherent_by_target.entry(target).or_insert_with(|| {
            let mut impls: Vec<IndexedImpl> = Vec::new();
            db.impls_for_target(target, &mut impls);
            let mut scratch: Vec<IndexedDef<'_>> = Vec::new();
            let mut out = Vec::new();
            for imp in impls {
                if imp.trait_def.is_some() || imp.is_negative {
                    continue;
                }
                scratch.clear();
                db.assoc_items(imp.id, &mut scratch);
                for d in &scratch {
                    out.push(CachedAssocItem {
                        name: IdentKey::new(d.name),
                        def: d.id,
                        impl_file: imp.file,
                        impl_krate: imp.id.krate,
                        kind: d.kind,
                        vis: d.vis,
                        has_self_receiver: d.has_value_ctor,
                    });
                }
            }
            out.into_boxed_slice()
        })
    }

    pub fn trait_decl_items(&mut self, db: &dyn ItemDb, trait_def: GlobalDefId) -> &[CachedAssocItem] {
        self.trait_decl_by_def.entry(trait_def).or_insert_with(|| {
            let mut scratch: Vec<IndexedDef<'_>> = Vec::new();
            db.assoc_items(trait_def, &mut scratch);
            scratch
                .into_iter()
                .map(|d| CachedAssocItem {
                    name: IdentKey::new(d.name),
                    def: d.id,
                    impl_file: FileId::NONE,
                    impl_krate: trait_def.krate,
                    kind: d.kind,
                    vis: VisKind::Public,
                    has_self_receiver: d.has_value_ctor,
                })
                .collect::<Vec<_>>()
                .into_boxed_slice()
        })
    }

    /// Returns `[trait_def, transitive_supertraits...]` (bounded BFS up to 16 traits).
    pub fn trait_hierarchy(&mut self, db: &dyn ItemDb, trait_def: GlobalDefId) -> &[GlobalDefId] {
        self.supertraits_by_def.entry(trait_def).or_insert_with(|| {
            let mut chain = Vec::with_capacity(4);
            chain.push(trait_def);
            let mut head = 0usize;
            let mut scratch = Vec::new();
            while head < chain.len() && chain.len() < 16 {
                let cur = chain[head];
                head += 1;
                scratch.clear();
                db.trait_supertraits(cur, &mut scratch);
                for &sup in &scratch {
                    if sup.is_some() && !chain.contains(&sup) && chain.len() < 16 {
                        chain.push(sup);
                    }
                }
            }
            chain.into_boxed_slice()
        })
    }

    pub fn target_trait_impls(&mut self, db: &dyn ItemDb, target: ImplTargetKey) -> &[CachedTraitImpl] {
        self.trait_impls_by_target.entry(target).or_insert_with(|| {
            let mut impls: Vec<IndexedImpl> = Vec::new();
            db.impls_for_target(target, &mut impls);
            let mut scratch: Vec<IndexedDef<'_>> = Vec::new();
            let mut out = Vec::new();
            for imp in impls {
                if !imp.trait_def.is_some() || imp.is_negative {
                    continue;
                }
                scratch.clear();
                db.assoc_items(imp.id, &mut scratch);
                let items = scratch
                    .iter()
                    .map(|d| CachedAssocItem {
                        name: IdentKey::new(d.name),
                        def: d.id,
                        impl_file: imp.file,
                        impl_krate: imp.id.krate,
                        kind: d.kind,
                        vis: VisKind::Public,
                        has_self_receiver: d.has_value_ctor,
                    })
                    .collect::<Vec<_>>()
                    .into_boxed_slice();
                out.push(CachedTraitImpl { trait_def: imp.trait_def, file: imp.file, items });
            }
            out.into_boxed_slice()
        })
    }

    pub fn blanket_trait_impls(&mut self, db: &dyn ItemDb, trait_def: GlobalDefId) -> &[CachedTraitImpl] {
        let map = self.blanket_by_trait.get_or_insert_with(|| {
            let mut impls: Vec<IndexedImpl> = Vec::new();
            db.impls_for_target(ImplTargetKey::Param, &mut impls);
            let mut scratch: Vec<IndexedDef<'_>> = Vec::new();
            let mut grouped: HashMap<GlobalDefId, Vec<CachedTraitImpl>> = HashMap::new();
            for imp in impls {
                if !imp.trait_def.is_some() || imp.is_negative {
                    continue;
                }
                scratch.clear();
                db.assoc_items(imp.id, &mut scratch);
                let items = scratch
                    .iter()
                    .map(|d| CachedAssocItem {
                        name: IdentKey::new(d.name),
                        def: d.id,
                        impl_file: imp.file,
                        impl_krate: imp.id.krate,
                        kind: d.kind,
                        vis: VisKind::Public,
                        has_self_receiver: d.has_value_ctor,
                    })
                    .collect::<Vec<_>>()
                    .into_boxed_slice();
                grouped.entry(imp.trait_def).or_default().push(CachedTraitImpl { trait_def: imp.trait_def, file: imp.file, items });
            }
            grouped.into_iter().map(|(k, v)| (k, v.into_boxed_slice())).collect()
        });
        map.get(&trait_def).map_or(&[], |b| &**b)
    }

    pub fn hierarchy_decl_items(&mut self, db: &dyn ItemDb, trait_def: GlobalDefId) -> &[CachedAssocItem] {
        if !self.hierarchy_decl_by_def.contains_key(&trait_def) {
            let chain: Box<[GlobalDefId]> = self.trait_hierarchy(db, trait_def).into();
            let mut items = Vec::new();
            for &td in &*chain {
                items.extend_from_slice(self.trait_decl_items(db, td));
            }
            self.hierarchy_decl_by_def.insert(trait_def, items.into_boxed_slice());
        }
        &self.hierarchy_decl_by_def[&trait_def]
    }

    pub fn clean_module_trait_candidates<'a>(
        &'a mut self,
        db: &dyn ItemDb,
        scope_hash: u64,
        visible_traits: &[GlobalDefId],
        prelude_traits: &[GlobalDefId],
        include_prelude: bool,
        name: &str,
    ) -> Option<&'a [(GlobalDefId, NsMask)]> {
        let needs_build = match self.clean_mod_trait_items.get(&scope_hash) {
            Some(entry) => entry.include_prelude != include_prelude || entry.visible_traits.as_ref() != visible_traits,
            None => true,
        };
        if needs_build {
            let mut grouped: HashMap<IdentKey, Vec<(GlobalDefId, NsMask)>> = HashMap::new();
            let prel = if include_prelude { prelude_traits } else { &[] };
            for &tdef in visible_traits.iter().chain(prel.iter()) {
                for c in self.hierarchy_decl_items(db, tdef) {
                    let mask = c.kind.ns_mask(false);
                    let list = grouped.entry(c.name.clone()).or_default();
                    if let Some(existing) = list.iter_mut().find(|(d, _)| *d == tdef) {
                        existing.1 .0 |= mask.0;
                    } else {
                        list.push((tdef, mask));
                    }
                }
            }
            let by_name: ModTraitAssocMap = grouped.into_iter().map(|(k, v)| (k, v.into_boxed_slice())).collect();
            self.clean_mod_trait_items.insert(scope_hash, CachedModTraitScope { visible_traits: visible_traits.into(), include_prelude, by_name });
        }
        self.clean_mod_trait_items.get(&scope_hash).and_then(|entry| entry.by_name.get(name).map(|b| &**b))
    }
}
