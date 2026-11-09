/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Per-shard pre-indexing (`ShardTables`) and `.ridx` `v5` enum mappers for [`crate::v5_db::RidxV5ItemDb`].

use std::collections::{HashMap, HashSet};

use memmap2::Mmap;
use ridx_format::{CtorShape, DefKind, IndexReader, ItemVis, TyKindView, UseOwner, UsePrimTy, USE_FLAG_GLOB, USE_FLAG_UNDERSCORE};
use ridx_syntax::name_res::{FileId, ItemKind, PrimTy, VisKind};

use crate::meta::{CorpusSourceDb, ReaderMeta};
use crate::v5_db::CRATE_ROOT_DEF_IDX;

pub struct ShardTables {
    pub root_mod_def_idx: Option<u32>,
    /// `rustc` `LocalDefIndex` (`DefEntry::def_id()`) -> `def_idx` (`0..definitions().len()`).
    pub def_id_to_idx: HashMap<u32, u32>,
    /// Parent `Mod` `def_idx` (or `CRATE_ROOT_DEF_IDX`) -> direct child `def_idx`s.
    pub mod_children: HashMap<u32, Vec<u32>>,
    /// Enum `def_idx` -> `Variant` `def_idx`s.
    pub enum_variants: HashMap<u32, Vec<u32>>,
    /// Trait `def_idx` -> declared `AssocFn` / `AssocConst` / `AssocTy` `def_idx`s.
    pub trait_assocs: HashMap<u32, Vec<u32>>,
    /// Trait-declared assoc `def_idx` -> parent `Trait` `def_idx`.
    pub trait_parent_of_assoc: HashMap<u32, u32>,
    /// `Impl` `DefEntry` `def_idx` -> `impl_idx` (`0..impls().len()`).
    pub impl_def_to_impl_idx: HashMap<u32, u32>,
    /// `(start_line, start_col)` -> candidate `def_idx`s across files in this shard.
    pub line_col_defs: HashMap<(u32, u32), Vec<u32>>,
    /// Shard `ridx_file_id` -> workspace `CorpusSourceDb` `FileId`.
    pub file_map: Vec<FileId>,
    /// `(FileId, start_byte, end_byte)` -> `def_idx` for `def_at_byte_span`.
    pub span_to_def_idx: HashMap<(FileId, u32, u32), u32>,
    /// Sysroot `DefEntry::signature` (canonical path of private-module re-export) -> `def_idx`.
    pub reexport_sig_to_idx: HashMap<String, u32>,
    /// Synthetic proc-macro export names (`#[proc_macro_derive(X)]`, `#[proc_macro]`) omitted from `DefEntry`.
    pub synth_macros: Vec<String>,
    /// Synthetic proc-macro name -> `PROC_MACRO_SYNTH_BIT | sym_idx`.
    pub synth_macro_by_name: HashMap<String, u32>,
    /// Unmaterialized `TyDefEntry` path (`local_def_idx == 0x7FFF_FFFF`, e.g. `core::sync::atomic::Atomic`) -> `TY_DEF_SYNTH_BIT | ty_def_idx`.
    pub synth_ty_def_by_path: HashMap<String, u32>,
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

pub fn build_shard_tables(reader: &IndexReader<Mmap>, meta: &ReaderMeta, src: &CorpusSourceDb) -> ShardTables {
    let defs = reader.definitions();
    let files_count = reader.header().files_count as usize;
    let mut file_map = vec![FileId::NONE; files_count];
    if !meta.is_dependency {
        for (fid, slot) in file_map.iter_mut().enumerate() {
            let fstr = reader.get_file(fid as u32);
            if !fstr.is_empty() {
                if let Some(cfid) = src.lookup_abs_or_rel(meta.source_root.as_deref(), fstr) {
                    *slot = cfid;
                }
            }
        }
    }

    let mut root_mod_def_idx = None;
    let mut def_id_to_idx = HashMap::with_capacity(defs.len());
    let mut mod_path_to_idx: HashMap<&str, u32> = HashMap::new();
    let mut enum_path_to_idx: HashMap<&str, u32> = HashMap::new();
    let mut trait_path_to_idx: HashMap<&str, u32> = HashMap::new();
    let mut line_col_defs: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
    let mut span_to_def_idx: HashMap<(FileId, u32, u32), u32> = HashMap::new();
    let mut reexport_sig_to_idx: HashMap<String, u32> = HashMap::new();

    let cname_prefix = format!("{}::", meta.crate_name);
    for (i, d) in defs.iter().enumerate() {
        let didx = i as u32;
        let dk = DefKind::from(d.kind);
        if d.is_reexport() {
            if meta.is_sysroot {
                let sig = reader.get_str(d.sig_offset());
                if !sig.is_empty() {
                    reexport_sig_to_idx.entry(sig.to_string()).or_insert(didx);
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
            def_id_to_idx.entry(d.def_id()).or_insert(didx);
            let sp = d.span();
            if sp.start_line() > 0 {
                line_col_defs.entry((sp.start_line(), sp.start_col())).or_default().push(didx);
            }
        }
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

        if !meta.is_dependency && !d.is_macro_expansion() && !matches!(dk, DefKind::Use | DefKind::Impl) {
            let sp = d.span();
            if let Some(&cfid) = file_map.get(sp.file_id() as usize) {
                if cfid.is_some() {
                    let cf = &src.files[cfid.0 as usize];
                    if let (Some(sb), Some(eb)) = (cf.lines.to_byte(&cf.text, sp.start_line(), sp.start_col()), cf.lines.to_byte(&cf.text, sp.end_line(), sp.end_col())) {
                        if sb < eb {
                            span_to_def_idx.entry((cfid, sb, eb)).or_insert(didx);
                        }
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
        let didx = i as u32;
        let dk = DefKind::from(d.kind);
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

    let mut impl_def_to_impl_idx = HashMap::new();
    for iv in reader.impls_iter() {
        if let Some(didx) = iv.def_index() {
            impl_def_to_impl_idx.insert(didx, iv.index());
        }
    }
    let mut synth_ty_def_by_path = HashMap::new();
    let ty_arena = reader.ty_arena();
    for i in 0..reader.ty_defs().len() {
        if let Some(td) = ty_arena.def(ridx_format::TyDefId(i as u32)) {
            let is_local_synth = td.local_def_idx.map_or(td.extern_unit.is_none(), |idx| idx as usize >= defs.len());
            if is_local_synth && !td.path.is_empty() {
                synth_ty_def_by_path.entry(td.path.to_string()).or_insert(crate::v5_db::TY_DEF_SYNTH_BIT | (i as u32));
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
        impl_def_to_impl_idx,
        line_col_defs,
        file_map,
        span_to_def_idx,
        reexport_sig_to_idx,
        synth_macros: Vec::new(),
        synth_macro_by_name: HashMap::new(),
        synth_ty_def_by_path,
    }
}

#[inline]
pub fn map_vis(v: u8) -> VisKind {
    match v {
        x if x == ItemVis::Public as u8 => VisKind::Public,
        x if x == ItemVis::Crate as u8 => VisKind::Crate,
        x if x == ItemVis::Restricted as u8 => VisKind::Restricted,
        _ => VisKind::Private,
    }
}

#[inline]
pub fn map_use_prim(p: UsePrimTy) -> u8 {
    match p {
        UsePrimTy::None => 0,
        UsePrimTy::Bool => PrimTy::Bool as u8,
        UsePrimTy::Char => PrimTy::Char as u8,
        UsePrimTy::Str => PrimTy::Str as u8,
        UsePrimTy::I8 => PrimTy::I8 as u8,
        UsePrimTy::I16 => PrimTy::I16 as u8,
        UsePrimTy::I32 => PrimTy::I32 as u8,
        UsePrimTy::I64 => PrimTy::I64 as u8,
        UsePrimTy::I128 => PrimTy::I128 as u8,
        UsePrimTy::Isize => PrimTy::Isize as u8,
        UsePrimTy::U8 => PrimTy::U8 as u8,
        UsePrimTy::U16 => PrimTy::U16 as u8,
        UsePrimTy::U32 => PrimTy::U32 as u8,
        UsePrimTy::U64 => PrimTy::U64 as u8,
        UsePrimTy::U128 => PrimTy::U128 as u8,
        UsePrimTy::Usize => PrimTy::Usize as u8,
        UsePrimTy::F32 => PrimTy::F32 as u8,
        UsePrimTy::F64 => PrimTy::F64 as u8,
        UsePrimTy::F16 | UsePrimTy::F128 => 0,
    }
}

#[inline]
pub fn map_ridx_kind(dk: DefKind) -> Option<ItemKind> {
    match dk {
        DefKind::Mod => Some(ItemKind::Mod),
        DefKind::Struct | DefKind::StructCtor => Some(ItemKind::Struct),
        DefKind::Enum => Some(ItemKind::Enum),
        DefKind::Union => Some(ItemKind::Union),
        DefKind::Trait | DefKind::TraitAlias => Some(ItemKind::Trait),
        DefKind::Fn => Some(ItemKind::Fn),
        DefKind::TyAlias | DefKind::ForeignTy => Some(ItemKind::TypeAlias),
        DefKind::Const => Some(ItemKind::Const),
        DefKind::Static => Some(ItemKind::Static),
        DefKind::Variant | DefKind::VariantCtor => Some(ItemKind::Variant),
        DefKind::Field => Some(ItemKind::Field),
        DefKind::AssocFn => Some(ItemKind::AssocFn),
        DefKind::AssocTy => Some(ItemKind::AssocTy),
        DefKind::AssocConst => Some(ItemKind::AssocConst),
        DefKind::Macro => Some(ItemKind::Macro),
        DefKind::Impl => Some(ItemKind::Impl),
        DefKind::Use => Some(ItemKind::Use),
        DefKind::ExternCrate => Some(ItemKind::ExternCrate),
        _ => None,
    }
}

#[inline]
pub fn def_has_value_ctor(reader: &IndexReader<Mmap>, def_idx: u32, kind: ItemKind) -> bool {
    if !matches!(kind, ItemKind::Struct | ItemKind::Variant) {
        return false;
    }
    if let Some(dt) = reader.def_ty_for_def(def_idx) {
        return matches!(dt.ctor_shape(), CtorShape::Tuple | CtorShape::Unit);
    }
    reader.fn_sig_for_def(def_idx).is_some() || reader.fields_for_owner(def_idx).next().is_none()
}

pub fn collect_shard_alias_pairs(reader: &IndexReader<Mmap>, out: &mut HashSet<(String, String)>) {
    let arena = reader.ty_arena();
    let mut owners = vec![UseOwner::CrateRoot];
    for (didx, d) in reader.definitions().iter().enumerate() {
        let dk = DefKind::from(d.kind);
        if !d.is_reexport() && dk == DefKind::Mod {
            owners.push(UseOwner::Def(didx as u32));
        }
        if d.is_reexport() {
            let alias = reader.get_str(d.name_offset()).trim_start_matches("r#");
            let sig = reader.get_str(d.sig_offset());
            let orig_leaf = sig.rsplit("::").next().unwrap_or(sig).trim_start_matches("r#");
            if !alias.is_empty() && !orig_leaf.is_empty() && alias != orig_leaf {
                out.insert((alias.to_string(), orig_leaf.to_string()));
            }
        } else if dk == DefKind::TyAlias {
            let alias = reader.get_str(d.name_offset()).trim_start_matches("r#");
            if let Some(dt) = reader.def_ty_for_def(didx as u32) {
                if let TyKindView::Adt { def, .. } = arena.kind(dt.ty()) {
                    if let Some(td) = arena.def(def) {
                        let orig_leaf = td.path.rsplit("::").next().unwrap_or(td.path).trim_start_matches("r#");
                        if !alias.is_empty() && !orig_leaf.is_empty() && alias != orig_leaf {
                            out.insert((alias.to_string(), orig_leaf.to_string()));
                        }
                    }
                }
            }
        }
    }
    for owner in owners {
        for u in reader.uses_for_owner(owner) {
            if (u.flags & (USE_FLAG_GLOB | USE_FLAG_UNDERSCORE)) != 0 || u.name.is_empty() {
                continue;
            }
            let alias = u.name.trim_start_matches("r#");
            for td_id in [u.type_def, u.value_def, u.macro_def].into_iter().flatten() {
                if let Some(td) = arena.def(td_id) {
                    let orig_leaf = td.path.rsplit("::").next().unwrap_or(td.path).trim_start_matches("r#");
                    if !orig_leaf.is_empty() && alias != orig_leaf {
                        out.insert((alias.to_string(), orig_leaf.to_string()));
                    }
                }
            }
        }
    }
}
