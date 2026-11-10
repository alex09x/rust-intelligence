/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Pre-computed `.ridx` `v5` method, trait, `impl`, associated-type, and `#[lang]` lookup tables
//! (`V5TyTables`) keyed by zero-alloc 64-bit FNV-1a hashes + string-pool [`PoolStr`] verification.

use std::collections::hash_map::Entry;
use std::collections::HashMap;

use ridx_format::{DefKind, TyDefId};
use ridx_syntax::name_res::{CrateId, GlobalDefId, ImplTargetKey};
use ridx_syntax::ty_infer::{TraitAssocFnMeta, TyLangItem};

use super::v5_db::{RidxV5ItemDb, CRATE_ROOT_DEF_IDX, IMPL_SYNTH_BIT, PROC_MACRO_SYNTH_BIT};
use super::v5_lazy::PoolStr;
pub use super::v5_refs::fnv1a_str;
use super::v5_ty_inst::{extend_flat_groups, slice_flat_group};

#[derive(Clone, Copy)]
pub struct TraitMethodInfo {
    pub name: PoolStr,
    pub method_didx: u32,
    pub trait_gc: u16,
    pub method_gc: u16,
    pub has_self: bool,
}

#[derive(Clone, Copy)]
pub struct InherentMethodInfo {
    pub name: PoolStr,
    pub r_idx: u32,
    pub method_didx: u32,
    pub impl_gc: u16,
    pub method_gc: u16,
}

#[derive(Clone, Copy)]
pub struct TraitImplInfo {
    pub r_idx: u32,
    pub impl_idx: u32,
    pub impl_gc: u16,
    pub is_blanket: bool,
    pub fn_bound_arity: u8,
}

#[derive(Default)]
pub struct V5TyTables {
    pub reexport_canon: HashMap<GlobalDefId, GlobalDefId>,
    pub ty_def_gids: Vec<Vec<GlobalDefId>>,
    pub lang_to_gid: HashMap<u16, GlobalDefId>,
    pub gid_to_lang: HashMap<GlobalDefId, TyLangItem>,
    pub cstr_gid: Option<GlobalDefId>,
    pub vec_gid: Option<GlobalDefId>,
    pub global_gid: Option<GlobalDefId>,
    pub fmt_args_gid: Option<GlobalDefId>,
    pub fmt_err_gid: Option<GlobalDefId>,
    pub thread_local_key_gid: Option<GlobalDefId>,
    pub assoc_to_impl: HashMap<GlobalDefId, u32>,
    pub trait_non_lt_gc: HashMap<GlobalDefId, u16>,
    pub trait_assoc_fns: HashMap<(GlobalDefId, u64), TraitMethodInfo>,
    pub trait_fn_meta: HashMap<GlobalDefId, TraitAssocFnMeta>,
    pub trait_assoc_tys: HashMap<(GlobalDefId, u64), (GlobalDefId, PoolStr, u16)>,
    pub assoc_ty_names: HashMap<GlobalDefId, PoolStr>,
    pub inherent_arena: Vec<InherentMethodInfo>,
    pub inherent_methods: HashMap<(ImplTargetKey, u64), (u32, u32)>,
    pub trait_impl_arena: Vec<TraitImplInfo>,
    pub concrete_trait_impls: HashMap<(GlobalDefId, ImplTargetKey), (u32, u32)>,
    pub blanket_trait_impls: HashMap<GlobalDefId, (u32, u32)>,
    pub synth_def_locs: std::sync::Mutex<HashMap<GlobalDefId, Option<super::query::LiveLocation>>>,
}

impl V5TyTables {
    #[inline]
    pub fn inherent_slice(&self, key: &(ImplTargetKey, u64)) -> &[InherentMethodInfo] {
        slice_flat_group(&self.inherent_arena, &self.inherent_methods, key)
    }
    #[inline]
    pub fn concrete_impls(&self, key: &(GlobalDefId, ImplTargetKey)) -> &[TraitImplInfo] {
        slice_flat_group(&self.trait_impl_arena, &self.concrete_trait_impls, key)
    }
    #[inline]
    pub fn blanket_impls(&self, trait_def: &GlobalDefId) -> &[TraitImplInfo] {
        slice_flat_group(&self.trait_impl_arena, &self.blanket_trait_impls, trait_def)
    }
}

impl RidxV5ItemDb<'_> {
    #[inline]
    pub fn canonical_def(&self, mut def: GlobalDefId) -> GlobalDefId {
        for _ in 0..4 {
            let Some(&next) = self.ty_tables.reexport_canon.get(&def) else { break };
            if next == def || !next.is_some() {
                break;
            }
            def = next;
        }
        def
    }

    #[inline]
    pub fn ty_def_gid(&self, r_idx: usize, td: TyDefId) -> GlobalDefId {
        self.ty_tables.ty_def_gids.get(r_idx).and_then(|v| v.get(td.0 as usize)).copied().unwrap_or(GlobalDefId::NONE)
    }

    fn resolve_reexport_canon(&self, def: GlobalDefId) -> GlobalDefId {
        if !def.is_some() || def.index == CRATE_ROOT_DEF_IDX || (def.index & (IMPL_SYNTH_BIT | PROC_MACRO_SYNTH_BIT)) != 0 {
            return def;
        }
        let r_idx = def.krate.0 as usize;
        let Some(reader) = self.ws.readers().get(r_idx) else {
            return def;
        };
        let Some(d) = reader.definitions().get(def.index as usize) else {
            return def;
        };
        if d.is_reexport() {
            let raw_sig = reader.get_str(d.sig_offset());
            let sig = raw_sig.strip_prefix("crate::").or_else(|| raw_sig.strip_prefix("self::")).unwrap_or(raw_sig);
            let first = sig.split("::").next().unwrap_or("");
            if !first.is_empty() {
                let ext_r = if first == self.metas[r_idx].crate_name {
                    Some(r_idx)
                } else {
                    self.resolve_extern_name_to_reader(r_idx, first).or_else(|| self.sysroot_by_name.get(first).copied())
                };
                if let Some(tr_idx) = ext_r {
                    let cg = self.find_def_in_reader_by_path(tr_idx, sig);
                    if cg.is_some() && cg != def {
                        return cg;
                    }
                }
                let local_cg = self.find_def_in_reader_by_path(r_idx, sig);
                if local_cg.is_some() && local_cg != def {
                    return local_cg;
                }
            }
            if let Some(&tdidx) = self.shards[r_idx].def_id_to_idx.get(&d.def_id()) {
                if tdidx != def.index {
                    return GlobalDefId::new(def.krate, tdidx);
                }
            }
        }
        def
    }

    #[inline]
    pub fn lookup_trait_assoc_ty_def(&self, trait_gid: GlobalDefId, name: &str) -> GlobalDefId {
        let h = fnv1a_str(name);
        if let Some(&(agid, _, _)) = self.ty_tables.trait_assoc_tys.get(&(trait_gid, h)).filter(|&&(_, s, _)| s.as_str(self.ws) == name) {
            return agid;
        }
        if let Some(supers) = self.trait_supertraits.get(&trait_gid) {
            for &sup in supers {
                if let Some(&(agid, _, _)) = self.ty_tables.trait_assoc_tys.get(&(sup, h)).filter(|&&(_, s, _)| s.as_str(self.ws) == name) {
                    return agid;
                }
            }
        }
        GlobalDefId::NONE
    }

    pub fn build_ty_tables_for(&mut self, active: &[usize]) {
        let ws = self.ws;
        for &r_idx in active {
            let reader = &ws.readers()[r_idx];
            let krate = CrateId(r_idx as u32);
            let mut uniq_by_kind_name: HashMap<(u8, &str), u32> = HashMap::new();
            for (didx, d) in reader.definitions().iter().enumerate() {
                if !d.is_reexport() {
                    let nm = reader.get_str(d.name_offset()).trim_start_matches("r#");
                    uniq_by_kind_name.entry((d.kind, nm)).and_modify(|e| *e = u32::MAX).or_insert(didx as u32);
                }
            }
            for (didx, d) in reader.definitions().iter().enumerate() {
                if d.is_reexport() {
                    let g = GlobalDefId::new(krate, didx as u32);
                    let mut cg = self.resolve_reexport_canon(g);
                    if (cg == g || !cg.is_some()) && reader.get_str(d.sig_offset()).is_empty() {
                        let nm = reader.get_str(d.name_offset()).trim_start_matches("r#");
                        if let Some(&u_idx) = uniq_by_kind_name.get(&(d.kind, nm)).filter(|&&u| u != u32::MAX && u != didx as u32) {
                            cg = GlobalDefId::new(krate, u_idx);
                        }
                    }
                    if cg != g && cg.is_some() {
                        self.state.as_mut().ty_tables.reexport_canon.insert(g, cg);
                    }
                }
            }
            if !self.bench_mode {
                self.state.as_mut().shards[r_idx].def_id_to_idx = HashMap::new();
            }
        }
        for &r_idx in active {
            let reader = &ws.readers()[r_idx];
            let arena = reader.ty_arena();
            let td_len = reader.ty_defs().len();
            let mut vec = Vec::with_capacity(td_len);
            for i in 0..td_len {
                vec.push(self.canonical_def(self.resolve_ty_def(r_idx, arena, TyDefId(i as u32))));
            }
            self.state.as_mut().ty_tables.ty_def_gids[r_idx] = vec;
        }

        let mut order: Vec<usize> = active.to_vec();
        order.sort_by_key(|&idx| (!self.metas[idx].is_sysroot, self.metas[idx].is_test_harness, idx));

        for &r_idx in &order {
            let reader = &ws.readers()[r_idx];
            let krate = CrateId(r_idx as u32);
            for lv in reader.lang_items_iter() {
                let raw_k = lv.kind as u16;
                let gid = match lv.def_index {
                    Some(didx) => self.canonical_def(GlobalDefId::new(krate, didx)),
                    None => self.ty_def_gid(r_idx, lv.ty_def),
                };
                if gid.is_some() {
                    let st = self.state.as_mut();
                    st.ty_tables.lang_to_gid.entry(raw_k).or_insert(gid);
                    if let Some(tl) = u16_to_ty_lang(raw_k) {
                        st.ty_tables.gid_to_lang.entry(gid).or_insert(tl);
                    }
                }
            }
        }

        if let Some(&core_r) = self.sysroot_by_name.get("core").filter(|&&r| self.shard_loaded[r]) {
            let cg = self.find_def_in_reader_by_path(core_r, "core::ffi::c_str::CStr");
            let cg2 = if cg.is_some() { cg } else { self.find_def_in_reader_by_path(core_r, "core::ffi::CStr") };
            if cg2.is_some() {
                let cgid = self.canonical_def(cg2);
                self.state.as_mut().ty_tables.cstr_gid = Some(cgid);
            }
            let send_g = self.find_def_in_reader_by_path(core_r, "core::marker::Send");
            if send_g.is_some() {
                let sgid = self.canonical_def(send_g);
                self.state.as_mut().ty_tables.gid_to_lang.entry(sgid).or_insert(ridx_syntax::ty_infer::TyLangItem::Sync);
            }
        }

        for &r_idx in active {
            let reader = &ws.readers()[r_idx];
            let krate = CrateId(r_idx as u32);
            let defs = reader.definitions();
            let st = self.state.as_mut();
            for (&trait_didx, assocs) in &st.shards[r_idx].trait_assocs {
                let mut trait_gid = GlobalDefId::new(krate, trait_didx);
                for _ in 0..4 {
                    let Some(&nx) = st.ty_tables.reexport_canon.get(&trait_gid) else { break };
                    if nx == trait_gid || !nx.is_some() {
                        break;
                    }
                    trait_gid = nx;
                }
                let trait_gc = count_def_non_lt_non_self(reader, trait_didx);
                st.ty_tables.trait_non_lt_gc.insert(trait_gid, trait_gc);
                let mut assoc_ord = 0u16;
                for &adidx in assocs {
                    let Some(ad) = defs.get(adidx as usize) else { continue };
                    let aname = reader.get_str(ad.name_offset()).trim_start_matches("r#");
                    if aname.is_empty() {
                        continue;
                    }
                    let aname_ps = PoolStr::from_offset(r_idx, reader, ad.name_offset(), true);
                    let nh = fnv1a_str(aname);
                    match DefKind::from(ad.kind) {
                        DefKind::AssocFn => {
                            let method_gc = count_def_non_lt_non_self(reader, adidx);
                            let has_self = reader.fn_sig_for_def(adidx).is_some_and(|s| s.has_self());
                            let info = TraitMethodInfo { name: aname_ps, method_didx: adidx, trait_gc, method_gc, has_self };
                            st.ty_tables.trait_assoc_fns.insert((trait_gid, nh), info);
                            let fn_gid = GlobalDefId::new(krate, adidx);
                            st.ty_tables
                                .trait_fn_meta
                                .insert(fn_gid, TraitAssocFnMeta { trait_def: trait_gid, trait_fn_def: fn_gid, trait_generics: trait_gc, method_generics: method_gc });
                        }
                        DefKind::AssocTy => {
                            let agid = GlobalDefId::new(krate, adidx);
                            st.ty_tables.trait_assoc_tys.insert((trait_gid, nh), (agid, aname_ps, assoc_ord));
                            st.ty_tables.assoc_ty_names.insert(agid, aname_ps);
                            assoc_ord = assoc_ord.saturating_add(1);
                        }
                        _ => {}
                    }
                }
            }
        }

        let mut inh_groups: HashMap<(ImplTargetKey, u64), Vec<InherentMethodInfo>> = HashMap::new();
        let mut conc_groups: HashMap<(GlobalDefId, ImplTargetKey), Vec<TraitImplInfo>> = HashMap::new();
        let mut blank_groups: HashMap<GlobalDefId, Vec<TraitImplInfo>> = HashMap::new();

        for &r_idx in active {
            let reader = &ws.readers()[r_idx];
            let arena = reader.ty_arena();
            let krate = CrateId(r_idx as u32);
            let defs = reader.definitions();

            for iv in reader.impls_iter() {
                let impl_idx = iv.index();
                {
                    let st = self.state.as_mut();
                    for assoc_didx in iv.assoc_def_indices() {
                        st.ty_tables.assoc_to_impl.insert(GlobalDefId::new(krate, assoc_didx), impl_idx);
                    }
                }
                if iv.is_negative() || iv.is_reservation() {
                    continue;
                }
                let raw_target = self.lower_ty_to_target_key(r_idx, arena, iv.self_ty(), 0);
                let target = match raw_target {
                    ImplTargetKey::Adt(d) => ImplTargetKey::Adt(self.canonical_def(d)),
                    ImplTargetKey::Dyn(d) => ImplTargetKey::Dyn(self.canonical_def(d)),
                    other => other,
                };
                if target == ImplTargetKey::Unknown {
                    continue;
                }
                let impl_gc = count_impl_non_lt(reader, impl_idx);
                if iv.is_inherent() {
                    for assoc_didx in iv.assoc_def_indices() {
                        let Some(ad) = defs.get(assoc_didx as usize) else { continue };
                        if DefKind::from(ad.kind) != DefKind::AssocFn || !reader.fn_sig_for_def(assoc_didx).is_some_and(|s| s.has_self()) {
                            continue;
                        }
                        let mname = reader.get_str(ad.name_offset()).trim_start_matches("r#");
                        let mname_ps = PoolStr::from_offset(r_idx, reader, ad.name_offset(), true);
                        let method_gc = count_def_non_lt_non_self(reader, assoc_didx);
                        inh_groups.entry((target, fnv1a_str(mname))).or_default().push(InherentMethodInfo {
                            name: mname_ps,
                            r_idx: r_idx as u32,
                            method_didx: assoc_didx,
                            impl_gc,
                            method_gc,
                        });
                    }
                } else if let Some(td) = iv.trait_def() {
                    let tgid = self.ty_def_gid(r_idx, td);
                    if !tgid.is_some() {
                        continue;
                    }
                    let fn_bound_arity = self.detect_impl_self_fn_bound_arity(r_idx, reader, arena, impl_idx, iv.self_ty());
                    let st = self.state.as_mut();
                    for ab in iv.assoc_tys() {
                        let aname = ab.name.trim_start_matches("r#");
                        if !aname.is_empty() {
                            if let Some(aname_ps) = PoolStr::from_pool_slice(r_idx, reader, aname) {
                                let nh = fnv1a_str(aname);
                                if let Entry::Vacant(vac) = st.ty_tables.trait_assoc_tys.entry((tgid, nh)) {
                                    let agid = GlobalDefId::new(tgid.krate, super::v5_db::TY_DEF_SYNTH_BIT | 0x2000_0000 | ((nh as u32) & 0x1fff_ffff));
                                    vac.insert((agid, aname_ps, 0));
                                    st.ty_tables.assoc_ty_names.insert(agid, aname_ps);
                                }
                            }
                        }
                    }
                    for assoc_didx in iv.assoc_def_indices() {
                        if let Some(ad) = defs.get(assoc_didx as usize).filter(|a| DefKind::from(a.kind) == DefKind::AssocFn) {
                            let mname = reader.get_str(ad.name_offset()).trim_start_matches("r#");
                            if let Some(&tm) = st.ty_tables.trait_assoc_fns.get(&(tgid, fnv1a_str(mname))).filter(|t| t.name.as_str(ws) == mname) {
                                let tfn = GlobalDefId::new(tgid.krate, tm.method_didx);
                                let meta = TraitAssocFnMeta { trait_def: tgid, trait_fn_def: tfn, trait_generics: tm.trait_gc, method_generics: tm.method_gc };
                                st.ty_tables.trait_fn_meta.insert(GlobalDefId::new(krate, assoc_didx), meta);
                            }
                        }
                    }
                    let is_ref_to_param =
                        matches!(arena.kind(iv.self_ty()), ridx_format::TyKindView::Ref { inner, .. } if matches!(arena.kind(inner), ridx_format::TyKindView::Param { .. }));
                    let info = TraitImplInfo { r_idx: r_idx as u32, impl_idx, impl_gc, is_blanket: target == ImplTargetKey::Param || is_ref_to_param, fn_bound_arity };
                    if target == ImplTargetKey::Param {
                        blank_groups.entry(tgid).or_default().push(info);
                    } else {
                        conc_groups.entry((tgid, target)).or_default().push(info);
                    }
                }
            }
        }
        {
            let st = self.state.as_mut();
            extend_flat_groups(&mut st.ty_tables.inherent_arena, &mut st.ty_tables.inherent_methods, inh_groups);
            extend_flat_groups(&mut st.ty_tables.trait_impl_arena, &mut st.ty_tables.concrete_trait_impls, conc_groups);
            extend_flat_groups(&mut st.ty_tables.trait_impl_arena, &mut st.ty_tables.blanket_trait_impls, blank_groups);
        }
        self.populate_v5_std_macro_gids();
    }
}

pub use super::v5_ty_defaults::u16_to_ty_lang;
pub use super::v5_ty_inst::{count_def_non_lt_non_self, count_impl_non_lt};
