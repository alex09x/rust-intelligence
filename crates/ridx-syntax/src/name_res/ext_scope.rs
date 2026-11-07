/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Cached lookup over external / compiler-indexed modules, enums, builtin `core::primitive`,
//! and `std`/`core` edition preludes (`2015`, `2018`, `2021`, `2024`).

use std::collections::HashMap;

use super::db::{CrateId, Edition, GlobalDefId, ItemDb, ItemKind, ModuleId, NsMask, VisKind};
use super::mod_tree::ModuleTree;
use super::per_ns::{BindingOrigin, LocalItemId, PerNs, PrimTy, ResolvedBinding};

/// Sentinel `GlobalDefId` representing the synthetic `core::primitive` / `std::primitive` module.
pub const SYNTHETIC_PRIMITIVE_MOD: GlobalDefId = GlobalDefId::new(CrateId(u32::MAX - 1), u32::MAX - 1);

/// Cached exported symbol table of one indexed `GlobalDefId` module or enum.
#[derive(Debug, Default)]
pub struct ExtModuleScope {
    pub entries: HashMap<String, PerNs>,
}

impl ExtModuleScope {
    #[inline]
    pub fn get(&self, name: &str) -> PerNs {
        self.entries.get(name).copied().unwrap_or(PerNs::EMPTY)
    }
}

/// Memoizes `ItemDb::module_defs` + `ItemDb::module_uses` + `ItemDb::enum_variants` queries across a crate.
#[derive(Debug, Default)]
pub struct ExtScopeCache {
    mod_scopes: HashMap<GlobalDefId, ExtModuleScope>,
    enum_scopes: HashMap<GlobalDefId, ExtModuleScope>,
    in_progress: Vec<GlobalDefId>,
}

impl ExtScopeCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the exported scope of indexed module `mod_def` (visibility-filtered to `Public`
    /// unless `same_crate == Some(mod_def.krate)`).
    pub fn module_scope(&mut self, db: &dyn ItemDb, mod_def: GlobalDefId) -> &ExtModuleScope {
        if !self.mod_scopes.contains_key(&mod_def) {
            let built = if mod_def == SYNTHETIC_PRIMITIVE_MOD { build_primitive_scope() } else { self.build_ext_mod_scope(db, mod_def) };
            self.mod_scopes.insert(mod_def, built);
        }
        &self.mod_scopes[&mod_def]
    }

    /// Returns the variant scope of indexed enum `enum_def`.
    pub fn enum_scope(&mut self, db: &dyn ItemDb, enum_def: GlobalDefId) -> &ExtModuleScope {
        self.enum_scopes.entry(enum_def).or_insert_with(|| {
            let mut defs = Vec::new();
            db.enum_variants(enum_def, &mut defs);
            let mut entries = HashMap::with_capacity(defs.len());
            for d in defs {
                if d.kind != ItemKind::Variant && d.kind != ItemKind::Const {
                    continue;
                }
                let b = ResolvedBinding {
                    def: d.id,
                    local: LocalItemId::NONE,
                    local_mod: ModuleId::NONE,
                    vis_mod: ModuleId::NONE,
                    kind: d.kind,
                    vis: VisKind::Public,
                    origin: BindingOrigin::Explicit,
                    prim_ty: 0,
                    ambiguous: false,
                };
                let mask = d.kind.ns_mask(d.has_value_ctor);
                entries.entry(d.name.to_string()).or_insert(PerNs::EMPTY).merge_explicit(PerNs::from_binding(b, mask));
            }
            ExtModuleScope { entries }
        })
    }

    fn build_ext_mod_scope(&mut self, db: &dyn ItemDb, mod_def: GlobalDefId) -> ExtModuleScope {
        if self.in_progress.contains(&mod_def) {
            return ExtModuleScope::default();
        }
        self.in_progress.push(mod_def);

        let mut defs = Vec::new();
        db.module_defs(mod_def, &mut defs);
        let mut uses = Vec::new();
        db.module_uses(mod_def, &mut uses);

        let mut entries: HashMap<String, PerNs> = HashMap::with_capacity(defs.len() + uses.len());

        // Synthesize `primitive` child module at the root of `core` / `std` if not already present.
        let kname = db.crate_name(mod_def.krate);
        if matches!(kname, "core" | "std") && db.crate_root_def(mod_def.krate) == Some(mod_def) {
            let prim_mod_b = ResolvedBinding {
                def: SYNTHETIC_PRIMITIVE_MOD,
                local: LocalItemId::NONE,
                local_mod: ModuleId::NONE,
                vis_mod: ModuleId::NONE,
                kind: ItemKind::Mod,
                vis: VisKind::Public,
                origin: BindingOrigin::Builtin,
                prim_ty: 0,
                ambiguous: false,
            };
            entries.insert("primitive".to_string(), PerNs::from_binding(prim_mod_b, NsMask::TYPE));
        }

        for d in defs {
            if d.vis != VisKind::Public {
                continue;
            }
            let b = ResolvedBinding {
                def: d.id,
                local: LocalItemId::NONE,
                local_mod: ModuleId::NONE,
                vis_mod: ModuleId::NONE,
                kind: d.kind,
                vis: d.vis,
                origin: BindingOrigin::Explicit,
                prim_ty: 0,
                ambiguous: false,
            };
            let mask = d.kind.ns_mask(d.has_value_ctor);
            entries.entry(d.name.to_string()).or_insert(PerNs::EMPTY).merge_explicit(PerNs::from_binding(b, mask));
        }

        let mut glob_targets: Vec<GlobalDefId> = Vec::new();
        for u in uses {
            if u.vis != VisKind::Public || u.is_prelude {
                continue;
            }
            if u.is_glob {
                if u.type_def.is_some() && u.type_def != mod_def {
                    glob_targets.push(u.type_def);
                }
                continue;
            }
            if u.is_underscore || u.name.is_empty() {
                continue;
            }
            let pns = indexed_use_to_per_ns(db, &u);
            if !pns.is_empty() {
                entries.entry(u.name.to_string()).or_insert(PerNs::EMPTY).merge_explicit(pns);
            }
        }

        for g_def in glob_targets {
            let sub = self.module_scope(db, g_def);
            let cloned: Vec<(String, PerNs)> = sub.entries.iter().map(|(k, &v)| (k.clone(), v)).collect();
            for (name, pns) in cloned {
                entries.entry(name).or_insert(PerNs::EMPTY).merge_glob(pns);
            }
        }

        self.in_progress.pop();
        ExtModuleScope { entries }
    }

    /// Resolves the `GlobalDefId` of the active standard prelude module (`std::prelude::rust_2021`, etc.).
    pub fn resolve_prelude_mod(&mut self, db: &dyn ItemDb, tree: &ModuleTree) -> Option<GlobalDefId> {
        if tree.no_core {
            return None;
        }
        if let Some(explicit) = db.prelude_module(tree.krate) {
            return Some(explicit);
        }
        let prelude_krate = if !tree.no_std { db.extern_crate(tree.krate, "std").or_else(|| db.extern_crate(tree.krate, "core")) } else { db.extern_crate(tree.krate, "core") }?;
        let root_def = db.crate_root_def(prelude_krate)?;
        let prelude_container = self.module_scope(db, root_def).get("prelude").types.filter(|b| b.def.is_some())?.def;
        let ed_mod_name = match tree.edition {
            Edition::Edition2015 => "rust_2015",
            Edition::Edition2018 => "rust_2018",
            Edition::Edition2021 => "rust_2021",
            Edition::Edition2024 => "rust_2024",
        };
        let p_scope = self.module_scope(db, prelude_container);
        p_scope.get(ed_mod_name).types.or_else(|| p_scope.get("v1").types).filter(|b| b.def.is_some()).map(|b| b.def)
    }
}

fn build_primitive_scope() -> ExtModuleScope {
    let mut entries = HashMap::with_capacity(PrimTy::ALL.len());
    for p in PrimTy::ALL {
        let b = ResolvedBinding::primitive(p, VisKind::Public, ModuleId::NONE, BindingOrigin::Explicit);
        entries.insert(p.name().to_string(), PerNs::from_binding(b, NsMask::TYPE));
    }
    ExtModuleScope { entries }
}

fn indexed_use_to_per_ns(db: &dyn ItemDb, u: &super::db::IndexedUse<'_>) -> PerNs {
    let types = if u.prim_ty_code != 0 {
        PrimTy::from_code(u.prim_ty_code).map(|p| ResolvedBinding::primitive(p, u.vis, ModuleId::NONE, BindingOrigin::Explicit))
    } else if u.type_def.is_some() {
        let kind = db.def_kind(u.type_def).unwrap_or(ItemKind::Struct);
        Some(ResolvedBinding {
            def: u.type_def,
            local: LocalItemId::NONE,
            local_mod: ModuleId::NONE,
            vis_mod: ModuleId::NONE,
            kind,
            vis: u.vis,
            origin: BindingOrigin::Explicit,
            prim_ty: 0,
            ambiguous: false,
        })
    } else {
        None
    };
    let values = u.value_def.is_some().then(|| {
        let kind = db.def_kind(u.value_def).unwrap_or(ItemKind::Fn);
        ResolvedBinding {
            def: u.value_def,
            local: LocalItemId::NONE,
            local_mod: ModuleId::NONE,
            vis_mod: ModuleId::NONE,
            kind,
            vis: u.vis,
            origin: BindingOrigin::Explicit,
            prim_ty: 0,
            ambiguous: false,
        }
    });
    let macros = u.macro_def.is_some().then_some(ResolvedBinding {
        def: u.macro_def,
        local: LocalItemId::NONE,
        local_mod: ModuleId::NONE,
        vis_mod: ModuleId::NONE,
        kind: ItemKind::Macro,
        vis: u.vis,
        origin: BindingOrigin::Explicit,
        prim_ty: 0,
        ambiguous: false,
    });
    PerNs { types, values, macros }
}
