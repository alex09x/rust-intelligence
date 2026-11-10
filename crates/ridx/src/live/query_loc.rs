/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Definition location resolution, item name/signature formatting, visibility checks,
//! and overlay text / enclosing-function helpers for [`super::query::LiveDb`].

use std::collections::HashMap;
use std::fmt::Write;
use std::path::{Path, PathBuf};

use ridx_format::{join_resolving_overlap, DefKind, ItemVis};
use ridx_syntax::name_res::{CrateDefMap, CrateId, GlobalDefId, ItemKind as IK, ModuleId};
use ridx_syntax::ty_infer::{FnBodySummary, InferTyId, InferTyKind, LocalCstDefs};
use ridx_syntax::{NodeId, SyntaxKind as K, SyntaxTree};

use crate::overlay::OverlayManager;

use super::meta::LineIndex;
use super::query::LiveLocation;
use super::query_ty_fmt::TyReader;
use super::v5_db::{RidxV5ItemDb, CRATE_ROOT_DEF_IDX, IMPL_SYNTH_BIT, PROC_MACRO_SYNTH_BIT, TY_DEF_SYNTH_BIT};
use super::v5_ty_tables::fnv1a_str;

pub(crate) struct CachedParsed {
    pub file: ridx_syntax::name_res::FileId,
    pub text_hash: u64,
    pub src: String,
    pub lines: LineIndex,
    pub(crate) bufs: std::cell::Cell<(Vec<ridx_syntax::Token>, Vec<ridx_syntax::tree::Node>)>,
    pub fn_summaries: Vec<FnBodySummary>,
}

pub(crate) struct TreeLease<'a> {
    owner: Option<&'a std::cell::Cell<(Vec<ridx_syntax::Token>, Vec<ridx_syntax::tree::Node>)>>,
    st: SyntaxTree<'a>,
}

impl<'a> std::ops::Deref for TreeLease<'a> {
    type Target = SyntaxTree<'a>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.st
    }
}

impl Drop for TreeLease<'_> {
    #[inline]
    fn drop(&mut self) {
        if let Some(cell) = self.owner.take() {
            cell.set((std::mem::take(&mut self.st.tokens), std::mem::take(&mut self.st.nodes)));
        }
    }
}

impl CachedParsed {
    #[inline]
    pub fn tree(&self) -> TreeLease<'_> {
        let (tokens, nodes) = self.bufs.take();
        if !nodes.is_empty() || self.src.is_empty() {
            TreeLease { owner: Some(&self.bufs), st: SyntaxTree { src: &self.src, tokens, nodes } }
        } else {
            TreeLease { owner: None, st: ridx_syntax::parse(&self.src) }
        }
    }
}

pub(crate) fn write_def_name(
    db: &RidxV5ItemDb<'_>,
    dm: &CrateDefMap,
    parsed: &CachedParsed,
    local_defs: Option<&LocalCstDefs>,
    raw_def: GlobalDefId,
    qualified: bool,
    out: &mut String,
) -> std::fmt::Result {
    if raw_def.krate == CrateId::LOCAL_OVERLAY {
        if let Some(ld) = local_defs {
            let nraw = ld.fn_node_for_def(raw_def);
            if nraw != u32::MAX {
                let st = parsed.tree();
                if let Some(nn) = st.child(NodeId(nraw), K::Name) {
                    return out.write_str(st.text(nn).trim_start_matches("r#"));
                }
            }
        }
        if let Some(it) = dm.items.iter().find(|it| it.file.is_some() && (it.index_def == raw_def || (0x8000_0000 | it.id.0) == raw_def.index || it.id.0 == raw_def.index)) {
            return out.write_str(dm.item_name(it.id));
        }
        return out.write_str("{local}");
    }
    let def = db.canonical_def(raw_def);
    let r_idx = def.krate.0 as usize;
    if let Some(rdr) = db.ws.readers().get(r_idx) {
        if (def.index & TY_DEF_SYNTH_BIT) != 0 && (def.index & 0x4000_0000) == 0 {
            let td_idx = def.index & !TY_DEF_SYNTH_BIT;
            if let Some(td) = rdr.ty_arena().def(ridx_format::TyDefId(td_idx)) {
                let s = if qualified { td.path } else { td.path.rsplit("::").next().unwrap_or(td.path) };
                return out.write_str(s.trim_start_matches("r#"));
            }
        }
        if let Some(d) = rdr.definitions().get(def.index as usize) {
            if qualified {
                let p = rdr.get_str(d.path_offset());
                let cname = db.metas[r_idx].crate_name.as_str();
                if db.metas[r_idx].is_sysroot || p.starts_with(cname) {
                    return out.write_str(p);
                }
                return write!(out, "{cname}::{p}");
            }
            return out.write_str(rdr.get_str(d.name_offset()).trim_start_matches("r#"));
        }
    }
    if let Some(&anm) = db.ty_tables.assoc_ty_names.get(&def) {
        return out.write_str(anm);
    }
    out.write_str("{unknown}")
}

pub(crate) fn def_signature_detail(db: &RidxV5ItemDb<'_>, parsed: &CachedParsed, local_defs: Option<&LocalCstDefs>, raw_def: GlobalDefId) -> Option<String> {
    if raw_def.krate == CrateId::LOCAL_OVERLAY {
        let nraw = local_defs?.fn_node_for_def(raw_def);
        if nraw == u32::MAX {
            return None;
        }
        let st = parsed.tree();
        let node = NodeId(nraw);
        let r = st.range(node);
        let end = st.child(node, K::BlockExpr).map_or(r.end, |b| st.range(b).start);
        return parsed.src.get(r.start..end).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    }
    let def = db.canonical_def(raw_def);
    let rdr = db.ws.readers().get(def.krate.0 as usize)?;
    let d = rdr.definitions().get(def.index as usize)?;
    let sig = rdr.get_str(d.sig_offset());
    (!sig.is_empty()).then(|| sig.to_string())
}

pub(crate) fn lookup_cst_param_name(parsed: &CachedParsed, fn_node: Option<NodeId>, idx: u16) -> Option<&str> {
    let fn_n = fn_node?;
    let st = parsed.tree();
    let mut cur = 0u16;
    if let Some(imp) = st.ancestors(fn_n).find(|&a| matches!(st.kind(a), K::Impl | K::Trait)) {
        if st.kind(imp) == K::Trait {
            if idx == 0 {
                return Some("Self");
            }
            cur += 1;
        }
        if let Some(gpl) = st.child(imp, K::GenericParamList) {
            for ch in st.children(gpl).filter(|&c| matches!(st.kind(c), K::TypeParam | K::ConstParam)) {
                if cur == idx {
                    let r = st.range(st.child(ch, K::Name)?);
                    return parsed.src.get(r.start..r.end).map(|s| s.trim_start_matches("r#"));
                }
                cur += 1;
            }
        }
    }
    let gpl = st.child(fn_n, K::GenericParamList)?;
    for ch in st.children(gpl).filter(|&c| matches!(st.kind(c), K::TypeParam | K::ConstParam)) {
        if cur == idx {
            let r = st.range(st.child(ch, K::Name)?);
            return parsed.src.get(r.start..r.end).map(|s| s.trim_start_matches("r#"));
        }
        cur += 1;
    }
    None
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn collect_ty_def_locs<A: TyReader>(
    db: &RidxV5ItemDb<'_>,
    dm: &CrateDefMap,
    active_canon: &Path,
    parsed: &CachedParsed,
    local_defs: Option<&LocalCstDefs>,
    arena: &A,
    ty: InferTyId,
    depth: u8,
    out: &mut Vec<LiveLocation>,
) {
    if !ty.is_some() || depth > 24 {
        return;
    }
    let mut push_gid = |g: GlobalDefId| {
        if !g.is_some() || db.ty_tables.global_gid.is_some_and(|gg| db.canonical_def(g) == gg) {
            return;
        }
        if let Some(loc) = resolve_def_location(db, dm, active_canon, parsed, local_defs, g) {
            if !out.contains(&loc) {
                out.push(loc);
            }
        }
    };
    match arena.kind(ty) {
        InferTyKind::Adt { def, args } => {
            push_gid(def);
            for &a in arena.slice(args) {
                collect_ty_def_locs(db, dm, active_canon, parsed, local_defs, arena, a, depth + 1, out);
            }
        }
        InferTyKind::Dyn { principal, args } | InferTyKind::Opaque { def: principal, args } => {
            push_gid(principal);
            for &a in arena.slice(args) {
                collect_ty_def_locs(db, dm, active_canon, parsed, local_defs, arena, a, depth + 1, out);
            }
        }
        InferTyKind::Alias { trait_def, assoc_def, self_ty, args } => {
            push_gid(assoc_def);
            push_gid(trait_def);
            collect_ty_def_locs(db, dm, active_canon, parsed, local_defs, arena, self_ty, depth + 1, out);
            for &a in arena.slice(args) {
                collect_ty_def_locs(db, dm, active_canon, parsed, local_defs, arena, a, depth + 1, out);
            }
        }
        InferTyKind::Ref { inner, .. } | InferTyKind::Ptr { inner, .. } | InferTyKind::Slice(inner) | InferTyKind::Array { elem: inner, .. } => {
            collect_ty_def_locs(db, dm, active_canon, parsed, local_defs, arena, inner, depth + 1, out);
        }
        InferTyKind::Tuple(r) | InferTyKind::FnPtr { sig: r } | InferTyKind::Closure { sig: r, .. } | InferTyKind::FnDef { args: r, .. } => {
            for &el in arena.slice(r) {
                collect_ty_def_locs(db, dm, active_canon, parsed, local_defs, arena, el, depth + 1, out);
            }
        }
        InferTyKind::Param { def, .. } => push_gid(def),
        _ => {}
    }
}

pub(crate) fn resolve_def_location(
    db: &RidxV5ItemDb<'_>,
    dm: &CrateDefMap,
    active_canon: &Path,
    parsed: &CachedParsed,
    local_defs: Option<&LocalCstDefs>,
    raw_def: GlobalDefId,
) -> Option<LiveLocation> {
    if raw_def.krate == CrateId::LOCAL_OVERLAY {
        if let Some(ld) = local_defs {
            let nraw = ld.fn_node_for_def(raw_def);
            if nraw != u32::MAX {
                let st = parsed.tree();
                let target_n = st.child(NodeId(nraw), K::Name).unwrap_or(NodeId(nraw));
                let r = st.range(target_n);
                let (sl, sc) = parsed.lines.to_line_col(&parsed.src, r.start as u32);
                let (el, ec) = parsed.lines.to_line_col(&parsed.src, r.end as u32);
                return Some(LiveLocation { file: active_canon.to_path_buf(), line: sl, col: sc, end_line: el, end_col: ec });
            }
        }
        let it = dm.items.iter().find(|it| it.file.is_some() && (it.index_def == raw_def || (0x8000_0000 | it.id.0) == raw_def.index || it.id.0 == raw_def.index))?;
        let cf = db.src.files.get(it.file.0 as usize)?;
        let fpath = if Path::new(&cf.rel_path).is_absolute() { PathBuf::from(&cf.rel_path) } else { db.src.ws_root.join(&cf.rel_path) };
        let (sl, sc) = cf.lines.to_line_col(&cf.text, it.span_start);
        let (el, ec) = cf.lines.to_line_col(&cf.text, it.span_end);
        return Some(LiveLocation { file: fpath, line: sl, col: sc, end_line: el, end_col: ec });
    }
    let def = db.canonical_def(raw_def);
    if !def.is_some() || matches!(def.index, CRATE_ROOT_DEF_IDX) || (def.index & (IMPL_SYNTH_BIT | PROC_MACRO_SYNTH_BIT | TY_DEF_SYNTH_BIT)) != 0 {
        return None;
    }
    let r_idx = def.krate.0 as usize;
    let rdr = db.ws.readers().get(r_idx)?;
    let d = rdr.definitions().get(def.index as usize)?;
    let sp = d.span();
    if sp.start_line() == 0 {
        return None;
    }
    let raw_file = rdr.get_file(sp.file_id());
    if raw_file.is_empty() {
        return None;
    }
    let rel = Path::new(raw_file);
    let file = if rel.is_absolute() {
        rel.to_path_buf()
    } else {
        let root = if db.metas[r_idx].is_dependency { db.metas[r_idx].source_root.as_deref() } else { db.metas[r_idx].source_root.as_deref().or(Some(db.src.ws_root.as_path())) };
        root.map_or_else(|| rel.to_path_buf(), |r| join_resolving_overlap(r, rel))
    };
    Some(LiveLocation { file, line: sp.start_line(), col: sp.start_col(), end_line: sp.end_line(), end_col: sp.end_col() })
}

#[inline]
pub(crate) fn is_def_visible_from(db: &RidxV5ItemDb<'_>, cur_krate: CrateId, cur_mod: ModuleId, dm: &CrateDefMap, raw_def: GlobalDefId) -> bool {
    if raw_def.krate == CrateId::LOCAL_OVERLAY {
        return true;
    }
    let def = db.canonical_def(raw_def);
    let r_idx = def.krate.0 as usize;
    let Some(rdr) = db.ws.readers().get(r_idx) else { return false };
    let Some(d) = rdr.definitions().get(def.index as usize) else { return true };
    if d.visibility == ItemVis::Public as u8 {
        return true;
    }
    if def.krate != cur_krate {
        return false;
    }
    if d.visibility == ItemVis::Crate as u8 {
        return true;
    }
    let mod_path = dm.mod_tree.crate_path(cur_mod);
    let cur_rel = mod_path.strip_prefix("crate::").unwrap_or(if mod_path == "crate" { "" } else { mod_path });
    let dp = rdr.get_str(d.path_offset());
    let owner_mod = dp.rsplit_once("::").map_or("", |(p, _)| p).rsplit_once("::").map_or("", |(mp, _)| mp);
    owner_mod.is_empty() || cur_rel == owner_mod || cur_rel.starts_with(&format!("{owner_mod}::"))
}

pub(crate) fn map_item_kind_to_def_kind(k: IK) -> DefKind {
    match k {
        IK::Mod | IK::ExternCrate => DefKind::Mod,
        IK::Struct => DefKind::Struct,
        IK::Enum => DefKind::Enum,
        IK::Union => DefKind::Union,
        IK::Trait => DefKind::Trait,
        IK::Fn => DefKind::Fn,
        IK::TypeAlias => DefKind::TyAlias,
        IK::Const => DefKind::Const,
        IK::Static => DefKind::Static,
        IK::Variant => DefKind::Variant,
        IK::Field => DefKind::Field,
        IK::AssocFn => DefKind::AssocFn,
        IK::AssocTy => DefKind::AssocTy,
        IK::AssocConst => DefKind::AssocConst,
        IK::Macro => DefKind::Macro,
        IK::Impl | IK::Use => DefKind::Other,
    }
}

pub(crate) fn find_enclosing_fn_summary(st: &SyntaxTree<'_>, summaries: &[FnBodySummary], byte_off: u32) -> Option<(NodeId, u64, u64)> {
    let mut best_fn: Option<&FnBodySummary> = None;
    for s in summaries {
        if byte_off >= s.fn_start && byte_off <= s.fn_end && best_fn.is_none_or(|b| (s.fn_end - s.fn_start) <= (b.fn_end - b.fn_start)) {
            best_fn = Some(s);
        }
    }
    if let Some(s) = best_fn {
        return Some((s.fn_node, s.fn_key, s.body_hash ^ s.sig_hash.rotate_left(7)));
    }
    for nid in (0..st.nodes.len() as u32).map(NodeId) {
        if matches!(st.kind(nid), K::Const | K::Static) {
            let r = st.range(nid);
            if (byte_off as usize) >= r.start && (byte_off as usize) <= r.end {
                let h = fnv1a_str(st.text(nid));
                return Some((nid, 0xC057_0000_0000_0000 | u64::from(nid.0), h));
            }
        }
    }
    None
}

pub(crate) fn lookup_live_text(abs: &Path, canon: &Path, manual: &HashMap<PathBuf, String>, overlays: Option<&OverlayManager>) -> Option<String> {
    if let Some(t) = manual.get(canon).or_else(|| manual.get(abs)) {
        return Some(t.clone());
    }
    let ov = overlays?;
    if let Some(s) = canon.to_str().and_then(|s| ov.get(s)).or_else(|| abs.to_str().and_then(|s| ov.get(s))) {
        return Some(s.text.clone());
    }
    for pstr in ov.open_file_paths() {
        let p = Path::new(&pstr);
        if p == abs || p == canon || p.canonicalize().is_ok_and(|c| c == canon) {
            if let Some(doc) = ov.get(&pstr) {
                return Some(doc.text.clone());
            }
        }
    }
    None
}
