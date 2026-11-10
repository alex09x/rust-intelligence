/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Per-shard pre-indexing (`ShardTables`) and `.ridx` `v5` enum mappers for [`super::v5_db::RidxV5ItemDb`].

use std::collections::{HashMap, HashSet};

use memmap2::Mmap;
use ridx_format::{DefKind, IndexReader, ItemVis, WorkspaceIndex};
use ridx_syntax::name_res::FileId;

use super::meta::{CorpusSourceDb, ReaderMeta};
use super::v5_db::CRATE_ROOT_DEF_IDX;
use super::v5_lazy::PoolStr;
use super::v5_ty_tables::fnv1a_str;

#[derive(Default)]
pub struct ShardTables {
    pub root_mod_def_idx: Option<u32>,
    /// `rustc` `LocalDefIndex` -> `def_idx` (retained in `bench` mode; freed after `build_ty_tables_for` in `LiveDb`).
    pub def_id_to_idx: HashMap<u32, u32>,
    /// Parent `Mod` `def_idx` (or `CRATE_ROOT_DEF_IDX`) -> direct child `def_idx`s.
    pub mod_children: HashMap<u32, Vec<u32>>,
    /// Enum `def_idx` -> `Variant` `def_idx`s.
    pub enum_variants: HashMap<u32, Vec<u32>>,
    /// Trait `def_idx` -> declared `AssocFn` / `AssocConst` / `AssocTy` `def_idx`s.
    pub trait_assocs: HashMap<u32, Vec<u32>>,
    /// Trait-declared assoc `def_idx` -> parent `Trait` `def_idx`.
    pub trait_parent_of_assoc: HashMap<u32, u32>,
    /// Sorted `(def_idx, impl_idx)` pairs for `Impl` `DefEntry` items.
    pub impl_def_to_impl_idx: Box<[(u32, u32)]>,
    /// Sorted `(assoc_didx, impl_idx)` pairs for `Impl`-owned associated items.
    pub assoc_to_impl: Box<[(u32, u32)]>,
    /// `(start_line, start_col)` -> candidate `def_idx`s (populated only in `bench` mode).
    pub line_col_defs: HashMap<(u32, u32), Vec<u32>>,
    /// Shard `ridx_file_id` -> workspace `CorpusSourceDb` `FileId`.
    pub file_map: Vec<FileId>,
    /// `(FileId, start_byte, end_byte)` -> `def_idx` for `def_at_byte_span`.
    pub span_to_def_idx: HashMap<(FileId, u32, u32), u32>,
    /// Sysroot `DefEntry::signature` string-pool ref -> `def_idx` (zero `String` copies).
    pub reexport_sig_to_idx: HashMap<u64, (PoolStr, u32)>,
    /// Synthetic proc-macro export names (`#[proc_macro_derive(X)]`, `#[proc_macro]`) omitted from `DefEntry`.
    pub synth_macros: Vec<String>,
    /// Synthetic proc-macro name -> `PROC_MACRO_SYNTH_BIT | sym_idx`.
    pub synth_macro_by_name: HashMap<String, u32>,
    /// `fnv1a_str(mac_name)` -> `def_index` (`u32::MAX` on hash/name collision) for `macro_rules!`.
    pub macro_rules_by_name: HashMap<u64, u32>,
    /// Unmaterialized `TyDefEntry` string-pool ref -> `TY_DEF_SYNTH_BIT | ty_def_idx` (zero `String` copies).
    pub synth_ty_def_by_path: HashMap<u64, (PoolStr, u32)>,
}

impl ShardTables {
    #[inline]
    pub fn lookup_impl_idx(&self, def_idx: u32) -> Option<u32> {
        self.impl_def_to_impl_idx.binary_search_by_key(&def_idx, |&(k, _)| k).ok().map(|i| self.impl_def_to_impl_idx[i].1)
    }

    #[inline]
    pub fn lookup_assoc_impl(&self, assoc_didx: u32) -> Option<u32> {
        self.assoc_to_impl.binary_search_by_key(&assoc_didx, |&(k, _)| k).ok().map(|i| self.assoc_to_impl[i].1)
    }

    #[inline]
    pub fn lookup_reexport_sig(&self, ws: &WorkspaceIndex, s: &str) -> Option<u32> {
        self.reexport_sig_to_idx.get(&fnv1a_str(s)).filter(|&&(ps, _)| ps.as_str(ws) == s).map(|&(_, idx)| idx)
    }

    #[inline]
    pub fn lookup_synth_ty_def(&self, ws: &WorkspaceIndex, s: &str) -> Option<u32> {
        self.synth_ty_def_by_path.get(&fnv1a_str(s)).filter(|&&(ps, _)| ps.as_str(ws) == s).map(|&(_, idx)| idx)
    }

    #[inline]
    pub fn lookup_macro_rules<'r>(&self, reader: &'r IndexReader<Mmap>, mac_name: &str) -> Option<&'r str> {
        let &didx = self.macro_rules_by_name.get(&fnv1a_str(mac_name))?;
        if didx != u32::MAX {
            let d = reader.definitions().get(didx as usize)?;
            if reader.get_str(d.name_offset()).trim_start_matches("r#") == mac_name {
                return reader.macro_def_for_def(didx).filter(|mv| mv.is_macro_rules()).map(|mv| mv.body);
            }
            return None;
        }
        reader
            .macro_defs_iter()
            .find(|mv| mv.is_macro_rules() && reader.definitions().get(mv.def_index as usize).is_some_and(|d| reader.get_str(d.name_offset()).trim_start_matches("r#") == mac_name))
            .map(|mv| mv.body)
    }
}

#[inline]
fn shard_rel_path<'a>(raw_p: &'a str, meta: &ReaderMeta, cname_prefix: &str, def_id_raw: u32) -> (&'a str, bool) {
    if meta.is_sysroot {
        if raw_p == meta.crate_name || raw_p.is_empty() {
            ("", true)
        } else {
            (raw_p.strip_prefix(cname_prefix).unwrap_or(raw_p), false)
        }
    } else if raw_p.is_empty() || def_id_raw == 0 {
        ("", true)
    } else {
        (raw_p, false)
    }
}

pub fn build_shard_tables(r_idx: usize, reader: &IndexReader<Mmap>, meta: &ReaderMeta, src: &CorpusSourceDb, include_bench_tables: bool) -> ShardTables {
    let defs = reader.definitions();
    let files_count = reader.header().files_count as usize;
    let mut file_map = vec![FileId::NONE; files_count];
    let mut has_mapped_files = false;
    if !meta.is_dependency {
        for (fid, slot) in file_map.iter_mut().enumerate() {
            let fstr = reader.get_file(fid as u32);
            if !fstr.is_empty() {
                if let Some(cfid) = src.lookup_abs_or_rel(meta.source_root.as_deref(), fstr) {
                    *slot = cfid;
                    has_mapped_files = true;
                }
            }
        }
    }

    let mut reexport_def_ids: HashSet<u32> = HashSet::new();
    for d in defs {
        if d.is_reexport() {
            reexport_def_ids.insert(d.def_id());
        }
    }

    let mut root_mod_def_idx = None;
    let mut def_id_to_idx = HashMap::with_capacity(if include_bench_tables { defs.len() / 2 } else { reexport_def_ids.len() });
    let mut mod_path_to_idx: HashMap<&str, u32> = HashMap::new();
    let mut enum_path_to_idx: HashMap<&str, u32> = HashMap::new();
    let mut trait_path_to_idx: HashMap<&str, u32> = HashMap::new();
    let mut line_col_defs: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
    let mut span_to_def_idx: HashMap<(FileId, u32, u32), u32> = HashMap::new();
    let mut reexport_sig_to_idx: HashMap<u64, (PoolStr, u32)> = HashMap::new();

    let cname_prefix = format!("{}::", meta.crate_name);
    for (i, d) in defs.iter().enumerate() {
        let didx = i as u32;
        let dk = DefKind::from(d.kind);
        if d.is_reexport() {
            if meta.is_sysroot {
                let sig = reader.get_str(d.sig_offset());
                if !sig.is_empty() {
                    let ps = PoolStr::from_offset(r_idx, reader, d.sig_offset(), false);
                    reexport_sig_to_idx.entry(fnv1a_str(sig)).or_insert((ps, didx));
                    if let Some(sig_rel) = sig.strip_prefix(cname_prefix.as_str()) {
                        match dk {
                            DefKind::Enum => {
                                enum_path_to_idx.entry(sig_rel).or_insert(didx);
                            }
                            DefKind::Trait => {
                                trait_path_to_idx.entry(sig_rel).or_insert(didx);
                            }
                            _ => {}
                        }
                    }
                }
            }
            continue;
        }
        if !matches!(dk, DefKind::StructCtor | DefKind::VariantCtor) {
            if include_bench_tables || reexport_def_ids.contains(&d.def_id()) {
                def_id_to_idx.entry(d.def_id()).or_insert(didx);
            }
            if include_bench_tables {
                let sp = d.span();
                if sp.start_line() > 0 {
                    line_col_defs.entry((sp.start_line(), sp.start_col())).or_default().push(didx);
                }
            }
        }
        if matches!(dk, DefKind::Mod | DefKind::Enum | DefKind::Trait) {
            let raw_p = reader.get_str(d.path_offset());
            let (rel_p, is_root) = shard_rel_path(raw_p, meta, &cname_prefix, d.def_id());
            match dk {
                DefKind::Mod => {
                    if is_root {
                        if root_mod_def_idx.is_none() {
                            root_mod_def_idx = Some(didx);
                        }
                    } else {
                        mod_path_to_idx.insert(rel_p, didx);
                    }
                }
                DefKind::Enum if !is_root => {
                    enum_path_to_idx.insert(rel_p, didx);
                }
                DefKind::Trait if !is_root => {
                    trait_path_to_idx.insert(rel_p, didx);
                }
                _ => {}
            }
        }

        if has_mapped_files && !d.is_macro_expansion() && !matches!(dk, DefKind::Use | DefKind::Impl) {
            let sp = d.span();
            if let Some(&cfid) = file_map.get(sp.file_id() as usize).filter(|f| f.is_some()) {
                let cf = &src.files[cfid.0 as usize];
                if let (Some(sb), Some(eb)) = (cf.lines.to_byte(&cf.text, sp.start_line(), sp.start_col()), cf.lines.to_byte(&cf.text, sp.end_line(), sp.end_col())) {
                    if sb < eb {
                        span_to_def_idx.entry((cfid, sb, eb)).or_insert(didx);
                    }
                }
            }
        }
    }

    let root_key = root_mod_def_idx.unwrap_or(CRATE_ROOT_DEF_IDX);
    let mut mod_children: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut enum_variants: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut trait_assocs: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut trait_parent_of_assoc: HashMap<u32, u32> = HashMap::new();

    for (i, d) in defs.iter().enumerate() {
        if d.is_reexport() {
            continue;
        }
        let dk = DefKind::from(d.kind);
        if matches!(dk, DefKind::Field | DefKind::Impl | DefKind::Use | DefKind::StructCtor | DefKind::VariantCtor | DefKind::ExternCrate) {
            continue;
        }
        if meta.is_dependency
            && d.visibility != ItemVis::Public as u8
            && !matches!(dk, DefKind::Mod | DefKind::Variant | DefKind::AssocFn | DefKind::AssocConst | DefKind::AssocTy | DefKind::Macro)
        {
            continue;
        }
        if matches!(dk, DefKind::AssocFn | DefKind::AssocConst | DefKind::AssocTy) && trait_path_to_idx.is_empty() {
            continue;
        }
        let didx = i as u32;
        let raw_p = reader.get_str(d.path_offset());
        let (rel_p, is_root) = shard_rel_path(raw_p, meta, &cname_prefix, d.def_id());
        if is_root {
            continue;
        }
        let parent_rel = rel_p.rsplit_once("::").map_or("", |(p, _)| p);
        let pub_or_exported_macro = dk == DefKind::Macro && (d.visibility == ItemVis::Public as u8 || reader.macro_def_for_def(didx).is_some_and(|m| m.is_exported()));
        match dk {
            DefKind::Mod
            | DefKind::Struct
            | DefKind::Enum
            | DefKind::Union
            | DefKind::Trait
            | DefKind::TraitAlias
            | DefKind::Fn
            | DefKind::TyAlias
            | DefKind::ForeignTy
            | DefKind::Const
            | DefKind::Static
            | DefKind::Macro => {
                if parent_rel.is_empty() {
                    mod_children.entry(root_key).or_default().push(didx);
                } else if let Some(&pm_idx) = mod_path_to_idx.get(parent_rel) {
                    mod_children.entry(pm_idx).or_default().push(didx);
                } else if pub_or_exported_macro {
                    mod_children.entry(root_key).or_default().push(didx);
                }
            }
            DefKind::Variant => {
                if let Some(&e_idx) = enum_path_to_idx.get(parent_rel) {
                    enum_variants.entry(e_idx).or_default().push(didx);
                }
            }
            DefKind::AssocFn | DefKind::AssocConst | DefKind::AssocTy => {
                if let Some(&tr_idx) = trait_path_to_idx.get(parent_rel) {
                    trait_assocs.entry(tr_idx).or_default().push(didx);
                    trait_parent_of_assoc.insert(didx, tr_idx);
                }
            }
            _ => {}
        }
        if pub_or_exported_macro && !parent_rel.is_empty() {
            let root_vec = mod_children.entry(root_key).or_default();
            if !root_vec.contains(&didx) {
                root_vec.push(didx);
            }
        }
    }

    let mut macro_rules_by_name: HashMap<u64, u32> = HashMap::new();
    for mv in reader.macro_defs_iter() {
        if mv.is_macro_rules() {
            if let Some(d) = defs.get(mv.def_index as usize) {
                let nm = reader.get_str(d.name_offset()).trim_start_matches("r#");
                if !nm.is_empty() {
                    macro_rules_by_name.entry(fnv1a_str(nm)).and_modify(|e| *e = u32::MAX).or_insert(mv.def_index);
                }
            }
        }
    }

    let mut impl_pairs: Vec<(u32, u32)> = Vec::new();
    let mut assoc_pairs: Vec<(u32, u32)> = Vec::new();
    for iv in reader.impls_iter() {
        let impl_idx = iv.index();
        if let Some(didx) = iv.def_index() {
            impl_pairs.push((didx, impl_idx));
        }
        for adidx in iv.assoc_def_indices() {
            assoc_pairs.push((adidx, impl_idx));
        }
    }
    impl_pairs.sort_unstable_by_key(|&(k, _)| k);
    impl_pairs.dedup_by_key(|p| p.0);
    assoc_pairs.sort_unstable_by_key(|&(k, _)| k);
    assoc_pairs.dedup_by_key(|p| p.0);

    let mut synth_ty_def_by_path = HashMap::new();
    let ty_arena = reader.ty_arena();
    for i in 0..reader.ty_defs().len() {
        if let Some(td) = ty_arena.def(ridx_format::TyDefId(i as u32)) {
            let is_local_synth = td.local_def_idx.map_or(td.extern_unit.is_none(), |idx| idx as usize >= defs.len());
            if is_local_synth && !td.path.is_empty() {
                if let Some(ps) = PoolStr::from_pool_slice(r_idx, reader, td.path) {
                    synth_ty_def_by_path.entry(fnv1a_str(td.path)).or_insert((ps, super::v5_db::TY_DEF_SYNTH_BIT | (i as u32)));
                }
            }
        }
    }

    ShardTables {
        root_mod_def_idx,
        def_id_to_idx,
        mod_children,
        enum_variants,
        trait_assocs,
        trait_parent_of_assoc,
        impl_def_to_impl_idx: impl_pairs.into_boxed_slice(),
        assoc_to_impl: assoc_pairs.into_boxed_slice(),
        line_col_defs,
        file_map,
        span_to_def_idx,
        reexport_sig_to_idx,
        synth_macros: Vec::new(),
        synth_macro_by_name: HashMap::new(),
        macro_rules_by_name,
        synth_ty_def_by_path,
    }
}

pub use super::v5_refs::{collect_shard_alias_pairs, def_has_value_ctor, lookup_def_idx_by_exact_path, map_ridx_kind, map_use_prim, map_vis};
