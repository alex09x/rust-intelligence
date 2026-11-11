/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Picks the crate unit that owns a source file for [`super::query_state::LiveEngine`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ridx_format::{join_resolving_overlap, DefKind, UseOwner, WorkspaceIndex};
use ridx_syntax::name_res::{CrateId, FileId, GlobalDefId, ImplTargetKey};

use super::meta::CorpusSourceDb;
use super::v5_db::{LiveState, RidxV5ItemDb};
use super::v5_lazy::PoolStr;

/// Whether the reader at `idx` stores `path`, answered from the reader's own file table without touching the disk.
pub(super) fn reader_stores(ws: &WorkspaceIndex, idx: usize, path: &Path) -> bool {
    path.to_str().and_then(|s| ws.target_file_for_reader(idx, s)).is_some_and(|tf| ws.readers()[idx].find_file_id(tf.as_ref()).is_some())
}

/// Whether the reader at `idx` stores `abs`/`canon` under any spelling, resolving every stored path on disk.
fn reader_stores_resolved(ws: &WorkspaceIndex, state: &LiveState, idx: usize, abs: &Path, canon: &Path) -> bool {
    let rdr = &ws.readers()[idx];
    let root = state.metas[idx].source_root.as_deref().or_else(|| ws.workspace_root());
    (0..rdr.header().files_count).any(|fid| {
        let fstr = rdr.get_file(fid);
        !fstr.is_empty() && {
            let joined = root.map_or_else(|| PathBuf::from(fstr), |r| join_resolving_overlap(r, Path::new(fstr)));
            joined == abs || joined == canon || joined.canonicalize().is_ok_and(|c| c == canon)
        }
    })
}

/// The workspace unit that compiles `abs` (`canon` is its canonical form): library targets before other targets
/// before test harnesses, the unit with more references first; failing that, the unit with the deepest source
/// root above the file.
///
/// The indexed lookup runs first; the disk-resolving scan over every stored path of every unit is kept only for
/// spellings the index cannot map (symlinked roots), because it costs one `canonicalize` per stored file.
pub(super) fn find_best_unit(ws: &WorkspaceIndex, state: &mut LiveState, abs: &Path, canon: &Path) -> Option<usize> {
    for resolve_on_disk in [false, true] {
        let mut best: Option<((u8, u8, u32), usize)> = None;
        for idx in 0..ws.crate_count() {
            if state.metas[idx].is_dependency {
                continue;
            }
            let has_file =
                if resolve_on_disk { reader_stores_resolved(ws, state, idx, abs, canon) } else { reader_stores(ws, idx, abs) || (canon != abs && reader_stores(ws, idx, canon)) };
            if has_file {
                state.ensure_meta(ws, idx);
                let meta = &state.metas[idx];
                if meta.is_dependency || meta.target_kind == "custom-build" {
                    continue;
                }
                let kind_rank = match meta.target_kind.as_str() {
                    "lib" | "rlib" | "proc-macro" | "" if !meta.is_test_harness => 0,
                    _ if !meta.is_test_harness => 1,
                    _ => 2,
                };
                let key = (kind_rank, u8::from(meta.is_test_harness), u32::MAX - ws.readers()[idx].header().refs_count);
                if best.is_none_or(|(bk, _)| key < bk) {
                    best = Some((key, idx));
                }
            }
        }
        if let Some((_, idx)) = best {
            return Some(idx);
        }
    }
    let mut best_anc: Option<((usize, u8), usize)> = None;
    for idx in 0..state.metas.len() {
        if state.metas[idx].is_dependency {
            continue;
        }
        let Some(sr) = state.metas[idx].source_root.clone() else { continue };
        let sr_canon = sr.canonicalize().unwrap_or_else(|_| sr.clone());
        if canon.starts_with(&sr_canon) || abs.starts_with(&sr) {
            state.ensure_meta(ws, idx);
            let meta = &state.metas[idx];
            if meta.is_dependency || meta.target_kind == "custom-build" {
                continue;
            }
            let score = (sr_canon.as_os_str().len(), u8::from(!meta.is_test_harness));
            if best_anc.is_none_or(|(bs, _)| score > bs) {
                best_anc = Some((score, idx));
            }
        }
    }
    best_anc.map(|(_, i)| i)
}

/// Registers `fid` (`canon`) in `state.shards[r_idx]` (`file_map` and `span_to_def_idx`) and returns
/// its file-root module path (`""` for crate root, `"foo::bar"` for submodule) for `bind_indexed_file`.
pub(super) fn register_single_file_spans(ws: &WorkspaceIndex, src: &CorpusSourceDb, state: &mut LiveState, r_idx: usize, fid: FileId, canon: &Path) -> String {
    let rdr = &ws.readers()[r_idx];
    let sr = state.metas[r_idx].source_root.as_deref().or_else(|| ws.workspace_root());
    let rfid_opt = canon.to_str().and_then(|s| ws.target_file_for_reader(r_idx, s)).and_then(|tf| rdr.find_file_id(tf.as_ref())).or_else(|| {
        (0..rdr.header().files_count).find(|&i| {
            let fstr = rdr.get_file(i);
            !fstr.is_empty() && {
                let joined = sr.map_or_else(|| PathBuf::from(fstr), |r| join_resolving_overlap(r, Path::new(fstr)));
                joined == canon || joined.canonicalize().is_ok_and(|c| c == canon)
            }
        })
    });
    let Some(rfid) = rfid_opt else { return String::new() };
    let defs = rdr.definitions();
    let shard = &mut state.shards[r_idx];
    if (rfid as usize) < shard.file_map.len() && shard.file_map[rfid as usize] != fid {
        shard.file_map[rfid as usize] = fid;
        let cf = &src.files[fid.0 as usize];
        for (i, d) in defs.iter().enumerate() {
            let sp = d.span();
            if sp.file_id() != rfid || d.is_reexport() || d.is_macro_expansion() || matches!(DefKind::from(d.kind), DefKind::Use | DefKind::Impl) {
                continue;
            }
            if let (Some(sb), Some(eb)) = (cf.lines.to_byte(&cf.text, sp.start_line(), sp.start_col()), cf.lines.to_byte(&cf.text, sp.end_line(), sp.end_col())) {
                if sb < eb {
                    shard.span_to_def_idx.entry((fid, sb, eb)).or_insert(i as u32);
                }
            }
        }
    }
    if state.metas[r_idx].root_rs_abs.as_deref().is_some_and(|rp| rp == canon || rp.canonicalize().ok().as_deref() == Some(canon)) {
        return String::new();
    }
    let mut best_mod: Option<&str> = None;
    for d in defs {
        if d.span().file_id() != rfid || d.is_reexport() || d.is_macro_expansion() {
            continue;
        }
        let dk = DefKind::from(d.kind);
        if matches!(
            dk,
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
                | DefKind::Macro
        ) {
            let raw_p = rdr.get_str(d.path_offset());
            if d.def_id() == 0 || raw_p.is_empty() {
                return String::new();
            }
            let parent = raw_p.rsplit_once("::").map_or("", |(p, _)| p);
            if best_mod.is_none_or(|b| parent.len() < b.len()) {
                best_mod = Some(parent);
            }
        }
    }
    if let Some(m) = best_mod {
        return m.to_string();
    }
    for (didx, d) in defs.iter().enumerate() {
        if !d.is_reexport() && DefKind::from(d.kind) == DefKind::Mod {
            for ue in rdr.uses_for_owner(UseOwner::Def(didx as u32)) {
                if ue.def_index.and_then(|ui| defs.get(ui as usize)).is_some_and(|ud| ud.span().file_id() == rfid) {
                    return rdr.get_str(d.path_offset()).to_string();
                }
            }
        }
    }
    derive_mod_path_from_file_str(rdr.get_file(rfid), sr)
}

fn derive_mod_path_from_file_str(fstr: &str, sr: Option<&Path>) -> String {
    let p = Path::new(fstr);
    let rel = sr.and_then(|r| p.strip_prefix(r).ok()).unwrap_or(p).to_string_lossy().replace('\\', "/");
    let trimmed = rel.strip_prefix("src/").unwrap_or(&rel);
    let stem = trimmed.strip_suffix("/mod.rs").or_else(|| trimmed.strip_suffix(".rs")).unwrap_or(trimmed);
    if matches!(stem, "" | "lib" | "main" | "mod") {
        String::new()
    } else {
        stem.replace('/', "::")
    }
}

pub(super) fn extend_method_index(
    ws: &WorkspaceIndex,
    src: &CorpusSourceDb,
    state: &LiveState,
    trait_method_names: &mut HashMap<GlobalDefId, Vec<PoolStr>>,
    inherent_names_by_head: &mut HashMap<ImplTargetKey, Vec<PoolStr>>,
    newly: &[usize],
) {
    let db = RidxV5ItemDb::view(ws, src, state);
    for &r_idx in newly {
        let rdr = &ws.readers()[r_idx];
        let defs = rdr.definitions();
        for (&trait_didx, assocs) in &state.shards[r_idx].trait_assocs {
            let tgid = db.canonical_def(GlobalDefId::new(CrateId(r_idx as u32), trait_didx));
            let entry = trait_method_names.entry(tgid).or_default();
            for &adidx in assocs {
                if let Some(ad) = defs.get(adidx as usize).filter(|_| rdr.fn_sig_for_def(adidx).is_some_and(|s| s.has_self())) {
                    let nm = rdr.get_str(ad.name_offset()).trim_start_matches("r#");
                    if !nm.is_empty() && !entry.iter().any(|p| p.as_str(ws) == nm) {
                        entry.push(PoolStr::from_offset(r_idx, rdr, ad.name_offset(), true));
                    }
                }
            }
        }
        let arena = rdr.ty_arena();
        for iv in rdr.impls_iter() {
            if !iv.is_inherent() || iv.is_negative() || iv.is_reservation() {
                continue;
            }
            let raw_t = db.lower_ty_to_target_key(r_idx, arena, iv.self_ty(), 0);
            let head = match raw_t {
                ImplTargetKey::Adt(d) => ImplTargetKey::Adt(db.canonical_def(d)),
                ImplTargetKey::Dyn(d) => ImplTargetKey::Dyn(db.canonical_def(d)),
                other => other,
            };
            if head == ImplTargetKey::Unknown {
                continue;
            }
            let entry = inherent_names_by_head.entry(head).or_default();
            for adidx in iv.assoc_def_indices() {
                if let Some(ad) = defs.get(adidx as usize).filter(|_| rdr.fn_sig_for_def(adidx).is_some_and(|s| s.has_self())) {
                    let nm = rdr.get_str(ad.name_offset()).trim_start_matches("r#");
                    if !nm.is_empty() && !entry.iter().any(|p| p.as_str(ws) == nm) {
                        entry.push(PoolStr::from_offset(r_idx, rdr, ad.name_offset(), true));
                    }
                }
            }
        }
    }
}

impl super::query_state::LiveEngine {
    /// The unit to answer a query at `off` in: the file's own unit, or another one that compiles the same file when
    /// only that one enables the code at the cursor (`#[cfg(test)]` code goes to the test harness).
    pub(super) fn unit_for_cursor(&self, ws: &WorkspaceIndex, file_unit: usize, canon: &Path, off: u32) -> usize {
        let Some(parsed) = self.parsed.as_ref() else { return file_unit };
        let st = parsed.tree();
        let db = RidxV5ItemDb::view(ws, &self.src, &self.state);
        super::query_cfg::unit_enabling_cursor(&st, off, &db, file_unit, canon).unwrap_or(file_unit)
    }
}
