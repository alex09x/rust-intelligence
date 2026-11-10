/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Pre-indexed `WorkspaceIndex` (`v5` `.ridx`) adapter state (`RidxV5ItemDb`) mapping
//! compiler shards, `TyArena` definitions/types, and `ImplEntry` tables onto `ItemDb`.

use std::collections::{HashMap, HashSet};

use ridx_format::{
    DefKind, FloatTyWidth, IntTyWidth, TyArenaView, TyDefId, TyId, TyKindView, TyMutability, UintTyWidth, UseOwner, WherePredKind, WorkspaceIndex, IMPL_FLAG_NEGATIVE,
};
use ridx_syntax::name_res::{eval_outer_attrs, CrateId, FileId, GlobalDefId, ImplTargetKey, IndexedImpl, PrimTy};
use ridx_syntax::{parse, SyntaxKind as K};

use super::meta::{CorpusSourceDb, ReaderMeta};
use super::v5_lazy::SourceStore;
use super::v5_shard::{build_shard_tables, collect_shard_alias_pairs, ShardTables};
use super::v5_ty_tables::V5TyTables;

/// Sentinel `def_idx` representing a shard's crate-root module (`UseOwner::CrateRoot`).
pub const CRATE_ROOT_DEF_IDX: u32 = u32::MAX - 1;
/// Bit flag distinguishing synthetic `ImplEntry` indices (`def_index == None`) in `GlobalDefId::index`.
pub const IMPL_SYNTH_BIT: u32 = 0x8000_0000;
/// Bit flag distinguishing synthetic `#[proc_macro_derive]` / `#[proc_macro]` root exports.
pub const PROC_MACRO_SYNTH_BIT: u32 = 0x4000_0000;
/// Bit flag distinguishing unmaterialized `TyDefEntry` items (`local_def_idx == 0x7FFF_FFFF`).
pub const TY_DEF_SYNTH_BIT: u32 = 0x2000_0000;

pub struct RidxV5ItemDb<'w> {
    pub ws: &'w WorkspaceIndex,
    pub src: SourceStore<'w>,
    pub metas: Vec<ReaderMeta>,
    pub shards: Vec<ShardTables>,
    pub shard_loaded: Vec<bool>,
    pub sysroot_by_name: HashMap<String, usize>,
    pub impls_by_adt: HashMap<GlobalDefId, Vec<IndexedImpl>>,
    pub impls_by_other: HashMap<ImplTargetKey, Vec<IndexedImpl>>,
    pub impls_by_crate: Vec<Vec<IndexedImpl>>,
    pub trait_supertraits: HashMap<GlobalDefId, Vec<GlobalDefId>>,
    pub impl_assoc_parent: HashMap<GlobalDefId, (GlobalDefId, ImplTargetKey)>,
    pub impl_self_target: HashMap<GlobalDefId, ImplTargetKey>,
    pub valid_alias_leaf_pairs: HashSet<(String, String)>,
    pub ty_tables: V5TyTables<'w>,
    pub active_owner_def: std::cell::Cell<GlobalDefId>,
}

impl<'w> RidxV5ItemDb<'w> {
    pub fn build(ws: &'w WorkspaceIndex, src: &'w CorpusSourceDb, metas: Vec<ReaderMeta>) -> Self {
        let n = ws.crate_count();
        let mut sysroot_by_name = HashMap::new();
        for (idx, meta) in metas.iter().enumerate() {
            if matches!(meta.crate_name.as_str(), "core" | "alloc" | "std" | "proc_macro") {
                sysroot_by_name.entry(meta.crate_name.clone()).or_insert(idx);
            }
        }

        let mut shards = Vec::with_capacity(n);
        let mut valid_alias_leaf_pairs = HashSet::new();
        for (rdr, meta) in ws.readers().iter().zip(metas.iter()).take(n) {
            shards.push(build_shard_tables(rdr, meta, src));
            collect_shard_alias_pairs(rdr, &mut valid_alias_leaf_pairs);
        }

        let all: Vec<usize> = (0..n).collect();
        let mut db = Self {
            ws,
            src: SourceStore::Borrowed(src),
            metas,
            shards,
            shard_loaded: vec![true; n],
            sysroot_by_name,
            impls_by_adt: HashMap::new(),
            impls_by_other: HashMap::new(),
            impls_by_crate: vec![Vec::new(); n],
            trait_supertraits: HashMap::new(),
            impl_assoc_parent: HashMap::new(),
            impl_self_target: HashMap::new(),
            valid_alias_leaf_pairs,
            ty_tables: V5TyTables::default(),
            active_owner_def: std::cell::Cell::new(GlobalDefId::NONE),
        };
        db.ty_tables.ty_def_gids.resize_with(n, Vec::new);
        db.index_impls_and_traits_for(&all);
        db.index_proc_macro_exports_for(&all);
        db.build_ty_tables_for(&all);
        db
    }

    fn register_synth_macro(&mut self, r_idx: usize, name: &str) {
        if name.is_empty() || self.shards[r_idx].synth_macro_by_name.contains_key(name) {
            return;
        }
        let reader = &self.ws.readers()[r_idx];
        if lookup_def_idx_by_exact_path(reader, &self.shards[r_idx], name).is_some_and(|didx| DefKind::from(reader.definitions()[didx as usize].kind) == DefKind::Macro) {
            return;
        }
        let shard = &mut self.shards[r_idx];
        let sym_idx = shard.synth_macros.len() as u32;
        let didx = PROC_MACRO_SYNTH_BIT | sym_idx;
        shard.synth_macros.push(name.to_string());
        shard.synth_macro_by_name.insert(name.to_string(), didx);
        let root_key = shard.root_mod_def_idx.unwrap_or(CRATE_ROOT_DEF_IDX);
        shard.mod_children.entry(root_key).or_default().push(didx);
    }

    pub(crate) fn index_proc_macro_exports_for(&mut self, active: &[usize]) {
        let mut pending: Vec<(usize, String)> = Vec::new();
        for &r_idx in active {
            if !self.metas[r_idx].is_dependency {
                let krate = CrateId(r_idx as u32);
                let mut fids: Vec<FileId> = self.shards[r_idx].file_map.iter().copied().filter(|f| f.is_some()).collect();
                fids.sort_unstable_by_key(|f| f.0);
                fids.dedup();
                for fid in fids {
                    let st = parse(&self.src.files[fid.0 as usize].text);
                    for nid in (0..st.nodes.len() as u32).map(ridx_syntax::NodeId) {
                        if st.kind(nid) != K::Fn {
                            continue;
                        }
                        let attrs = eval_outer_attrs(&st, nid, self, krate);
                        if !attrs.enabled {
                            continue;
                        }
                        let pm = attrs
                            .proc_macro_derive_name
                            .or_else(|| attrs.is_proc_macro_fn.then(|| st.child(nid, K::Name).map(|nn| st.text(nn).trim_start_matches("r#"))).flatten());
                        if let Some(mname) = pm {
                            pending.push((r_idx, mname.to_string()));
                        }
                    }
                }
            }
            let reader = &self.ws.readers()[r_idx];
            let arena = reader.ty_arena();
            let mut owners = vec![UseOwner::CrateRoot];
            for (didx, d) in reader.definitions().iter().enumerate() {
                if !d.is_reexport() && DefKind::from(d.kind) == DefKind::Mod {
                    owners.push(UseOwner::Def(didx as u32));
                }
            }
            for owner in owners {
                for u in reader.uses_for_owner(owner) {
                    let Some(td_id) = u.macro_def else {
                        continue;
                    };
                    if self.resolve_ty_def(r_idx, arena, td_id).is_some() {
                        continue;
                    }
                    let Some(td) = arena.def(td_id) else {
                        continue;
                    };
                    let tr_opt = td.extern_unit.and_then(|uk| self.resolve_unit_to_reader(r_idx, uk)).or_else(|| {
                        let first = td.path.split("::").next().unwrap_or(td.path);
                        self.resolve_extern_name_to_reader(r_idx, first)
                    });
                    if let Some(tr_idx) = tr_opt {
                        let leaf = td.path.rsplit("::").next().unwrap_or(td.path).trim_start_matches("r#");
                        if !leaf.is_empty() {
                            pending.push((tr_idx, leaf.to_string()));
                        }
                    }
                }
            }
        }
        for (tr_idx, name) in pending {
            self.register_synth_macro(tr_idx, &name);
        }
    }

    pub(crate) fn index_impls_and_traits_for(&mut self, active: &[usize]) {
        for &r_idx in active {
            let reader = &self.ws.readers()[r_idx];
            let arena = reader.ty_arena();
            let krate = CrateId(r_idx as u32);
            for iv in reader.impls_iter() {
                let (id_index, impl_file) = match iv.def_index() {
                    Some(didx) => {
                        let f = reader.definitions().get(didx as usize).and_then(|d| self.shards[r_idx].file_map.get(d.span().file_id() as usize).copied()).unwrap_or(FileId::NONE);
                        (didx, f)
                    }
                    None => (IMPL_SYNTH_BIT | iv.index(), FileId::NONE),
                };
                let target = self.lower_ty_to_target_key(r_idx, arena, iv.self_ty(), 0);
                let impl_gid = GlobalDefId::new(krate, id_index);
                if target != ImplTargetKey::Unknown {
                    self.impl_self_target.insert(impl_gid, target);
                }
                let trait_def = iv.trait_def().map_or(GlobalDefId::NONE, |td| self.resolve_ty_def(r_idx, arena, td));
                for assoc_didx in iv.assoc_def_indices() {
                    self.impl_assoc_parent.insert(GlobalDefId::new(krate, assoc_didx), (trait_def, target));
                }
                if target == ImplTargetKey::Unknown {
                    continue;
                }
                let is_negative = (iv.flags() & IMPL_FLAG_NEGATIVE) != 0;
                let entry = IndexedImpl { id: impl_gid, file: impl_file, target, trait_def, is_negative };
                self.impls_by_crate[r_idx].push(entry);
                match target {
                    ImplTargetKey::Adt(adt_def) => self.impls_by_adt.entry(adt_def).or_default().push(entry),
                    other => self.impls_by_other.entry(other).or_default().push(entry),
                }
            }
            for (didx, d) in reader.definitions().iter().enumerate() {
                if d.is_reexport() || DefKind::from(d.kind) != DefKind::Trait {
                    continue;
                }
                let trait_gid = GlobalDefId::new(krate, didx as u32);
                let mut supers = Vec::new();
                for wp in reader.where_preds_for_def(didx as u32) {
                    if wp.kind() != WherePredKind::TraitBound || wp.is_negative() {
                        continue;
                    }
                    if matches!(arena.kind(wp.lhs_ty()), TyKindView::Param { index: 0, .. }) {
                        if let Some(td) = wp.trait_def() {
                            let sg = self.resolve_ty_def(r_idx, arena, td);
                            if sg.is_some() && sg != trait_gid && !supers.contains(&sg) {
                                supers.push(sg);
                            }
                        }
                    }
                }
                if !supers.is_empty() {
                    self.trait_supertraits.insert(trait_gid, supers);
                }
            }
        }
    }

    /// Resolves a shard-local or cross-crate `TyDefId` into a `GlobalDefId`.
    pub fn resolve_ty_def(&self, from_reader: usize, arena: TyArenaView<'_>, td_id: TyDefId) -> GlobalDefId {
        let Some(td) = arena.def(td_id) else {
            return GlobalDefId::NONE;
        };
        if let Some(local_idx) = td.local_def_idx {
            if (local_idx as usize) < self.ws.readers()[from_reader].definitions().len() {
                return GlobalDefId::new(CrateId(from_reader as u32), local_idx);
            }
            let by_path = self.find_def_in_reader_by_path(from_reader, td.path);
            if by_path.is_some() {
                return by_path;
            }
            return GlobalDefId::new(CrateId(from_reader as u32), local_idx);
        }
        if td.extern_unit.is_none() && (td.path == self.metas[from_reader].crate_name || td.path.is_empty()) {
            return self.crate_root_gid(from_reader);
        }
        let target_reader = td.extern_unit.and_then(|uk| self.resolve_unit_to_reader(from_reader, uk)).or_else(|| {
            let first_seg = td.path.split("::").next().unwrap_or(td.path);
            self.resolve_extern_name_to_reader(from_reader, first_seg).or((first_seg == self.metas[from_reader].crate_name).then_some(from_reader))
        });
        let Some(tr_idx) = target_reader else {
            return GlobalDefId::NONE;
        };
        self.find_def_in_reader_by_path(tr_idx, td.path)
    }

    #[inline]
    pub fn crate_root_gid(&self, r_idx: usize) -> GlobalDefId {
        let idx = self.shards[r_idx].root_mod_def_idx.unwrap_or(CRATE_ROOT_DEF_IDX);
        GlobalDefId::new(CrateId(r_idx as u32), idx)
    }

    pub fn resolve_unit_to_reader(&self, _from_reader: usize, unit_key: &str) -> Option<usize> {
        if let Some(&sr) = self.sysroot_by_name.get(unit_key) {
            return Some(sr);
        }
        self.ws.unit_match().readers_for_unit(unit_key).and_then(|s| s.first().copied())
    }

    pub fn resolve_extern_name_to_reader(&self, from_reader: usize, extern_name: &str) -> Option<usize> {
        if let Some(uk) = self.metas.get(from_reader).and_then(|m| m.extern_units.get(extern_name)) {
            if let Some(r) = self.resolve_unit_to_reader(from_reader, uk) {
                return Some(r);
            }
        }
        if let Some(&sr) = self.sysroot_by_name.get(extern_name) {
            return Some(sr);
        }
        None
    }

    pub fn find_def_in_reader_by_path(&self, r_idx: usize, raw_path: &str) -> GlobalDefId {
        let reader = &self.ws.readers()[r_idx];
        let cname = self.metas[r_idx].crate_name.as_str();
        if raw_path.is_empty() || raw_path == cname {
            return self.crate_root_gid(r_idx);
        }
        let follow_reexport = |didx: u32| -> GlobalDefId {
            if self.metas[r_idx].is_sysroot {
                if let Some(d) = reader.definitions().get(didx as usize) {
                    if d.is_reexport() {
                        let sig = reader.get_str(d.sig_offset());
                        let sig_first = sig.split("::").next().unwrap_or("");
                        if !sig_first.is_empty() && sig_first != cname {
                            if let Some(&sr) = self.sysroot_by_name.get(sig_first) {
                                let g = self.find_def_in_reader_by_path(sr, sig);
                                if g.is_some() {
                                    return g;
                                }
                            }
                        }
                    }
                }
            }
            GlobalDefId::new(CrateId(r_idx as u32), didx)
        };
        let shard = &self.shards[r_idx];
        if let Some(didx) = lookup_def_idx_by_exact_path(reader, shard, raw_path) {
            return follow_reexport(didx);
        }
        if let Some(stripped) = raw_path.strip_prefix(cname).and_then(|s| s.strip_prefix("::")) {
            if stripped.is_empty() {
                return self.crate_root_gid(r_idx);
            }
            if let Some(didx) = lookup_def_idx_by_exact_path(reader, shard, stripped) {
                return follow_reexport(didx);
            }
        }
        let prefixed = format!("{cname}::{raw_path}");
        if let Some(didx) = lookup_def_idx_by_exact_path(reader, shard, &prefixed) {
            return follow_reexport(didx);
        }
        if let Some(&didx) = self.shards[r_idx].reexport_sig_to_idx.get(raw_path).or_else(|| self.shards[r_idx].reexport_sig_to_idx.get(&prefixed)) {
            return follow_reexport(didx);
        }
        let rel_cand = raw_path.strip_prefix(cname).and_then(|s| s.strip_prefix("::")).unwrap_or(raw_path);
        if let Some(&didx) = self.shards[r_idx].synth_macro_by_name.get(rel_cand) {
            return GlobalDefId::new(CrateId(r_idx as u32), didx);
        }
        if let Some(&didx) = self.shards[r_idx].synth_ty_def_by_path.get(raw_path).or_else(|| self.shards[r_idx].synth_ty_def_by_path.get(&prefixed)) {
            return GlobalDefId::new(CrateId(r_idx as u32), didx);
        }
        GlobalDefId::NONE
    }

    pub fn lower_ty_to_target_key(&self, r_idx: usize, arena: TyArenaView<'_>, ty_id: TyId, depth: u8) -> ImplTargetKey {
        if depth > 6 {
            return ImplTargetKey::Unknown;
        }
        match arena.kind(ty_id) {
            TyKindView::Unit => ImplTargetKey::Tuple(0),
            TyKindView::Bool => ImplTargetKey::Prim(PrimTy::Bool),
            TyKindView::Char => ImplTargetKey::Prim(PrimTy::Char),
            TyKindView::Str => ImplTargetKey::Prim(PrimTy::Str),
            TyKindView::Never => ImplTargetKey::Never,
            TyKindView::Int(w) => match w {
                IntTyWidth::I8 => ImplTargetKey::Prim(PrimTy::I8),
                IntTyWidth::I16 => ImplTargetKey::Prim(PrimTy::I16),
                IntTyWidth::I32 => ImplTargetKey::Prim(PrimTy::I32),
                IntTyWidth::I64 => ImplTargetKey::Prim(PrimTy::I64),
                IntTyWidth::I128 => ImplTargetKey::Prim(PrimTy::I128),
                IntTyWidth::Isize => ImplTargetKey::Prim(PrimTy::Isize),
            },
            TyKindView::Uint(w) => match w {
                UintTyWidth::U8 => ImplTargetKey::Prim(PrimTy::U8),
                UintTyWidth::U16 => ImplTargetKey::Prim(PrimTy::U16),
                UintTyWidth::U32 => ImplTargetKey::Prim(PrimTy::U32),
                UintTyWidth::U64 => ImplTargetKey::Prim(PrimTy::U64),
                UintTyWidth::U128 => ImplTargetKey::Prim(PrimTy::U128),
                UintTyWidth::Usize => ImplTargetKey::Prim(PrimTy::Usize),
            },
            TyKindView::Float(FloatTyWidth::F32) => ImplTargetKey::Prim(PrimTy::F32),
            TyKindView::Float(FloatTyWidth::F64) => ImplTargetKey::Prim(PrimTy::F64),
            TyKindView::Adt { def, .. } => {
                let g = self.resolve_ty_def(r_idx, arena, def);
                if g.is_some() {
                    ImplTargetKey::Adt(g)
                } else {
                    ImplTargetKey::Unknown
                }
            }
            TyKindView::Ref { mutability, .. } => ImplTargetKey::Ref { is_mut: mutability == TyMutability::Mut },
            TyKindView::Ptr { mutability, .. } => ImplTargetKey::Ptr { is_mut: mutability == TyMutability::Mut },
            TyKindView::Slice(_) => ImplTargetKey::Slice,
            TyKindView::Array { .. } => ImplTargetKey::Array,
            TyKindView::Tuple(list) => ImplTargetKey::Tuple(list.count() as u16),
            TyKindView::FnPtr { inputs, .. } => ImplTargetKey::FnPtr { arity: inputs.count() as u16 },
            TyKindView::Dyn { mut bounds, .. } => {
                bounds.next().map(|f| self.resolve_ty_def(r_idx, arena, f.trait_def)).filter(|g| g.is_some()).map_or(ImplTargetKey::Unknown, ImplTargetKey::Dyn)
            }
            TyKindView::Param { .. } => ImplTargetKey::Param,
            _ => ImplTargetKey::Unknown,
        }
    }
}

pub use super::v5_shard::lookup_def_idx_by_exact_path;
