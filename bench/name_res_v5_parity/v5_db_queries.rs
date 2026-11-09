/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `ridx_syntax::name_res::ItemDb` implementation for [`crate::v5_db::RidxV5ItemDb`].

use ridx_format::{DefKind, LangItemKind, UseOwner, USE_FLAG_GLOB, USE_FLAG_PRELUDE, USE_FLAG_REEXPORT, USE_FLAG_UNDERSCORE};
use ridx_syntax::name_res::{CrateId, Edition, FileId, GlobalDefId, ImplTargetKey, IndexedDef, IndexedImpl, IndexedUse, ItemDb, ItemKind, VisKind};

use crate::v5_db::{RidxV5ItemDb, CRATE_ROOT_DEF_IDX, IMPL_SYNTH_BIT, PROC_MACRO_SYNTH_BIT};
use crate::v5_shard::{def_has_value_ctor, map_ridx_kind, map_use_prim, map_vis};

impl<'w> RidxV5ItemDb<'w> {
    fn push_def_idx<'a>(&'a self, r_idx: usize, didx: u32, out: &mut Vec<IndexedDef<'a>>) {
        if (didx & PROC_MACRO_SYNTH_BIT) != 0 {
            let sym_idx = (didx & !PROC_MACRO_SYNTH_BIT) as usize;
            if let Some(name) = self.shards.get(r_idx).and_then(|s| s.synth_macros.get(sym_idx)) {
                out.push(IndexedDef { id: GlobalDefId::new(CrateId(r_idx as u32), didx), name: name.as_str(), kind: ItemKind::Macro, vis: VisKind::Public, has_value_ctor: false });
            }
            return;
        }
        let reader = &self.ws.readers()[r_idx];
        let Some(d) = reader.definitions().get(didx as usize) else {
            return;
        };
        let Some(kind) = map_ridx_kind(DefKind::from(d.kind)) else {
            return;
        };
        let name = reader.get_str(d.name_offset());
        if name.is_empty() {
            return;
        }
        out.push(IndexedDef {
            id: GlobalDefId::new(CrateId(r_idx as u32), didx),
            name,
            kind,
            vis: if d.is_reexport() { VisKind::Public } else { map_vis(d.visibility) },
            has_value_ctor: def_has_value_ctor(reader, didx, kind),
        });
    }
}

impl<'w> ItemDb for RidxV5ItemDb<'w> {
    #[inline]
    fn cfg_flag(&self, krate: CrateId, key: &str) -> bool {
        let Some(r) = self.ws.readers().get(krate.0 as usize) else {
            return false;
        };
        if key == "test" && self.metas.get(krate.0 as usize).is_some_and(|m| m.is_test_harness) {
            return true;
        }
        r.has_cfg_flag(key)
    }

    #[inline]
    fn cfg_key_value(&self, krate: CrateId, key: &str, value: &str) -> bool {
        let Some(r) = self.ws.readers().get(krate.0 as usize) else {
            return false;
        };
        r.has_cfg_key_value(key, value)
    }

    #[inline]
    fn crate_edition(&self, krate: CrateId) -> Edition {
        self.metas.get(krate.0 as usize).map_or(Edition::Edition2021, |m| m.edition)
    }

    #[inline]
    fn crate_name(&self, krate: CrateId) -> &str {
        self.metas.get(krate.0 as usize).map_or("", |m| m.crate_name.as_str())
    }

    #[inline]
    fn crate_env(&self, krate: CrateId, key: &str) -> Option<&str> {
        self.metas.get(krate.0 as usize)?.env.get(key).map(String::as_str)
    }

    #[inline]
    fn resolve_include_file(&self, from_file: FileId, rel_or_abs: &str) -> Option<(FileId, &str)> {
        crate::meta::resolve_include_in_corpus(self.src, from_file, rel_or_abs)
    }

    #[inline]
    fn include_bytes_len(&self, from_file: FileId, rel_or_abs: &str) -> Option<u64> {
        crate::meta::include_bytes_len_in_corpus(self.src, from_file, rel_or_abs)
    }

    fn extern_crate(&self, from_krate: CrateId, extern_name: &str) -> Option<CrateId> {
        let from_idx = from_krate.0 as usize;
        if let Some(uk) = self.metas.get(from_idx).and_then(|m| m.extern_units.get(extern_name)) {
            if let Some(r_idx) = self.resolve_unit_to_reader(from_idx, uk) {
                return Some(CrateId(r_idx as u32));
            }
        }
        if matches!(extern_name, "core" | "std") {
            if let Some(&sr) = self.sysroot_by_name.get(extern_name) {
                return Some(CrateId(sr as u32));
            }
        }
        if extern_name == "proc_macro" {
            if let Some(&sr) = self.sysroot_by_name.get("proc_macro") {
                return Some(CrateId(sr as u32));
            }
        }
        if extern_name == "alloc" {
            if let Some(&sr) = self.sysroot_by_name.get("alloc") {
                return Some(CrateId(sr as u32));
            }
        }
        None
    }

    fn extern_crates<'a>(&'a self, from_krate: CrateId, out: &mut Vec<(&'a str, CrateId)>) {
        let from_idx = from_krate.0 as usize;
        let Some(meta) = self.metas.get(from_idx) else {
            return;
        };
        for (alias, uk) in &meta.extern_units {
            if let Some(r_idx) = self.resolve_unit_to_reader(from_idx, uk) {
                out.push((alias.as_str(), CrateId(r_idx as u32)));
            }
        }
    }

    #[inline]
    fn crate_root_def(&self, krate: CrateId) -> Option<GlobalDefId> {
        let r_idx = krate.0 as usize;
        (r_idx < self.ws.crate_count()).then(|| self.crate_root_gid(r_idx))
    }

    fn prelude_module(&self, krate: CrateId) -> Option<GlobalDefId> {
        let r_idx = krate.0 as usize;
        let reader = self.ws.readers().get(r_idx)?;
        let pu = reader.prelude_use()?;
        let arena = reader.ty_arena();
        let td = pu.type_def.or(pu.value_def).or(pu.macro_def)?;
        let g = self.resolve_ty_def(r_idx, arena, td);
        g.is_some().then_some(g)
    }

    #[inline]
    fn macro_rules_body(&self, krate: CrateId, module_path: &str, call_path: &str) -> Option<&str> {
        crate::v5_macro_res::resolve_v5_macro_rules(self, krate, module_path, call_path).map(|(b, _)| b)
    }

    #[inline]
    fn macro_rules_with_crate(&self, krate: CrateId, module_path: &str, call_path: &str) -> Option<(&str, &str)> {
        crate::v5_macro_res::resolve_v5_macro_rules(self, krate, module_path, call_path)
    }

    #[inline]
    fn macro_rules_by_def(&self, from_krate: CrateId, def: GlobalDefId) -> Option<(&str, &str)> {
        crate::v5_macro_res::body_of_macro_gid(self, from_krate.0 as usize, def)
    }

    fn def_kind(&self, def: GlobalDefId) -> Option<ItemKind> {
        let r_idx = def.krate.0 as usize;
        let reader = self.ws.readers().get(r_idx)?;
        if def.index == CRATE_ROOT_DEF_IDX {
            return Some(ItemKind::Mod);
        }
        if (def.index & IMPL_SYNTH_BIT) != 0 {
            return Some(ItemKind::Impl);
        }
        if (def.index & PROC_MACRO_SYNTH_BIT) != 0 {
            return Some(ItemKind::Macro);
        }
        let d = reader.definitions().get(def.index as usize)?;
        map_ridx_kind(DefKind::from(d.kind))
    }

    fn def_file(&self, def: GlobalDefId) -> FileId {
        let r_idx = def.krate.0 as usize;
        if def.index == CRATE_ROOT_DEF_IDX || (def.index & (IMPL_SYNTH_BIT | PROC_MACRO_SYNTH_BIT)) != 0 {
            return FileId::NONE;
        }
        let Some(reader) = self.ws.readers().get(r_idx) else {
            return FileId::NONE;
        };
        let Some(d) = reader.definitions().get(def.index as usize) else {
            return FileId::NONE;
        };
        self.shards.get(r_idx).and_then(|s| s.file_map.get(d.span().file_id() as usize).copied()).unwrap_or(FileId::NONE)
    }

    fn type_alias_target(&self, alias_def: GlobalDefId) -> Option<ImplTargetKey> {
        let r_idx = alias_def.krate.0 as usize;
        let reader = self.ws.readers().get(r_idx)?;
        let dt = reader.def_ty_for_def(alias_def.index)?;
        let key = self.lower_ty_to_target_key(r_idx, reader.ty_arena(), dt.ty(), 0);
        (key != ImplTargetKey::Unknown).then_some(key)
    }

    fn impls_for_target(&self, target: ImplTargetKey, out: &mut Vec<IndexedImpl>) {
        match target {
            ImplTargetKey::Unknown => {}
            ImplTargetKey::Adt(adt_def) => {
                if let Some(list) = self.impls_by_adt.get(&adt_def) {
                    out.extend_from_slice(list);
                }
            }
            other => {
                if let Some(list) = self.impls_by_other.get(&other) {
                    out.extend_from_slice(list);
                }
            }
        }
    }

    fn crate_impls(&self, krate: CrateId, out: &mut Vec<IndexedImpl>) {
        if let Some(list) = self.impls_by_crate.get(krate.0 as usize) {
            out.extend_from_slice(list);
        }
    }

    fn trait_supertraits(&self, trait_def: GlobalDefId, out: &mut Vec<GlobalDefId>) {
        if let Some(list) = self.trait_supertraits.get(&trait_def) {
            out.extend_from_slice(list);
        }
    }

    fn module_defs<'a>(&'a self, module_def: GlobalDefId, out: &mut Vec<IndexedDef<'a>>) {
        let r_idx = module_def.krate.0 as usize;
        let Some(shard) = self.shards.get(r_idx) else {
            return;
        };
        let key = if module_def.index == CRATE_ROOT_DEF_IDX || Some(module_def.index) == shard.root_mod_def_idx {
            shard.root_mod_def_idx.unwrap_or(CRATE_ROOT_DEF_IDX)
        } else {
            module_def.index
        };
        if let Some(children) = shard.mod_children.get(&key) {
            for &didx in children {
                self.push_def_idx(r_idx, didx, out);
            }
        }
    }

    fn enum_variants<'a>(&'a self, enum_def: GlobalDefId, out: &mut Vec<IndexedDef<'a>>) {
        let r_idx = enum_def.krate.0 as usize;
        let Some(shard) = self.shards.get(r_idx) else {
            return;
        };
        if let Some(vars) = shard.enum_variants.get(&enum_def.index) {
            for &didx in vars {
                self.push_def_idx(r_idx, didx, out);
            }
        }
    }

    fn module_uses<'a>(&'a self, module_def: GlobalDefId, out: &mut Vec<IndexedUse<'a>>) {
        let r_idx = module_def.krate.0 as usize;
        let Some(reader) = self.ws.readers().get(r_idx) else {
            return;
        };
        let shard = &self.shards[r_idx];
        let owner = if module_def.index == CRATE_ROOT_DEF_IDX || Some(module_def.index) == shard.root_mod_def_idx { UseOwner::CrateRoot } else { UseOwner::Def(module_def.index) };
        let arena = reader.ty_arena();
        for u in reader.uses_for_owner(owner) {
            out.push(IndexedUse {
                name: u.name,
                type_def: u.type_def.map_or(GlobalDefId::NONE, |td| self.resolve_ty_def(r_idx, arena, td)),
                value_def: u.value_def.map_or(GlobalDefId::NONE, |td| self.resolve_ty_def(r_idx, arena, td)),
                macro_def: u.macro_def.map_or(GlobalDefId::NONE, |td| self.resolve_ty_def(r_idx, arena, td)),
                vis: map_vis(u.vis as u8),
                is_glob: (u.flags & USE_FLAG_GLOB) != 0,
                is_reexport: (u.flags & USE_FLAG_REEXPORT) != 0,
                is_prelude: (u.flags & USE_FLAG_PRELUDE) != 0,
                is_underscore: (u.flags & USE_FLAG_UNDERSCORE) != 0,
                prim_ty_code: map_use_prim(u.prim_ty),
            });
        }
    }

    fn assoc_items<'a>(&'a self, owner_def: GlobalDefId, out: &mut Vec<IndexedDef<'a>>) {
        let r_idx = owner_def.krate.0 as usize;
        let Some(reader) = self.ws.readers().get(r_idx) else {
            return;
        };
        let shard = &self.shards[r_idx];
        if (owner_def.index & IMPL_SYNTH_BIT) != 0 {
            let impl_idx = owner_def.index & !IMPL_SYNTH_BIT;
            if let Some(iv) = reader.impl_view(impl_idx) {
                for didx in iv.assoc_def_indices() {
                    self.push_def_idx(r_idx, didx, out);
                }
            }
            return;
        }
        if let Some(&impl_idx) = shard.impl_def_to_impl_idx.get(&owner_def.index) {
            if let Some(iv) = reader.impl_view(impl_idx) {
                for didx in iv.assoc_def_indices() {
                    self.push_def_idx(r_idx, didx, out);
                }
            }
            return;
        }
        if let Some(assocs) = shard.trait_assocs.get(&owner_def.index) {
            for &didx in assocs {
                self.push_def_idx(r_idx, didx, out);
            }
        }
    }

    fn adt_field(&self, adt_or_variant_def: GlobalDefId, field_name: &str) -> Option<GlobalDefId> {
        let r_idx = adt_or_variant_def.krate.0 as usize;
        let reader = self.ws.readers().get(r_idx)?;
        let fv = reader.find_field(adt_or_variant_def.index, field_name)?;
        if let Some(didx) = fv.def_index {
            return Some(GlobalDefId::new(CrateId(r_idx as u32), didx));
        }
        None
    }

    fn def_at_byte_span(&self, krate: CrateId, file: FileId, start: u32, end: u32) -> GlobalDefId {
        let r_idx = krate.0 as usize;
        let Some(shard) = self.shards.get(r_idx) else {
            return GlobalDefId::NONE;
        };
        shard.span_to_def_idx.get(&(file, start, end)).map_or(GlobalDefId::NONE, |&didx| GlobalDefId::new(krate, didx))
    }

    fn lang_item(&self, lang_kind: u16) -> Option<GlobalDefId> {
        let kind = LangItemKind::from_u16(lang_kind);
        if kind == LangItemKind::Other {
            return None;
        }
        for r_idx in 0..self.ws.crate_count() {
            let reader = &self.ws.readers()[r_idx];
            if let Some(lv) = reader.lang_item(kind) {
                if let Some(didx) = lv.def_index {
                    return Some(GlobalDefId::new(CrateId(r_idx as u32), didx));
                }
                let g = self.resolve_ty_def(r_idx, reader.ty_arena(), lv.ty_def);
                if g.is_some() {
                    return Some(g);
                }
            }
        }
        None
    }
}
