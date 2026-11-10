/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Lazy crate snapshot, dirty-overlay def-map, parsed CST, and `(file, fn_key, body_hash)`
//! single-function inference cache backing [`super::query::LiveDb`].

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use ridx_format::{join_resolving_overlap, WorkspaceIndex};
use ridx_syntax::name_res::{CrateDefMap, CrateId, CrateSnapshot, FileId, FileNameRes, GlobalDefId, ImplTargetKey, ModuleTree, OverlaySourceDb};
use ridx_syntax::ty_infer::{extract_file_fn_summaries, BodyInferCtx, FnInferResult, FrozenTyArena, LocalCstDefs};
use ridx_syntax::{parse, NodeId};

use crate::overlay::OverlayManager;

use super::meta::LineIndex;
pub(crate) use super::query_loc::CachedParsed;
use super::query_loc::{find_enclosing_fn_summary, lookup_live_text};
use super::v5_db::RidxV5ItemDb;
use super::v5_ty_tables::fnv1a_str;

const MAX_CACHED_FNS: usize = 256;

pub(crate) struct CachedFnInfer {
    pub fn_node: NodeId,
    pub file_res: FileNameRes,
    pub local_defs: LocalCstDefs,
    pub result: FnInferResult,
    pub arena: FrozenTyArena,
}

struct CachedDirtyCrate {
    sig: u64,
    new_mod_tree: Option<ModuleTree>,
    def_map: CrateDefMap,
}

pub(crate) struct LiveEngine<'w> {
    pub db: RidxV5ItemDb<'w>,
    loaded_dirs: HashSet<PathBuf>,
    src_loaded: Vec<bool>,
    activated_roots: Vec<bool>,
    path_cache: HashMap<PathBuf, (usize, FileId, PathBuf)>,
    file_to_unit: HashMap<PathBuf, usize>,
    snapshots: HashMap<usize, CrateSnapshot>,
    clean_env_hashes: HashMap<FileId, u64>,
    dirty_crates: HashMap<usize, CachedDirtyCrate>,
    pub inherent_names_by_head: HashMap<ImplTargetKey, Vec<&'w str>>,
    pub trait_method_names: HashMap<GlobalDefId, Vec<&'w str>>,
    pub parsed: Option<CachedParsed>,
    pub fn_cache: HashMap<(FileId, u64, u64), CachedFnInfer>,
    fn_fifo: VecDeque<(FileId, u64, u64)>,
    pub infer_ctx: BodyInferCtx,
}

impl<'w> LiveEngine<'w> {
    pub fn new(ws: &'w WorkspaceIndex) -> Self {
        let n = ws.crate_count();
        Self {
            db: RidxV5ItemDb::new_lazy(ws),
            loaded_dirs: HashSet::new(),
            src_loaded: vec![false; n],
            activated_roots: vec![false; n],
            path_cache: HashMap::new(),
            file_to_unit: HashMap::new(),
            snapshots: HashMap::new(),
            clean_env_hashes: HashMap::new(),
            dirty_crates: HashMap::new(),
            inherent_names_by_head: HashMap::new(),
            trait_method_names: HashMap::new(),
            parsed: None,
            fn_cache: HashMap::with_capacity(64),
            fn_fifo: VecDeque::with_capacity(64),
            infer_ctx: BodyInferCtx::new(),
        }
    }

    pub fn resolve_unit_and_file(&mut self, raw_path: &Path, manual: &HashMap<PathBuf, String>, overlays: Option<&OverlayManager>) -> Option<(usize, FileId, PathBuf)> {
        if let Some(hit) = self.path_cache.get(raw_path) {
            return Some(hit.clone());
        }
        let abs = if raw_path.is_absolute() { raw_path.to_path_buf() } else { self.db.src.ws_root.join(raw_path) };
        let canon = abs.canonicalize().unwrap_or_else(|_| abs.clone());
        let r_idx = if let Some(&cached) = self.file_to_unit.get(&canon) {
            cached
        } else {
            let found = self.find_best_unit(&abs, &canon)?;
            self.file_to_unit.insert(canon.clone(), found);
            found
        };
        if !self.activated_roots[r_idx] {
            self.activated_roots[r_idx] = true;
            let newly = self.db.activate_reachable_shards(r_idx, &mut self.loaded_dirs, &mut self.src_loaded);
            if !newly.is_empty() {
                self.extend_method_index(&newly);
            }
        }
        let fid = self.ensure_file_id(&abs, &canon, manual, overlays)?;
        self.ensure_snapshot(r_idx, fid);
        self.path_cache.insert(raw_path.to_path_buf(), (r_idx, fid, canon.clone()));
        Some((r_idx, fid, canon))
    }

    fn ensure_file_id(&mut self, abs: &Path, canon: &Path, manual: &HashMap<PathBuf, String>, overlays: Option<&OverlayManager>) -> Option<FileId> {
        if let Some(&fid) = self.db.src.by_abs.get(canon).or_else(|| self.db.src.by_abs.get(abs)) {
            return Some(fid);
        }
        let text = lookup_live_text(abs, canon, manual, overlays).or_else(|| fs::read_to_string(canon).ok()).or_else(|| fs::read_to_string(abs).ok())?;
        Some(self.db.src.owned_mut()?.upsert_file_text(canon, text))
    }

    fn find_best_unit(&self, abs: &Path, canon: &Path) -> Option<usize> {
        let mut best: Option<((u8, u8, u32), usize)> = None;
        for idx in 0..self.db.ws.crate_count() {
            let meta = &self.db.metas[idx];
            if meta.is_dependency || meta.target_kind == "custom-build" {
                continue;
            }
            let rdr = &self.db.ws.readers()[idx];
            let root = meta.source_root.as_deref().or_else(|| self.db.ws.workspace_root());
            let mut has_file = false;
            for fid in 0..rdr.header().files_count {
                let fstr = rdr.get_file(fid);
                if fstr.is_empty() {
                    continue;
                }
                let joined = root.map_or_else(|| PathBuf::from(fstr), |r| join_resolving_overlap(r, Path::new(fstr)));
                if joined == abs || joined == canon || joined.canonicalize().is_ok_and(|c| c == canon) {
                    has_file = true;
                    break;
                }
            }
            if has_file {
                let kind_rank = match meta.target_kind.as_str() {
                    "lib" | "rlib" | "proc-macro" | "" if !meta.is_test_harness => 0,
                    _ if !meta.is_test_harness => 1,
                    _ => 2,
                };
                let key = (kind_rank, u8::from(meta.is_test_harness), u32::MAX - rdr.header().refs_count);
                if best.is_none_or(|(bk, _)| key < bk) {
                    best = Some((key, idx));
                }
            }
        }
        if let Some((_, idx)) = best {
            return Some(idx);
        }
        // Unsaved/new file fallback: deepest matching non-dependency `source_root`.
        let mut best_anc: Option<((usize, u8), usize)> = None;
        for (idx, meta) in self.db.metas.iter().enumerate() {
            if meta.is_dependency || meta.target_kind == "custom-build" {
                continue;
            }
            let Some(sr) = meta.source_root.as_deref() else { continue };
            let sr_canon = sr.canonicalize().unwrap_or_else(|_| sr.to_path_buf());
            if canon.starts_with(&sr_canon) || abs.starts_with(sr) {
                let score = (sr_canon.as_os_str().len(), u8::from(!meta.is_test_harness));
                if best_anc.is_none_or(|(bs, _)| score > bs) {
                    best_anc = Some((score, idx));
                }
            }
        }
        best_anc.map(|(_, i)| i)
    }

    fn extend_method_index(&mut self, newly: &[usize]) {
        for &r_idx in newly {
            let rdr = &self.db.ws.readers()[r_idx];
            let defs = rdr.definitions();
            for (&trait_didx, assocs) in &self.db.shards[r_idx].trait_assocs {
                let tgid = self.db.canonical_def(GlobalDefId::new(CrateId(r_idx as u32), trait_didx));
                let entry = self.trait_method_names.entry(tgid).or_default();
                for &adidx in assocs {
                    if let Some(ad) = defs.get(adidx as usize).filter(|_| rdr.fn_sig_for_def(adidx).is_some_and(|s| s.has_self())) {
                        let nm = rdr.get_str(ad.name_offset()).trim_start_matches("r#");
                        if !nm.is_empty() && !entry.contains(&nm) {
                            entry.push(nm);
                        }
                    }
                }
            }
            let arena = rdr.ty_arena();
            for iv in rdr.impls_iter() {
                if !iv.is_inherent() || iv.is_negative() || iv.is_reservation() {
                    continue;
                }
                let raw_t = self.db.lower_ty_to_target_key(r_idx, arena, iv.self_ty(), 0);
                let head = match raw_t {
                    ImplTargetKey::Adt(d) => ImplTargetKey::Adt(self.db.canonical_def(d)),
                    ImplTargetKey::Dyn(d) => ImplTargetKey::Dyn(self.db.canonical_def(d)),
                    other => other,
                };
                if head == ImplTargetKey::Unknown {
                    continue;
                }
                let entry = self.inherent_names_by_head.entry(head).or_default();
                for adidx in iv.assoc_def_indices() {
                    if let Some(ad) = defs.get(adidx as usize).filter(|_| rdr.fn_sig_for_def(adidx).is_some_and(|s| s.has_self())) {
                        let nm = rdr.get_str(ad.name_offset()).trim_start_matches("r#");
                        if !nm.is_empty() && !entry.contains(&nm) {
                            entry.push(nm);
                        }
                    }
                }
            }
        }
    }

    fn ensure_snapshot(&mut self, r_idx: usize, fallback_fid: FileId) {
        if self.snapshots.contains_key(&r_idx) {
            return;
        }
        let meta = &self.db.metas[r_idx];
        let root_fid = meta
            .root_rs_abs
            .as_ref()
            .and_then(|p| self.db.src.lookup_abs_or_rel(meta.source_root.as_deref(), &p.to_string_lossy()))
            .or_else(|| self.db.src.lookup_abs_or_rel(meta.source_root.as_deref(), "src/lib.rs"))
            .or_else(|| self.db.src.lookup_abs_or_rel(meta.source_root.as_deref(), "src/main.rs"))
            .unwrap_or(fallback_fid);
        let snap = CrateSnapshot::build_clean(&self.db, &*self.db.src, CrateId(r_idx as u32), root_fid);
        self.snapshots.insert(r_idx, snap);
    }

    pub fn prepare_file_and_def_map(&mut self, r_idx: usize, fid: FileId, canon: &Path, manual: &HashMap<PathBuf, String>, overlays: Option<&OverlayManager>) -> Option<u64> {
        let no_overlays = manual.is_empty() && overlays.is_none_or(|o| o.open_count() == 0);
        if no_overlays && self.parsed.as_ref().is_some_and(|p| p.file == fid && p.text_hash == u64::MAX) && !self.dirty_crates.contains_key(&r_idx) {
            return Some(0);
        }
        let live_text_opt = if no_overlays { None } else { lookup_live_text(canon, canon, manual, overlays) };
        let clean_text = &self.db.src.files[fid.0 as usize].text;
        self.clean_env_hashes.entry(fid).or_insert_with(|| {
            let cst = parse(clean_text);
            let mut tmp = Vec::new();
            extract_file_fn_summaries(&cst, &mut tmp)
        });
        let (effective_src, th) = match live_text_opt.as_deref() {
            Some(s) => (s, fnv1a_str(s)),
            None => (clean_text.as_str(), u64::MAX),
        };
        if self.parsed.as_ref().is_none_or(|p| p.file != fid || p.text_hash != th) {
            let owned_src = effective_src.to_string();
            let lines = LineIndex::new(&owned_src);
            let st = parse(&owned_src);
            let mut fn_summaries = Vec::new();
            let _ = extract_file_fn_summaries(&st, &mut fn_summaries);
            let ridx_syntax::SyntaxTree { tokens, nodes, .. } = st;
            self.parsed = Some(CachedParsed { file: fid, text_hash: th, src: owned_src, lines, bufs: std::cell::Cell::new((tokens, nodes)), fn_summaries });
        }
        if no_overlays {
            self.dirty_crates.remove(&r_idx);
            return Some(0);
        }

        // Collect all dirty files in `r_idx` whose non-body environment hash differs from clean baseline.
        let mut dirty_overrides: Vec<(FileId, String)> = Vec::new();
        let mut sig = 0xcbf2_9ce4_8422_2325u64;
        self.collect_crate_dirty_files(r_idx, manual, overlays, &mut dirty_overrides, &mut sig);
        if dirty_overrides.is_empty() {
            self.dirty_crates.remove(&r_idx);
            return Some(0);
        }
        if self.dirty_crates.get(&r_idx).is_none_or(|c| c.sig != sig) {
            let snap = self.snapshots.get(&r_idx)?;
            let mut ov_db = OverlaySourceDb::new(&*self.db.src, self.db.src.files.len() as u32);
            let mut dfids = Vec::with_capacity(dirty_overrides.len());
            for (dfid, dtxt) in dirty_overrides {
                ov_db.set_file_override(dfid, dtxt);
                dfids.push(dfid);
            }
            let (new_mod_tree, def_map) = snap.with_dirty_files(&self.db, &ov_db, &dfids, |_| None);
            self.dirty_crates.insert(r_idx, CachedDirtyCrate { sig, new_mod_tree, def_map });
        }
        Some(sig)
    }

    fn collect_crate_dirty_files(&mut self, r_idx: usize, manual: &HashMap<PathBuf, String>, overlays: Option<&OverlayManager>, out: &mut Vec<(FileId, String)>, sig: &mut u64) {
        let sr = self.db.metas[r_idx].source_root.clone().unwrap_or_else(|| self.db.src.ws_root.clone());
        let sr_canon = sr.canonicalize().unwrap_or(sr);
        let mut check_one = |p: &Path, txt: &str, engine: &mut Self| {
            let pc = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
            if !pc.starts_with(&sr_canon) {
                return;
            }
            let dfid = match engine.db.src.by_abs.get(&pc).copied() {
                Some(id) => id,
                None => match engine.db.src.owned_mut() {
                    Some(sm) => sm.upsert_file_text(&pc, String::new()),
                    None => return,
                },
            };
            let clean_txt = &engine.db.src.files[dfid.0 as usize].text;
            if clean_txt == txt {
                return;
            }
            let clean_eh = *engine.clean_env_hashes.entry(dfid).or_insert_with(|| {
                let cst = parse(clean_txt);
                let mut tmp = Vec::new();
                extract_file_fn_summaries(&cst, &mut tmp)
            });
            let dst = parse(txt);
            let mut tmp = Vec::new();
            let live_eh = extract_file_fn_summaries(&dst, &mut tmp);
            if live_eh != clean_eh {
                *sig = (*sig ^ u64::from(dfid.0)).wrapping_mul(0x100_0000_01b3) ^ live_eh;
                out.push((dfid, txt.to_string()));
            }
        };
        for (p, t) in manual {
            check_one(p, t, self);
        }
        if let Some(ov) = overlays {
            for pstr in ov.open_file_paths() {
                if let Some(doc) = ov.get(&pstr).filter(|d| d.is_dirty()) {
                    let p = Path::new(&pstr);
                    if !manual.contains_key(p) {
                        check_one(p, &doc.text, self);
                    }
                }
            }
        }
    }

    pub fn active_def_map(&self, r_idx: usize) -> Option<(&CrateSnapshot, Option<&ModuleTree>, &CrateDefMap)> {
        let snap = self.snapshots.get(&r_idx)?;
        if let Some(dc) = self.dirty_crates.get(&r_idx) {
            Some((snap, dc.new_mod_tree.as_ref(), &dc.def_map))
        } else {
            Some((snap, None, &snap.clean_def_map))
        }
    }

    pub fn ensure_fn_inferred(&mut self, r_idx: usize, fid: FileId, env_salt: u64, byte_offset: u32) -> Option<(FileId, u64, u64)> {
        let parsed = self.parsed.as_ref()?;
        let st = parsed.tree();
        let (fn_node, fn_key, body_hash) = find_enclosing_fn_summary(&st, &parsed.fn_summaries, byte_offset)?;
        let cache_hash = body_hash ^ env_salt.rotate_left(13);
        let key = (fid, fn_key, cache_hash);
        if self.fn_cache.contains_key(&key) {
            return Some(key);
        }
        let snap = self.snapshots.get(&r_idx)?;
        let (mt_opt, dm) = match self.dirty_crates.get(&r_idx) {
            Some(dc) => (dc.new_mod_tree.as_ref(), &dc.def_map),
            None => (None, &snap.clean_def_map),
        };
        let has_v5_sigs = self.db.ws.readers().get(r_idx).is_some_and(|r| !r.fn_sigs().is_empty());
        let mut file_res = FileNameRes::new();
        if has_v5_sigs && mt_opt.is_none() && !dm.is_dirty_file(fid) {
            snap.resolve_fn_paths_into(&self.db, fid, &st, fn_node, &mut file_res);
        } else {
            snap.resolve_dirty_file_paths_into(&self.db, mt_opt, dm, fid, &st, &mut file_res);
        }
        let mut local_defs = LocalCstDefs::new();
        if has_v5_sigs {
            local_defs.index_target_fn_with_item_db(&st, Some(dm), Some(&self.db), fid, fn_node);
        } else {
            local_defs.index_file(&st, Some(dm), fid);
        }
        let mut result = FnInferResult::new();
        self.infer_ctx.infer_fn_into(&st, &file_res, &self.db, &local_defs, fn_node, &mut result);
        let mut arena = FrozenTyArena::with_capacity(self.infer_ctx.table.arena.len());
        arena.copy_from(&self.infer_ctx.table.arena);
        if self.fn_cache.len() >= MAX_CACHED_FNS {
            if let Some(old) = self.fn_fifo.pop_front() {
                self.fn_cache.remove(&old);
            }
        }
        self.fn_fifo.push_back(key);
        self.fn_cache.insert(key, CachedFnInfer { fn_node, file_res, local_defs, result, arena });
        Some(key)
    }
}
