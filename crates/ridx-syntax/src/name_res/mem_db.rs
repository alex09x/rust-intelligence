/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Fast in-memory [`SourceDb`] and [`ItemDb`] implementations for standalone tests,
//! workspace benchmarks, and overlay verification without requiring `ridx-format`.

use std::collections::HashMap;

use super::db::{CrateId, Edition, FileId, GlobalDefId, IndexedDef, IndexedUse, ItemDb, SourceDb};
use super::db_impls::{ImplTargetKey, IndexedImpl};
use super::mod_tree::join_norm_path;

/// In-memory source file table keyed by normalized `/` path.
#[derive(Clone, Debug, Default)]
pub struct MemSourceDb {
    paths: Vec<String>,
    texts: Vec<String>,
    by_path: HashMap<String, FileId>,
}

impl MemSourceDb {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self { paths: Vec::with_capacity(cap), texts: Vec::with_capacity(cap), by_path: HashMap::with_capacity(cap) }
    }

    /// Inserts or overwrites `path` (normalized to forward slashes) and returns its [`FileId`].
    pub fn add_file(&mut self, path: &str, text: impl Into<String>) -> FileId {
        let norm = join_norm_path("", path);
        if let Some(&existing) = self.by_path.get(&norm) {
            self.texts[existing.0 as usize] = text.into();
            return existing;
        }
        let id = FileId(self.paths.len() as u32);
        self.by_path.insert(norm.clone(), id);
        self.paths.push(norm);
        self.texts.push(text.into());
        id
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.paths.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }
}

impl SourceDb for MemSourceDb {
    #[inline]
    fn resolve_path(&self, normalized_path: &str) -> Option<FileId> {
        self.by_path.get(normalized_path).copied()
    }

    #[inline]
    fn file_path(&self, file: FileId) -> &str {
        &self.paths[file.0 as usize]
    }

    #[inline]
    fn file_text(&self, file: FileId) -> &str {
        &self.texts[file.0 as usize]
    }
}

#[derive(Clone, Debug, Default)]
struct CrateMemEntry {
    name: String,
    edition: Edition,
    /// Sorted bare cfg flags (`unix`, `test`, `debug_assertions`, ...).
    cfg_flags: Vec<String>,
    /// Sorted `(key, value)` pairs (`("feature", "rt")`, `("target_os", "linux")`, ...).
    cfg_kvs: Vec<(String, String)>,
    /// Optional indexed `macro_rules!` bodies keyed by call path (`"cfg_if"`, `"cfg::alloc"`).
    macros: HashMap<String, String>,
    externs: HashMap<String, CrateId>,
    root_def: Option<GlobalDefId>,
    prelude_mod: Option<GlobalDefId>,
}

/// In-memory [`ItemDb`] backing `cfg` evaluation, crate metadata, and indexed test fixtures.
#[derive(Clone, Debug, Default)]
pub struct MemItemDb {
    crates: HashMap<CrateId, CrateMemEntry>,
    def_kinds: HashMap<GlobalDefId, super::db::ItemKind>,
    def_files: HashMap<GlobalDefId, FileId>,
    alias_targets: HashMap<GlobalDefId, ImplTargetKey>,
    module_defs: HashMap<GlobalDefId, Vec<OwnedDef>>,
    module_uses: HashMap<GlobalDefId, Vec<OwnedUse>>,
    assoc_items: HashMap<GlobalDefId, Vec<OwnedDef>>,
    impls_by_target: HashMap<ImplTargetKey, Vec<IndexedImpl>>,
    impls_by_crate: HashMap<CrateId, Vec<IndexedImpl>>,
    lang_items: HashMap<u16, GlobalDefId>,
}

#[derive(Clone, Debug)]
pub struct OwnedDef {
    pub id: GlobalDefId,
    pub name: String,
    pub kind: super::db::ItemKind,
    pub vis: super::db::VisKind,
    pub has_value_ctor: bool,
}

#[derive(Clone, Debug)]
pub struct OwnedUse {
    pub name: String,
    pub type_def: GlobalDefId,
    pub value_def: GlobalDefId,
    pub macro_def: GlobalDefId,
    pub vis: super::db::VisKind,
    pub is_glob: bool,
    pub is_reexport: bool,
    pub is_prelude: bool,
    pub is_underscore: bool,
    pub prim_ty_code: u8,
}

impl MemItemDb {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_crate(&mut self, name: impl Into<String>, edition: Edition) -> CrateId {
        let id = CrateId(self.crates.len() as u32);
        self.set_crate(id, name, edition);
        id
    }

    pub fn set_crate(&mut self, krate: CrateId, name: impl Into<String>, edition: Edition) {
        let e = self.crates.entry(krate).or_default();
        e.name = name.into();
        e.edition = edition;
    }

    pub fn add_cfg_flag(&mut self, krate: CrateId, key: impl Into<String>) {
        let k = key.into();
        let list = &mut self.crates.entry(krate).or_default().cfg_flags;
        if let Err(pos) = list.binary_search(&k) {
            list.insert(pos, k);
        }
    }

    pub fn add_cfg_kv(&mut self, krate: CrateId, key: impl Into<String>, value: impl Into<String>) {
        let pair = (key.into(), value.into());
        let list = &mut self.crates.entry(krate).or_default().cfg_kvs;
        if let Err(pos) = list.binary_search(&pair) {
            list.insert(pos, pair);
        }
    }

    pub fn add_macro_rules(&mut self, krate: CrateId, call_path: impl Into<String>, body: impl Into<String>) {
        self.crates.entry(krate).or_default().macros.insert(call_path.into(), body.into());
    }

    pub fn add_extern_crate(&mut self, from_krate: CrateId, extern_name: impl Into<String>, target: CrateId) {
        self.crates.entry(from_krate).or_default().externs.insert(extern_name.into(), target);
    }

    pub fn set_crate_root_def(&mut self, krate: CrateId, def: GlobalDefId) {
        self.crates.entry(krate).or_default().root_def = Some(def);
        self.def_kinds.entry(def).or_insert(super::db::ItemKind::Mod);
    }

    pub fn set_prelude_module(&mut self, krate: CrateId, prelude: GlobalDefId) {
        self.crates.entry(krate).or_default().prelude_mod = Some(prelude);
        self.def_kinds.entry(prelude).or_insert(super::db::ItemKind::Mod);
    }

    pub fn set_def_kind(&mut self, def: GlobalDefId, kind: super::db::ItemKind) {
        self.def_kinds.insert(def, kind);
    }

    pub fn set_def_file(&mut self, def: GlobalDefId, file: FileId) {
        self.def_files.insert(def, file);
    }

    pub fn set_type_alias_target(&mut self, alias_def: GlobalDefId, target: ImplTargetKey) {
        self.alias_targets.insert(alias_def, target);
    }

    pub fn add_impl(&mut self, imp: IndexedImpl) {
        self.def_kinds.insert(imp.id, super::db::ItemKind::Impl);
        if imp.file.is_some() {
            self.def_files.insert(imp.id, imp.file);
        }
        self.impls_by_target.entry(imp.target).or_default().push(imp);
        self.impls_by_crate.entry(imp.id.krate).or_default().push(imp);
    }

    pub fn add_module_def_in_file(&mut self, module_def: GlobalDefId, file: FileId, def: OwnedDef) {
        if file.is_some() {
            self.def_files.insert(def.id, file);
        }
        self.add_module_def(module_def, def);
    }

    pub fn add_module_def(&mut self, module_def: GlobalDefId, def: OwnedDef) {
        self.def_kinds.entry(module_def).or_insert(super::db::ItemKind::Mod);
        self.def_kinds.insert(def.id, def.kind);
        self.module_defs.entry(module_def).or_default().push(def);
    }

    pub fn add_module_use(&mut self, module_def: GlobalDefId, u: OwnedUse) {
        self.module_uses.entry(module_def).or_default().push(u);
    }

    pub fn add_assoc_item(&mut self, owner_def: GlobalDefId, item: OwnedDef) {
        self.def_kinds.insert(item.id, item.kind);
        self.assoc_items.entry(owner_def).or_default().push(item);
    }

    pub fn set_lang_item(&mut self, lang_kind: u16, def: GlobalDefId) {
        self.lang_items.insert(lang_kind, def);
    }
}

impl ItemDb for MemItemDb {
    #[inline]
    fn cfg_flag(&self, krate: CrateId, key: &str) -> bool {
        self.crates.get(&krate).is_some_and(|c| c.cfg_flags.binary_search_by(|k| k.as_str().cmp(key)).is_ok())
    }

    #[inline]
    fn cfg_key_value(&self, krate: CrateId, key: &str, value: &str) -> bool {
        self.crates.get(&krate).is_some_and(|c| c.cfg_kvs.binary_search_by(|(k, v)| k.as_str().cmp(key).then_with(|| v.as_str().cmp(value))).is_ok())
    }

    #[inline]
    fn crate_edition(&self, krate: CrateId) -> Edition {
        self.crates.get(&krate).map_or(Edition::Edition2021, |c| c.edition)
    }

    #[inline]
    fn crate_name(&self, krate: CrateId) -> &str {
        self.crates.get(&krate).map_or("", |c| c.name.as_str())
    }

    #[inline]
    fn extern_crate(&self, from_krate: CrateId, extern_name: &str) -> Option<CrateId> {
        self.crates.get(&from_krate)?.externs.get(extern_name).copied()
    }

    fn extern_crates<'a>(&'a self, from_krate: CrateId, out: &mut Vec<(&'a str, CrateId)>) {
        if let Some(c) = self.crates.get(&from_krate) {
            out.extend(c.externs.iter().map(|(k, &v)| (k.as_str(), v)));
        }
    }

    #[inline]
    fn crate_root_def(&self, krate: CrateId) -> Option<GlobalDefId> {
        self.crates.get(&krate)?.root_def
    }

    #[inline]
    fn prelude_module(&self, krate: CrateId) -> Option<GlobalDefId> {
        self.crates.get(&krate)?.prelude_mod
    }

    #[inline]
    fn macro_rules_body(&self, krate: CrateId, _module_path: &str, call_path: &str) -> Option<&str> {
        let m = &self.crates.get(&krate)?.macros;
        m.get(call_path).or_else(|| call_path.rsplit("::").next().and_then(|s| m.get(s))).map(String::as_str)
    }

    #[inline]
    fn def_kind(&self, def: GlobalDefId) -> Option<super::db::ItemKind> {
        self.def_kinds.get(&def).copied()
    }

    #[inline]
    fn def_file(&self, def: GlobalDefId) -> FileId {
        self.def_files.get(&def).copied().unwrap_or(FileId::NONE)
    }

    #[inline]
    fn type_alias_target(&self, alias_def: GlobalDefId) -> Option<ImplTargetKey> {
        self.alias_targets.get(&alias_def).copied()
    }

    fn impls_for_target(&self, target: ImplTargetKey, out: &mut Vec<IndexedImpl>) {
        if let Some(list) = self.impls_by_target.get(&target) {
            out.extend_from_slice(list);
        }
    }

    fn crate_impls(&self, krate: CrateId, out: &mut Vec<IndexedImpl>) {
        if let Some(list) = self.impls_by_crate.get(&krate) {
            out.extend_from_slice(list);
        }
    }

    fn module_defs<'a>(&'a self, module_def: GlobalDefId, out: &mut Vec<IndexedDef<'a>>) {
        if let Some(list) = self.module_defs.get(&module_def) {
            out.extend(list.iter().map(|d| IndexedDef { id: d.id, name: &d.name, kind: d.kind, vis: d.vis, has_value_ctor: d.has_value_ctor }));
        }
    }

    fn enum_variants<'a>(&'a self, enum_def: GlobalDefId, out: &mut Vec<IndexedDef<'a>>) {
        self.module_defs(enum_def, out);
        if let Some(list) = self.assoc_items.get(&enum_def) {
            out.extend(list.iter().filter(|d| d.kind == super::db::ItemKind::Variant).map(|d| IndexedDef {
                id: d.id,
                name: &d.name,
                kind: d.kind,
                vis: d.vis,
                has_value_ctor: d.has_value_ctor,
            }));
        }
    }

    fn module_uses<'a>(&'a self, module_def: GlobalDefId, out: &mut Vec<IndexedUse<'a>>) {
        if let Some(list) = self.module_uses.get(&module_def) {
            out.extend(list.iter().map(|u| IndexedUse {
                name: &u.name,
                type_def: u.type_def,
                value_def: u.value_def,
                macro_def: u.macro_def,
                vis: u.vis,
                is_glob: u.is_glob,
                is_reexport: u.is_reexport,
                is_prelude: u.is_prelude,
                is_underscore: u.is_underscore,
                prim_ty_code: u.prim_ty_code,
            }));
        }
    }

    fn assoc_items<'a>(&'a self, owner_def: GlobalDefId, out: &mut Vec<IndexedDef<'a>>) {
        if let Some(list) = self.assoc_items.get(&owner_def) {
            out.extend(list.iter().map(|d| IndexedDef { id: d.id, name: &d.name, kind: d.kind, vis: d.vis, has_value_ctor: d.has_value_ctor }));
        }
    }

    fn adt_field(&self, adt_or_variant_def: GlobalDefId, field_name: &str) -> Option<GlobalDefId> {
        self.assoc_items.get(&adt_or_variant_def)?.iter().find(|d| d.kind == super::db::ItemKind::Field && d.name == field_name).map(|d| d.id)
    }

    fn lang_item(&self, lang_kind: u16) -> Option<GlobalDefId> {
        self.lang_items.get(&lang_kind).copied()
    }
}
