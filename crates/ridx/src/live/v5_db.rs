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

use ridx_format::{DefKind, TyArenaView, TyDefId, TyKindView, UseOwner, WherePredKind, WorkspaceIndex};
use ridx_syntax::name_res::{eval_outer_attrs, CrateId, FileId, GlobalDefId, ImplTargetKey, IndexedImpl};
use ridx_syntax::{parse, SyntaxKind as K};

use super::meta::{CorpusSourceDb, ReaderMeta};
use super::v5_lazy::{AtomicDefCell, LiveStateRef, SourceStore};
use super::v5_shard::{build_shard_tables, collect_shard_alias_pairs, ShardTables};
use super::v5_ty_inst::slice_flat_group;
use super::v5_ty_tables::V5TyTables;

/// Sentinel `def_idx` representing a shard's crate-root module (`UseOwner::CrateRoot`).
pub const CRATE_ROOT_DEF_IDX: u32 = u32::MAX - 1;
/// Bit flag distinguishing synthetic `ImplEntry` indices (`def_index == None`) in `GlobalDefId::index`.
pub const IMPL_SYNTH_BIT: u32 = 0x8000_0000;
/// Bit flag distinguishing synthetic `#[proc_macro_derive]` / `#[proc_macro]` root exports.
pub const PROC_MACRO_SYNTH_BIT: u32 = 0x4000_0000;
/// Bit flag distinguishing unmaterialized `TyDefEntry` items (`local_def_idx == 0x7FFF_FFFF`).
pub const TY_DEF_SYNTH_BIT: u32 = 0x2000_0000;

/// Owned, lifetime-free derived shard and type tables retained across LSP requests by [`super::query::LiveDb`].
pub struct LiveState {
    pub bench_mode: bool,
    pub metas: Vec<ReaderMeta>,
    pub meta_loaded: Vec<bool>,
    pub shards: Vec<ShardTables>,
    pub shard_loaded: Vec<bool>,
    pub shard_expanded_depth: Vec<u8>,
    pub units_by_file: HashMap<std::path::PathBuf, Vec<(usize, u32)>>,
    pub sysroot_by_name: HashMap<String, usize>,
    pub impls_arena: Vec<IndexedImpl>,
    pub impls_by_adt: HashMap<GlobalDefId, (u32, u32)>,
    pub impls_by_other: HashMap<ImplTargetKey, (u32, u32)>,
    pub trait_supertraits: HashMap<GlobalDefId, Vec<GlobalDefId>>,
    pub impl_assoc_parent: HashMap<GlobalDefId, (GlobalDefId, ImplTargetKey)>,
    pub impl_self_target: HashMap<GlobalDefId, ImplTargetKey>,
    pub valid_alias_leaf_pairs: HashSet<(String, String)>,
    pub ty_tables: V5TyTables,
    pub active_owner_def: AtomicDefCell,
}

impl LiveState {
    #[inline]
    pub fn ensure_meta(&mut self, ws: &WorkspaceIndex, idx: usize) {
        if !self.meta_loaded[idx] {
            self.meta_loaded[idx] = true;
            let has_core = self.sysroot_by_name.contains_key("core");
            self.metas[idx] = super::meta::load_reader_meta_at(ws, idx, has_core, true);
        }
    }

    #[inline]
    pub fn adt_impls(&self, adt: GlobalDefId) -> &[IndexedImpl] {
        slice_flat_group(&self.impls_arena, &self.impls_by_adt, &adt)
    }

    #[inline]
    pub fn other_impls(&self, key: ImplTargetKey) -> &[IndexedImpl] {
        slice_flat_group(&self.impls_arena, &self.impls_by_other, &key)
    }
}

/// Temporary query view pairing `&'a WorkspaceIndex` with [`LiveState`] and [`CorpusSourceDb`].
pub struct RidxV5ItemDb<'a> {
    pub ws: &'a WorkspaceIndex,
    pub src: SourceStore<'a>,
    pub state: LiveStateRef<'a>,
}

impl std::ops::Deref for RidxV5ItemDb<'_> {
    type Target = LiveState;
    #[inline]
    fn deref(&self) -> &LiveState {
        &self.state
    }
}

impl<'a> RidxV5ItemDb<'a> {
    #[inline]
    pub fn view(ws: &'a WorkspaceIndex, src: &'a CorpusSourceDb, state: &'a LiveState) -> Self {
        Self { ws, src: SourceStore::Borrowed(src), state: LiveStateRef::Borrowed(state) }
    }

    #[inline]
    pub(crate) fn view_mut(ws: &'a WorkspaceIndex, src: &'a mut CorpusSourceDb, state: &'a mut LiveState) -> Self {
        Self { ws, src: SourceStore::BorrowedMut(src), state: LiveStateRef::BorrowedMut(state) }
    }

    pub fn build(ws: &'a WorkspaceIndex, src: &'a CorpusSourceDb, metas: Vec<ReaderMeta>) -> Self {
        let n = ws.crate_count();
        let mut sysroot_by_name = HashMap::new();
        for (idx, meta) in metas.iter().enumerate() {
            if matches!(meta.crate_name.as_str(), "core" | "alloc" | "std" | "proc_macro") {
                sysroot_by_name.entry(meta.crate_name.clone()).or_insert(idx);
            }
        }
        let mut shards = Vec::with_capacity(n);
        let mut valid_alias_leaf_pairs = HashSet::new();
        for (idx, (rdr, meta)) in ws.readers().iter().zip(metas.iter()).enumerate().take(n) {
            shards.push(build_shard_tables(idx, rdr, meta, src, true));
            collect_shard_alias_pairs(rdr, &mut valid_alias_leaf_pairs);
        }
        let mut ty_tables = V5TyTables::default();
        ty_tables.raw_ty_def_gids.resize_with(n, Vec::new);
        ty_tables.ty_def_gids.resize_with(n, Vec::new);
        let state = LiveState {
            bench_mode: true,
            metas,
            meta_loaded: vec![true; n],
            shards,
            shard_loaded: vec![true; n],
            shard_expanded_depth: vec![0; n],
            units_by_file: HashMap::new(),
            sysroot_by_name,
            impls_arena: Vec::new(),
            impls_by_adt: HashMap::new(),
            impls_by_other: HashMap::new(),
            trait_supertraits: HashMap::new(),
            impl_assoc_parent: HashMap::new(),
            impl_self_target: HashMap::new(),
            valid_alias_leaf_pairs,
            ty_tables,
            active_owner_def: AtomicDefCell::new(GlobalDefId::NONE),
        };
        let all: Vec<usize> = (0..n).collect();
        let mut db = Self { ws, src: SourceStore::Borrowed(src), state: LiveStateRef::Owned(Box::new(state)) };
        db.index_proc_macro_exports_for(&all);
        db.populate_raw_ty_def_gids_for(&all);
        db.index_impls_and_traits_for(&all);
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
        let shard = &mut self.state.as_mut().shards[r_idx];
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
            let meta = &self.metas[r_idx];
            let may_define_proc_macros =
                !meta.is_dependency && (meta.target_kind.contains("proc-macro") || (meta.target_kind.is_empty() && self.ws.meta_path_at(r_idx).is_none() && !meta.is_sysroot));
            if may_define_proc_macros {
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
            let is_dep = self.metas[r_idx].is_dependency;
            let mut owners = vec![UseOwner::CrateRoot];
            if !is_dep {
                for (didx, d) in reader.definitions().iter().enumerate() {
                    if !d.is_reexport() && DefKind::from(d.kind) == DefKind::Mod {
                        owners.push(UseOwner::Def(didx as u32));
                    }
                }
            }
            for owner in owners {
                for u in reader.uses_for_owner(owner) {
                    if is_dep && (u.flags & ridx_format::USE_FLAG_REEXPORT) == 0 {
                        continue;
                    }
                    let Some(td_id) = u.macro_def.filter(|&id| !self.resolve_ty_def(r_idx, arena, id).is_some()) else { continue };
                    let Some(td) = arena.def(td_id) else { continue };
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
        let ws = self.ws;
        for &r_idx in active {
            let reader = &ws.readers()[r_idx];
            let arena = reader.ty_arena();
            let krate = CrateId(r_idx as u32);
            for (didx, d) in reader.definitions().iter().enumerate() {
                if d.is_reexport() || DefKind::from(d.kind) != DefKind::Trait {
                    continue;
                }
                let trait_gid = GlobalDefId::new(krate, didx as u32);
                let mut supers = Vec::new();
                for wp in reader.where_preds_for_def(didx as u32) {
                    if wp.kind() == WherePredKind::TraitBound && !wp.is_negative() && matches!(arena.kind(wp.lhs_ty()), TyKindView::Param { index: 0, .. }) {
                        if let Some(td) = wp.trait_def() {
                            let sg = self.resolve_ty_def(r_idx, arena, td);
                            if sg.is_some() && sg != trait_gid && !supers.contains(&sg) {
                                supers.push(sg);
                            }
                        }
                    }
                }
                if !supers.is_empty() {
                    self.state.as_mut().trait_supertraits.insert(trait_gid, supers);
                }
            }
        }
    }

    /// Resolves a shard-local or cross-crate `TyDefId` into a `GlobalDefId`.
    #[inline]
    pub fn resolve_ty_def(&self, from_reader: usize, arena: TyArenaView<'_>, td_id: TyDefId) -> GlobalDefId {
        self.ty_tables.raw_ty_def_gids.get(from_reader).and_then(|v| v.get(td_id.0 as usize)).copied().unwrap_or_else(|| self.resolve_ty_def_uncached(from_reader, arena, td_id))
    }

    pub(crate) fn resolve_ty_def_uncached(&self, from_reader: usize, arena: TyArenaView<'_>, td_id: TyDefId) -> GlobalDefId {
        let Some(td) = arena.def(td_id) else { return GlobalDefId::NONE };
        if let Some(local_idx) = td.local_def_idx {
            if (local_idx as usize) < self.ws.readers()[from_reader].definitions().len() {
                return GlobalDefId::new(CrateId(from_reader as u32), local_idx);
            }
            let by_path = self.type_ns_def(self.find_def_in_reader_by_path(from_reader, td.path));
            return if by_path.is_some() { by_path } else { GlobalDefId::new(CrateId(from_reader as u32), local_idx) };
        }
        if td.extern_unit.is_none() && (td.path == self.metas[from_reader].crate_name || td.path.is_empty()) {
            return self.crate_root_gid(from_reader);
        }
        let target_reader = td.extern_unit.and_then(|uk| self.resolve_unit_to_reader(from_reader, uk)).or_else(|| {
            let first_seg = td.path.split("::").next().unwrap_or(td.path);
            self.resolve_extern_name_to_reader(from_reader, first_seg).or((first_seg == self.metas[from_reader].crate_name).then_some(from_reader))
        });
        let Some(tr_idx) = target_reader else { return GlobalDefId::NONE };
        self.type_ns_def(self.find_def_in_reader_by_path(tr_idx, td.path))
    }

    /// A type reference that the path lookup resolved to a macro (a derive macro shares the path of its trait): the
    /// same-path definition from the type namespace instead, when the owner has one.
    fn type_ns_def(&self, gid: GlobalDefId) -> GlobalDefId {
        let Some(reader) = self.ws.readers().get(gid.krate.0 as usize).filter(|_| gid.is_some()) else { return gid };
        let defs = reader.definitions();
        let Some(d) = defs.get(gid.index as usize).filter(|d| DefKind::from(d.kind) == DefKind::Macro) else { return gid };
        let path = reader.get_str(d.path_offset());
        let Some((parent, _)) = path.rsplit_once("::") else { return gid };
        reader
            .find_definitions_by_owner(parent)
            .into_iter()
            .find(|cd| {
                !cd.is_reexport()
                    && matches!(DefKind::from(cd.kind), DefKind::Trait | DefKind::Struct | DefKind::Enum | DefKind::Union | DefKind::TyAlias)
                    && reader.get_str(cd.path_offset()) == path
            })
            .map_or(gid, |cd| GlobalDefId::new(gid.krate, super::v5_refs::def_idx_of_ref(defs, cd)))
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
        if let Some(didx) = shard.lookup_reexport_sig(self.ws, raw_path).or_else(|| shard.lookup_reexport_sig(self.ws, &prefixed)) {
            return follow_reexport(didx);
        }
        let rel_cand = raw_path.strip_prefix(cname).and_then(|s| s.strip_prefix("::")).unwrap_or(raw_path);
        if let Some(&didx) = shard.synth_macro_by_name.get(rel_cand) {
            return GlobalDefId::new(CrateId(r_idx as u32), didx);
        }
        if let Some(didx) = shard.lookup_synth_ty_def(self.ws, raw_path).or_else(|| shard.lookup_synth_ty_def(self.ws, &prefixed)) {
            return GlobalDefId::new(CrateId(r_idx as u32), didx);
        }
        GlobalDefId::NONE
    }
}

pub use super::v5_shard::lookup_def_idx_by_exact_path;
