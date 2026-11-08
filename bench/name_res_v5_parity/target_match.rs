/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Target-equivalence check (`Strict` vs `LenientAssocTrait`) and diagnostic formatting between
//! `ridx-driver` `RefEntry` and `ridx_syntax::name_res::ResTarget`.

use memmap2::Mmap;
use ridx_format::{DefKind, IndexReader, RefEntry};
use ridx_syntax::name_res::{CrateId, FileNameRes, GlobalDefId, ImplTargetKey, ItemDb, ItemKind, ResTarget};
use ridx_syntax::{lex, TokenKind};

use crate::meta::CorpusFile;
use crate::v5_db::{RidxV5ItemDb, CRATE_ROOT_DEF_IDX, IMPL_SYNTH_BIT, PROC_MACRO_SYNTH_BIT};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchKind {
    NoMatch = 0,
    LenientAssocTrait = 1,
    StrictLocalRefSelf = 2,
    StrictLocalRefMut = 3,
    Strict = 4,
}

impl MatchKind {
    #[inline]
    pub fn is_matched(self) -> bool {
        self != Self::NoMatch
    }
}

#[inline]
fn norm_ident(s: &str) -> &str {
    s.strip_prefix("r#").unwrap_or(s)
}

/// Exact token-level match between `rustc` HIR `PatKind::Binding` start (`drv_tb`) and CST
/// `LocalBindData::name_start` without any byte-distance window:
/// - `Direct`: `drv_tb == name_start`,
/// - `StrictLocalRefMut`: `rustc` span starts on `ref` / `mut` (`ref x`, `mut x`, `ref mut x`, `mut self`),
/// - `StrictLocalRefSelf`: `rustc` `SelfKind::Region` lowers `Param.pat.span` starting at `&`
///   (`&self`, `&mut self`, `&'lt self`, `&'lt mut self`).
fn match_local_name_start(src: &str, drv_tb: u32, name_start: u32, is_self: bool) -> MatchKind {
    if name_start == drv_tb {
        return MatchKind::Strict;
    }
    if name_start < drv_tb || (drv_tb as usize) >= src.len() {
        return MatchKind::NoMatch;
    }
    let s = drv_tb as usize;
    let end_bound = src.floor_char_boundary((name_start as usize).saturating_add(16).min(src.len()));
    let toks = lex(&src[s..end_bound]);
    let mut idx = 0usize;
    while idx < toks.len() && toks[idx].is_trivia() {
        idx += 1;
    }
    if idx >= toks.len() {
        return MatchKind::NoMatch;
    }
    if is_self && toks[idx].kind == TokenKind::Amp {
        idx += 1;
        while idx < toks.len() && toks[idx].is_trivia() {
            idx += 1;
        }
        if idx < toks.len() && toks[idx].kind == TokenKind::Lifetime {
            idx += 1;
            while idx < toks.len() && toks[idx].is_trivia() {
                idx += 1;
            }
        }
        if idx < toks.len() && toks[idx].kind == TokenKind::Mut {
            idx += 1;
            while idx < toks.len() && toks[idx].is_trivia() {
                idx += 1;
            }
        }
        if idx < toks.len() && toks[idx].kind == TokenKind::SelfKw && drv_tb + toks[idx].start == name_start {
            return MatchKind::StrictLocalRefSelf;
        }
        return MatchKind::NoMatch;
    }
    let mut saw_ref_mut = false;
    while idx < toks.len() {
        if toks[idx].is_trivia() {
            idx += 1;
            continue;
        }
        if matches!(toks[idx].kind, TokenKind::Ref | TokenKind::Mut) {
            saw_ref_mut = true;
            idx += 1;
            continue;
        }
        break;
    }
    if saw_ref_mut && idx < toks.len() && drv_tb + toks[idx].start == name_start {
        MatchKind::StrictLocalRefMut
    } else {
        MatchKind::NoMatch
    }
}

pub fn ref_targets_match(r_idx: usize, reader: &IndexReader<Mmap>, db: &RidxV5ItemDb<'_>, cf: &CorpusFile, out: &FileNameRes, our: ResTarget, drv: &RefEntry) -> MatchKind {
    let dkind = DefKind::from(drv.kind);
    match dkind {
        DefKind::Local => {
            let ResTarget::Local(lid) = our else {
                return MatchKind::NoMatch;
            };
            let lb = out.locals[lid.0 as usize];
            let our_name = norm_ident(lb.name(&cf.text));
            let drv_name = norm_ident(reader.get_str(drv.target_path_offset()));
            if our_name != drv_name {
                return MatchKind::NoMatch;
            }
            if let Some(drv_tb) = cf.lines.to_byte(&cf.text, drv.target_line(), drv.target_col()) {
                return match_local_name_start(&cf.text, drv_tb, lb.name_start, lb.is_self);
            }
            MatchKind::Strict
        }
        DefKind::TyParam | DefKind::ConstParam => {
            let ResTarget::Generic(gid) = our else {
                return MatchKind::NoMatch;
            };
            let gb = out.generics[gid.0 as usize];
            let (ol, _) = cf.lines.to_line_col(&cf.text, gb.name_start);
            if ol == drv.target_line() {
                MatchKind::Strict
            } else {
                MatchKind::NoMatch
            }
        }
        _ => {
            if let ResTarget::Prim(p) = our {
                if dkind == DefKind::Impl {
                    if let Some(drv_gid) = resolve_driver_gid(r_idx, reader, db, drv) {
                        if db.impl_self_target.get(&drv_gid) == Some(&ImplTargetKey::Prim(p)) {
                            return MatchKind::Strict;
                        }
                    }
                }
                return MatchKind::NoMatch;
            }
            if let ResTarget::SelfTy { target, trait_def } = our {
                if dkind == DefKind::Impl {
                    if let Some(drv_gid) = resolve_driver_gid(r_idx, reader, db, drv) {
                        if target != ImplTargetKey::Unknown && db.impl_self_target.get(&drv_gid) == Some(&target) {
                            return MatchKind::Strict;
                        }
                    }
                }
                let our_gid = match target {
                    ImplTargetKey::Adt(adt_def) => adt_def,
                    _ if trait_def.is_some() => trait_def,
                    _ => return MatchKind::NoMatch,
                };
                if !our_gid.is_some() || our_gid.is_overlay() {
                    return MatchKind::NoMatch;
                }
                return driver_def_matches_gid(r_idx, reader, db, drv, our_gid, ItemKind::Struct);
            }
            let ResTarget::Def { def: our_gid, kind: our_kind, .. } = our else {
                return MatchKind::NoMatch;
            };
            if !our_gid.is_some() || our_gid.is_overlay() {
                return MatchKind::NoMatch;
            }
            driver_def_matches_gid(r_idx, reader, db, drv, our_gid, our_kind)
        }
    }
}

fn resolve_driver_gid(r_idx: usize, reader: &IndexReader<Mmap>, db: &RidxV5ItemDb<'_>, drv: &RefEntry) -> Option<GlobalDefId> {
    if drv.target_def_id() != u32::MAX {
        if let Some(&didx) = db.shards[r_idx].def_id_to_idx.get(&drv.target_def_id()) {
            return Some(GlobalDefId::new(CrateId(r_idx as u32), didx));
        }
    }
    let tu = reader.get_str(drv.target_unit_offset());
    let drv_path = reader.get_str(drv.target_path_offset());
    let target_r = db.resolve_unit_to_reader(r_idx, tu).or_else(|| {
        let first = drv_path.split("::").next().unwrap_or("");
        db.resolve_extern_name_to_reader(r_idx, first)
    })?;
    let drv_leaf_raw = drv_path.rsplit("::").next().unwrap_or(drv_path);
    let drv_leaf = norm_ident(drv_leaf_raw);
    if drv.target_line() > 0 {
        if let Some(cands) = db.shards[target_r].line_col_defs.get(&(drv.target_line(), drv.target_col())) {
            let tr_reader = &db.ws.readers()[target_r];
            if let Some(&didx) = cands.iter().find(|&&didx| tr_reader.definitions().get(didx as usize).is_some_and(|d| norm_ident(tr_reader.get_str(d.name_offset())) == drv_leaf))
            {
                return Some(GlobalDefId::new(CrateId(target_r as u32), didx));
            }
            if cands.len() == 1 {
                return Some(GlobalDefId::new(CrateId(target_r as u32), cands[0]));
            }
        }
    }
    let g = db.find_def_in_reader_by_path(target_r, drv_path);
    g.is_some().then_some(g)
}

fn trait_has_super(db: &RidxV5ItemDb<'_>, sub_trait: GlobalDefId, target_super: GlobalDefId, depth: u8) -> bool {
    if sub_trait == target_super {
        return true;
    }
    if depth > 8 {
        return false;
    }
    let Some(supers) = db.trait_supertraits.get(&sub_trait) else {
        return false;
    };
    supers.iter().any(|&s| trait_has_super(db, s, target_super, depth + 1))
}

fn driver_def_matches_gid(r_idx: usize, reader: &IndexReader<Mmap>, db: &RidxV5ItemDb<'_>, drv: &RefEntry, our_gid: GlobalDefId, our_kind: ItemKind) -> MatchKind {
    let dkind = DefKind::from(drv.kind);
    let drv_path = reader.get_str(drv.target_path_offset());

    let drv_gid_opt = resolve_driver_gid(r_idx, reader, db, drv);
    if !matches!(dkind, DefKind::StructCtor | DefKind::VariantCtor) && drv_gid_opt == Some(our_gid) {
        return MatchKind::Strict;
    }

    // `hir_visitor.rs:169,173` synthesizes `Enum`/`Struct` on `Alias` in `Alias::Variant` / `Alias { .. }`.
    if our_kind == ItemKind::TypeAlias && matches!(dkind, DefKind::Enum | DefKind::Struct | DefKind::Union | DefKind::StructCtor) {
        if let Some(drv_gid) = drv_gid_opt {
            if db.type_alias_target(our_gid) == Some(ImplTargetKey::Adt(drv_gid)) {
                return MatchKind::Strict;
            }
        }
    }

    // Check strict path / span identity FIRST so `LenientAssocTrait` counts ONLY non-strict matches.
    let our_r_idx = our_gid.krate.0 as usize;
    let our_cname = db.metas[our_r_idx].crate_name.as_str();
    let (drv_cname, drv_rel) = if drv.target_def_id() != u32::MAX {
        (db.metas[r_idx].crate_name.as_str(), drv_path)
    } else {
        let tu = reader.get_str(drv.target_unit_offset());
        let cn = db.resolve_unit_to_reader(r_idx, tu).map(|i| db.metas[i].crate_name.as_str()).unwrap_or_else(|| drv_path.split("::").next().unwrap_or(""));
        let dp = format!("{cn}::");
        (cn, drv_path.strip_prefix(dp.as_str()).unwrap_or(drv_path))
    };

    if (our_gid.index & PROC_MACRO_SYNTH_BIT) != 0 {
        let sym_idx = (our_gid.index & !PROC_MACRO_SYNTH_BIT) as usize;
        let Some(our_mname) = db.shards[our_r_idx].synth_macros.get(sym_idx) else {
            return MatchKind::NoMatch;
        };
        let drv_leaf = drv_rel.rsplit("::").next().unwrap_or(drv_rel);
        return if our_cname == drv_cname && norm_ident(our_mname) == norm_ident(drv_leaf) { MatchKind::Strict } else { MatchKind::NoMatch };
    }
    if our_gid.index == CRATE_ROOT_DEF_IDX || Some(our_gid.index) == db.shards[our_r_idx].root_mod_def_idx {
        return if drv_path.is_empty() || drv_path == our_cname { MatchKind::Strict } else { MatchKind::NoMatch };
    }
    if (our_gid.index & IMPL_SYNTH_BIT) == 0 {
        let our_reader = &db.ws.readers()[our_r_idx];
        if let Some(our_def) = our_reader.definitions().get(our_gid.index as usize) {
            let our_raw_path = our_reader.get_str(our_def.path_offset());
            let our_prefix = format!("{our_cname}::");
            let our_rel = if db.metas[our_r_idx].is_sysroot { our_raw_path.strip_prefix(our_prefix.as_str()).unwrap_or(our_raw_path) } else { our_raw_path };
            if our_cname == drv_cname && our_rel == drv_rel {
                return MatchKind::Strict;
            }
            let osp = our_def.span();
            let our_name = our_reader.get_str(our_def.name_offset());
            let drv_leaf = drv_path.rsplit("::").next().unwrap_or(drv_path);
            if drv.target_line() > 0 && osp.start_line() == drv.target_line() && osp.start_col() == drv.target_col() && norm_ident(our_name) == norm_ident(drv_leaf) {
                return MatchKind::Strict;
            }
        }
    }

    // Lenient branch: `Instance::try_resolve` monomorphized `Trait::assoc` / `Ty::trait_assoc` to `<Concrete as Trait>::assoc`.
    if matches!(dkind, DefKind::AssocFn | DefKind::AssocConst | DefKind::AssocTy) && (our_gid.index & (IMPL_SYNTH_BIT | PROC_MACRO_SYNTH_BIT)) == 0 {
        let our_name = db.ws.readers()[our_r_idx].definitions().get(our_gid.index as usize).map_or("", |d| db.ws.readers()[our_r_idx].get_str(d.name_offset()));
        let drv_leaf = drv_path.rsplit("::").next().unwrap_or(drv_path);
        if norm_ident(our_name) == norm_ident(drv_leaf) {
            let mut drv_tr_cand = drv_gid_opt.and_then(|dg| db.impl_assoc_parent.get(&dg).map(|&(tr, _)| tr)).filter(|g| g.is_some());
            if drv_tr_cand.is_none() {
                if let Some(ufcs_tr_str) = extract_ufcs_trait_path(drv_path) {
                    let first = ufcs_tr_str.split("::").next().unwrap_or(ufcs_tr_str);
                    let tr_r = db.resolve_extern_name_to_reader(r_idx, first).unwrap_or(r_idx);
                    let g = db.find_def_in_reader_by_path(tr_r, ufcs_tr_str);
                    if g.is_some() {
                        drv_tr_cand = Some(g);
                    }
                }
            }
            if let Some(drv_tr) = drv_tr_cand {
                if let Some(&our_tr_didx) = db.shards[our_r_idx].trait_parent_of_assoc.get(&our_gid.index) {
                    let our_tr = GlobalDefId::new(our_gid.krate, our_tr_didx);
                    if trait_has_super(db, drv_tr, our_tr, 0) {
                        return MatchKind::LenientAssocTrait;
                    }
                }
                if let Some(&(our_tr, our_target)) = db.impl_assoc_parent.get(&our_gid) {
                    if let Some(drv_gid) = drv_gid_opt {
                        if let Some(&(_, drv_target)) = db.impl_assoc_parent.get(&drv_gid) {
                            if our_tr == drv_tr && our_target == drv_target && our_target != ImplTargetKey::Unknown {
                                return MatchKind::LenientAssocTrait;
                            }
                        }
                    }
                }
            }
        }
    }

    MatchKind::NoMatch
}

fn extract_ufcs_trait_path(drv_path: &str) -> Option<&str> {
    let bytes = drv_path.as_bytes();
    if bytes.first() != Some(&b'<') {
        return None;
    }
    let mut depth = 0i32;
    let mut as_end = None;
    let mut close_gt = None;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'-' if bytes.get(i + 1) == Some(&b'>') => {
                i += 2;
                continue;
            }
            b'<' => depth += 1,
            b'>' => {
                depth -= 1;
                if depth == 0 {
                    close_gt = Some(i);
                    break;
                }
            }
            b' ' if depth == 1 && drv_path[i..].starts_with(" as ") => {
                as_end = Some(i + 4);
                i += 4;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    let raw_tr = drv_path.get(as_end?..close_gt?)?.trim();
    let base = raw_tr.split('<').next()?.trim_end_matches("::").trim();
    (!base.is_empty()).then_some(base)
}

pub fn describe_our_target(db: &RidxV5ItemDb<'_>, cf: &CorpusFile, out: &FileNameRes, our: ResTarget) -> String {
    match our {
        ResTarget::Local(lid) => {
            let lb = out.locals[lid.0 as usize];
            let (l, c) = cf.lines.to_line_col(&cf.text, lb.name_start);
            format!("Local({}:{l}:{c})", lb.name(&cf.text))
        }
        ResTarget::Generic(gid) => {
            let gb = out.generics[gid.0 as usize];
            let (l, c) = cf.lines.to_line_col(&cf.text, gb.name_start);
            format!("Generic({}:{l}:{c})", gb.name(&cf.text))
        }
        ResTarget::Prim(p) => format!("Prim({p:?})"),
        ResTarget::SelfTy { target, trait_def } => format!("SelfTy({target:?}, tr={trait_def:?})"),
        ResTarget::Def { def, local, kind } => {
            if !def.is_some() || def.is_overlay() {
                return format!("OverlayDef(local={local:?}, kind={kind:?})");
            }
            let r_idx = def.krate.0 as usize;
            let cname = &db.metas[r_idx].crate_name;
            if def.index == CRATE_ROOT_DEF_IDX {
                return format!("{cname}::<root>({kind:?})");
            }
            let reader = &db.ws.readers()[r_idx];
            if let Some(d) = reader.definitions().get(def.index as usize) {
                let p = reader.get_str(d.path_offset());
                let sp = d.span();
                format!("{cname}::{p}({kind:?}@{}:{})", sp.start_line(), sp.start_col())
            } else {
                format!("{cname}::#{}({kind:?})", def.index)
            }
        }
    }
}
