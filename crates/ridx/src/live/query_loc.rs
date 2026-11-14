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
use ridx_syntax::ty_infer::{FnBodySummary, LocalCstDefs};
use ridx_syntax::{NodeId, SyntaxKind as K, SyntaxTree};

use crate::overlay::OverlayManager;

use super::meta::LineIndex;
use super::query::LiveLocation;
use super::v5_db::{RidxV5ItemDb, CRATE_ROOT_DEF_IDX, IMPL_SYNTH_BIT, PROC_MACRO_SYNTH_BIT, TY_DEF_SYNTH_BIT};
use super::v5_ty_tables::fnv1a_str;

pub(crate) struct CachedParsed {
    pub file: ridx_syntax::name_res::FileId,
    pub text_hash: u64,
    pub src: std::sync::Arc<String>,
    pub lines: LineIndex,
    pub(crate) bufs: std::sync::Mutex<(Vec<ridx_syntax::Token>, Vec<ridx_syntax::tree::Node>)>,
    pub fn_summaries: Vec<FnBodySummary>,
    pub focused_span: Option<(u32, u32)>,
}

pub(crate) struct TreeLease<'a> {
    owner: Option<&'a std::sync::Mutex<(Vec<ridx_syntax::Token>, Vec<ridx_syntax::tree::Node>)>>,
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
        if let Some(m) = self.owner.take() {
            *m.lock().unwrap_or_else(|e| e.into_inner()) = (std::mem::take(&mut self.st.tokens), std::mem::take(&mut self.st.nodes));
        }
    }
}

impl CachedParsed {
    #[inline]
    pub fn tree(&self) -> TreeLease<'_> {
        let (tokens, nodes) = std::mem::take(&mut *self.bufs.lock().unwrap_or_else(|e| e.into_inner()));
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
        return out.write_str(anm.as_str(db.ws));
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

pub(crate) use super::query_defs::collect_ty_def_locs;

pub(crate) fn resolve_def_location(
    db: &RidxV5ItemDb<'_>,
    dm: &CrateDefMap,
    active_canon: &Path,
    parsed: &CachedParsed,
    local_defs: Option<&LocalCstDefs>,
    raw_def: GlobalDefId,
) -> Option<LiveLocation> {
    if raw_def.krate == CrateId::LOCAL_OVERLAY {
        let src_path = |fid: u32| db.src.files.get(fid as usize).map(|cf| db.src.ws_root.join(&cf.rel_path));
        if let Some(ld) = local_defs {
            let nraw = ld.fn_node_for_def(raw_def);
            if nraw != u32::MAX {
                let st = parsed.tree();
                let target_n = st.child(NodeId(nraw), K::Name).unwrap_or(NodeId(nraw));
                let r = st.range(target_n);
                let (sl, sc) = parsed.lines.to_line_col(&parsed.src, r.start as u32);
                let (el, ec) = parsed.lines.to_line_col(&parsed.src, r.end as u32);
                return Some(LiveLocation { file: src_path(parsed.file.0).unwrap_or_else(|| active_canon.to_path_buf()), line: sl, col: sc, end_line: el, end_col: ec });
            }
        }
        let it = dm.items.iter().find(|it| it.file.is_some() && (it.index_def == raw_def || (0x8000_0000 | it.id.0) == raw_def.index || it.id.0 == raw_def.index))?;
        let (cf, fpath) = (db.src.files.get(it.file.0 as usize)?, src_path(it.file.0)?);
        let (sl, sc) = cf.lines.to_line_col(&cf.text, it.span_start);
        let (el, ec) = cf.lines.to_line_col(&cf.text, it.span_end);
        return Some(LiveLocation { file: fpath, line: sl, col: sc, end_line: el, end_col: ec });
    }
    let def = db.canonical_def(raw_def);
    if !def.is_some() || matches!(def.index, CRATE_ROOT_DEF_IDX) || (def.index & (IMPL_SYNTH_BIT | PROC_MACRO_SYNTH_BIT)) != 0 {
        return None;
    }
    if (def.index & TY_DEF_SYNTH_BIT) != 0 {
        return resolve_synth_ty_def_location(db, def);
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

fn resolve_synth_ty_def_location(db: &RidxV5ItemDb<'_>, def: GlobalDefId) -> Option<LiveLocation> {
    if (def.index & 0x4000_0000) != 0 {
        return None;
    }
    if let Ok(cache) = db.ty_tables.synth_def_locs.lock() {
        if let Some(cached) = cache.get(&def) {
            return cached.clone();
        }
    }
    let found = find_synth_ty_def_on_disk(db, def);
    if let Ok(mut cache) = db.ty_tables.synth_def_locs.lock() {
        cache.insert(def, found.clone());
    }
    found
}

fn find_synth_ty_def_on_disk(db: &RidxV5ItemDb<'_>, def: GlobalDefId) -> Option<LiveLocation> {
    let r_idx = def.krate.0 as usize;
    let rdr = db.ws.readers().get(r_idx)?;
    let td = rdr.ty_arena().def(ridx_format::TyDefId(def.index & !TY_DEF_SYNTH_BIT))?;
    let root = if db.metas[r_idx].is_dependency { db.metas[r_idx].source_root.as_deref()? } else { db.metas[r_idx].source_root.as_deref().unwrap_or(db.src.ws_root.as_path()) };
    let cname = db.metas[r_idx].crate_name.as_str();
    let rel_path = td.path.strip_prefix(cname).and_then(|s| s.strip_prefix("::")).unwrap_or(td.path);
    let (mod_rel, raw_item) = rel_path.rsplit_once("::").unwrap_or(("", rel_path));
    let item = raw_item.trim_start_matches("r#");
    if item.is_empty() {
        return None;
    }
    let src_dir = (0..rdr.header().files_count)
        .map(|fid| rdr.get_file(fid))
        .find(|f| f.ends_with("/lib.rs") || f.ends_with("\\lib.rs") || *f == "src/lib.rs" || *f == "lib.rs")
        .and_then(|f| Path::new(f).parent())
        .map(|p| join_resolving_overlap(root, p))
        .unwrap_or_else(|| root.join("src"));
    let mod_slash = mod_rel.replace("::", "/");
    let mut cands = Vec::with_capacity(5);
    if mod_slash.is_empty() {
        cands.push(src_dir.join("lib.rs"));
    } else {
        cands.push(src_dir.join(format!("{mod_slash}.rs")));
        cands.push(src_dir.join(&mod_slash).join("mod.rs"));
        if let Some((parent_mod, _)) = mod_slash.rsplit_once('/') {
            cands.push(src_dir.join(format!("{parent_mod}.rs")));
            cands.push(src_dir.join(parent_mod).join("mod.rs"));
        } else {
            cands.push(src_dir.join("lib.rs"));
        }
    }
    for cand in cands {
        let Ok(text) = std::fs::read_to_string(&cand) else { continue };
        for (idx, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            for kw in ["struct ", "enum ", "union ", "trait ", "type "] {
                if let Some(pos) = line.find(kw) {
                    if pos > 0 && line.as_bytes()[pos - 1].is_ascii_alphanumeric() {
                        continue;
                    }
                    let after = &line[pos + kw.len()..];
                    let ident_str = after.strip_prefix("r#").unwrap_or(after);
                    if let Some(rest) = ident_str.strip_prefix(item) {
                        if rest.chars().next().is_none_or(|c| !c.is_alphanumeric() && c != '_') {
                            let name_byte = line.len() - after.len();
                            let start_col = line[..name_byte].chars().count() as u32 + 1;
                            let end_col = start_col + (after.len() - rest.len()) as u32;
                            let ln = (idx + 1) as u32;
                            return Some(LiveLocation { file: cand, line: ln, col: start_col, end_line: ln, end_col });
                        }
                    }
                }
            }
        }
    }
    None
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
    let owner_mod = super::assoc_path::assoc_owner_module(dp);
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
    if let Some(s) = summaries.iter().filter(|s| byte_off >= s.fn_start && byte_off <= s.fn_end).min_by_key(|s| s.fn_end - s.fn_start) {
        return Some((s.fn_node, s.fn_key, s.body_hash ^ s.sig_hash.rotate_left(7)));
    }
    let b = byte_off as usize;
    let (mut cur, mut best_fn, mut first_cs) = (st.root(), None, None);
    while let Some(ch) = st.children(cur).find(|&c| (st.range(c).start..=st.range(c).end).contains(&b)) {
        if summaries.is_empty() && st.kind(ch) == K::Fn && st.child(ch, K::BlockExpr).is_some() {
            best_fn = Some(ch);
        } else if first_cs.is_none() && matches!(st.kind(ch), K::Const | K::Static) {
            first_cs = Some(ch);
        }
        cur = ch;
    }
    let best_fn = best_fn.filter(|&fn_node| {
        st.child(fn_node, K::BlockExpr).is_none_or(|body| {
            b >= st.range(body).start
                || (st.child(fn_node, K::ParamList).is_some_and(|pl| (st.range(pl).start..=st.range(pl).end).contains(&b))
                    && super::query_cursor::smallest_type_node_at(st, byte_off, None).is_none())
        })
    });
    let first_cs = first_cs.filter(|&cs| st.children(cs).find(|&c| st.kind(c).is_expr() || st.kind(c) == K::MacroCall).is_some_and(|e| b >= st.range(e).start));
    best_fn
        .map(|n| (n, (u64::from(st.range(n).start as u32) << 32) | u64::from(n.0), u64::MAX))
        .or_else(|| first_cs.map(|n| (n, 0xC057_0000_0000_0000 | (u64::from(st.range(n).start as u32) << 20) | u64::from(n.0), fnv1a_str(st.text(n)))))
}

pub(crate) fn lookup_live_text(abs: &Path, canon: &Path, manual: &HashMap<PathBuf, String>, overlays: Option<&OverlayManager>) -> Option<String> {
    if let Some(t) = manual.get(canon).or_else(|| manual.get(abs)) {
        return Some(t.clone());
    }
    let ov = overlays?;
    if let Some(s) = canon.to_str().and_then(|s| ov.get(s)).or_else(|| abs.to_str().and_then(|s| ov.get(s))) {
        return Some(s.text.clone());
    }
    ov.open_file_paths()
        .into_iter()
        .filter(|ps| {
            let p = Path::new(ps);
            p == abs || p == canon || p.canonicalize().is_ok_and(|c| c == canon)
        })
        .find_map(|ps| ov.get(&ps).map(|d| d.text.clone()))
}
