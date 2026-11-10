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
use std::sync::Mutex;

use ridx_format::WorkspaceIndex;
use ridx_syntax::name_res::{CrateDefMap, CrateId, CrateSnapshot, FileId, FileNameRes, GlobalDefId, ImplTargetKey, ModuleTree, OverlaySourceDb};
use ridx_syntax::ty_infer::{extract_file_fn_summaries, BodyInferCtx, FnInferResult, FrozenTyArena, LocalCstDefs};
use ridx_syntax::{parse, NodeId};

use crate::overlay::OverlayManager;

use super::meta::{CorpusSourceDb, LineIndex};
pub(crate) use super::query_loc::CachedParsed;
use super::query_loc::{find_enclosing_fn_summary, lookup_live_text};
use super::v5_db::{LiveState, RidxV5ItemDb};
use super::v5_lazy::PoolStr;
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

pub(crate) struct LiveEngine {
    pub src: CorpusSourceDb,
    pub state: LiveState,
    loaded_dirs: HashSet<PathBuf>,
    src_loaded: Vec<bool>,
    activated_roots: Vec<bool>,
    umbrella_queue: Option<Vec<usize>>,
    umbrella_cursor: usize,
    path_cache: HashMap<PathBuf, (usize, FileId, PathBuf)>,
    file_to_unit: HashMap<PathBuf, usize>,
    snapshots: HashMap<usize, CrateSnapshot>,
    clean_env_hashes: HashMap<FileId, u64>,
    dirty_crates: HashMap<usize, CachedDirtyCrate>,
    pending_method_shards: Vec<usize>,
    pub inherent_names_by_head: HashMap<ImplTargetKey, Vec<PoolStr>>,
    pub trait_method_names: HashMap<GlobalDefId, Vec<PoolStr>>,
    pub parsed: Option<CachedParsed>,
    pub fn_cache: HashMap<(FileId, u64, u64), CachedFnInfer>,
    fn_fifo: VecDeque<(FileId, u64, u64)>,
    pub infer_ctx: BodyInferCtx,
}

impl LiveEngine {
    pub fn new(ws: &WorkspaceIndex) -> Self {
        let n = ws.crate_count();
        let (src, state) = LiveState::new_lazy(ws);
        Self {
            src,
            state,
            loaded_dirs: HashSet::new(),
            src_loaded: vec![false; n],
            activated_roots: vec![false; n],
            umbrella_queue: None,
            umbrella_cursor: 0,
            path_cache: HashMap::new(),
            file_to_unit: HashMap::new(),
            snapshots: HashMap::new(),
            clean_env_hashes: HashMap::new(),
            dirty_crates: HashMap::new(),
            pending_method_shards: Vec::new(),
            inherent_names_by_head: HashMap::new(),
            trait_method_names: HashMap::new(),
            parsed: None,
            fn_cache: HashMap::with_capacity(64),
            fn_fifo: VecDeque::with_capacity(64),
            infer_ctx: BodyInferCtx::new(),
        }
    }

    pub fn locate_and_parse_file(
        &mut self,
        ws: &WorkspaceIndex,
        raw_path: &Path,
        manual: &HashMap<PathBuf, String>,
        overlays: Option<&OverlayManager>,
    ) -> Option<(usize, FileId, PathBuf)> {
        let (r_idx, fid, canon) = if let Some(hit) = self.path_cache.get(raw_path) {
            hit.clone()
        } else {
            let abs = if raw_path.is_absolute() { raw_path.to_path_buf() } else { self.src.ws_root.join(raw_path) };
            let canon = abs.canonicalize().unwrap_or_else(|_| abs.clone());
            let r_idx = if let Some(&cached) = self.file_to_unit.get(&canon) {
                cached
            } else {
                let found = super::query_unit::find_best_unit(ws, &mut self.state, &abs, &canon)?;
                self.file_to_unit.insert(canon.clone(), found);
                found
            };
            let fid = self.ensure_file_id(&abs, &canon, manual, overlays)?;
            self.path_cache.insert(raw_path.to_path_buf(), (r_idx, fid, canon.clone()));
            (r_idx, fid, canon)
        };
        let no_overlays = manual.is_empty() && overlays.is_none_or(|o| o.open_count() == 0);
        if !(no_overlays && self.parsed.as_ref().is_some_and(|p| p.file == fid && p.text_hash == u64::MAX)) {
            let live_text_opt = if no_overlays { None } else { lookup_live_text(&canon, &canon, manual, overlays) };
            let clean_text = &self.src.files[fid.0 as usize].text;
            let (effective_src, th) = match live_text_opt.as_deref() {
                Some(s) => (s, fnv1a_str(s)),
                None => (clean_text.as_str(), u64::MAX),
            };
            if th != u64::MAX && !self.clean_env_hashes.contains_key(&fid) {
                let cst = parse(clean_text);
                let mut tmp = Vec::new();
                self.clean_env_hashes.insert(fid, extract_file_fn_summaries(&cst, &mut tmp));
            }
            if self.parsed.as_ref().is_none_or(|p| p.file != fid || p.text_hash != th) {
                let owned_src = effective_src.to_string();
                let lines = LineIndex::new(&owned_src);
                let st = parse(&owned_src);
                let mut fn_summaries = Vec::new();
                let env_h = extract_file_fn_summaries(&st, &mut fn_summaries);
                if th == u64::MAX {
                    self.clean_env_hashes.entry(fid).or_insert(env_h);
                }
                let ridx_syntax::SyntaxTree { tokens, nodes, .. } = st;
                self.parsed = Some(CachedParsed { file: fid, text_hash: th, src: owned_src, lines, bufs: Mutex::new((tokens, nodes)), fn_summaries });
            }
        }
        Some((r_idx, fid, canon))
    }

    pub fn ensure_unit_shards(&mut self, ws: &WorkspaceIndex, r_idx: usize) {
        let mut loaded_count = 0usize;
        if !self.activated_roots[r_idx] {
            self.activated_roots[r_idx] = true;
            let newly = {
                let mut db = RidxV5ItemDb::view_mut(ws, &mut self.src, &mut self.state);
                db.activate_reachable_shards(r_idx, &mut self.loaded_dirs, &mut self.src_loaded)
            };
            loaded_count = newly.len();
            if !newly.is_empty() {
                self.pending_method_shards.extend_from_slice(&newly);
            }
        }
        if loaded_count < 3 && self.umbrella_queue.as_ref().is_none_or(|q| self.umbrella_cursor < q.len()) {
            let newly = {
                let mut db = RidxV5ItemDb::view_mut(ws, &mut self.src, &mut self.state);
                db.prewarm_umbrella_step(&mut self.umbrella_queue, &mut self.umbrella_cursor, &mut self.loaded_dirs, &mut self.src_loaded)
            };
            if !newly.is_empty() {
                self.pending_method_shards.extend_from_slice(&newly);
            }
        }
    }

    pub fn activate_unit_and_def_map(
        &mut self,
        ws: &WorkspaceIndex,
        r_idx: usize,
        fid: FileId,
        canon: &Path,
        manual: &HashMap<PathBuf, String>,
        overlays: Option<&OverlayManager>,
    ) -> Option<u64> {
        let no_overlays = manual.is_empty() && overlays.is_none_or(|o| o.open_count() == 0);
        self.ensure_unit_shards(ws, r_idx);
        self.ensure_snapshot(ws, r_idx, fid);
        if no_overlays {
            self.dirty_crates.remove(&r_idx);
            self.bind_clean_file_if_indexed(ws, r_idx, fid, canon);
            return Some(0);
        }

        let mut dirty_overrides: Vec<(FileId, String)> = Vec::new();
        let mut sig = 0xcbf2_9ce4_8422_2325u64;
        self.collect_crate_dirty_files(r_idx, manual, overlays, &mut dirty_overrides, &mut sig);
        if dirty_overrides.is_empty() {
            self.dirty_crates.remove(&r_idx);
            self.bind_clean_file_if_indexed(ws, r_idx, fid, canon);
            return Some(0);
        }
        if self.dirty_crates.get(&r_idx).is_none_or(|c| c.sig != sig) {
            if self.snapshots.get(&r_idx).is_some_and(CrateSnapshot::is_indexed_only) {
                {
                    let mut db = RidxV5ItemDb::view_mut(ws, &mut self.src, &mut self.state);
                    db.ensure_crate_sources(r_idx, &mut self.loaded_dirs, &mut self.src_loaded);
                }
                let root_fid = self.resolve_root_fid(r_idx, fid);
                let db = RidxV5ItemDb::view(ws, &self.src, &self.state);
                self.snapshots.insert(r_idx, CrateSnapshot::build_clean(&db, &self.src, CrateId(r_idx as u32), root_fid));
            }
            let snap = self.snapshots.get_mut(&r_idx)?;
            let mut ov_db = OverlaySourceDb::new(&self.src, self.src.files.len() as u32);
            let mut dfids = Vec::with_capacity(dirty_overrides.len());
            for (dfid, dtxt) in dirty_overrides {
                ov_db.set_file_override(dfid, dtxt);
                dfids.push(dfid);
            }
            let db = RidxV5ItemDb::view(ws, &self.src, &self.state);
            snap.ensure_dirty_base(&db, &self.src);
            let (new_mod_tree, def_map) = snap.with_dirty_files(&db, &ov_db, &dfids, |_| None);
            self.dirty_crates.insert(r_idx, CachedDirtyCrate { sig, new_mod_tree, def_map });
        }
        Some(sig)
    }

    fn bind_clean_file_if_indexed(&mut self, ws: &WorkspaceIndex, r_idx: usize, fid: FileId, canon: &Path) {
        if self.snapshots.get(&r_idx).is_none_or(|s| !s.is_indexed_only() || s.mod_tree.modules_for_file(fid).next().is_some()) {
            return;
        }
        let mod_path = super::query_unit::register_single_file_spans(ws, &self.src, &mut self.state, r_idx, fid, canon);
        if let (Some(snap), Some(parsed)) = (self.snapshots.get_mut(&r_idx), self.parsed.as_ref()) {
            let st = parsed.tree();
            let db = RidxV5ItemDb::view(ws, &self.src, &self.state);
            snap.bind_indexed_file(&db, fid, &st, &mod_path);
        }
    }

    fn resolve_root_fid(&self, r_idx: usize, fallback_fid: FileId) -> FileId {
        let meta = &self.state.metas[r_idx];
        meta.root_rs_abs
            .as_ref()
            .and_then(|p| self.src.lookup_abs_or_rel(meta.source_root.as_deref(), &p.to_string_lossy()))
            .or_else(|| self.src.lookup_abs_or_rel(meta.source_root.as_deref(), "src/lib.rs"))
            .or_else(|| self.src.lookup_abs_or_rel(meta.source_root.as_deref(), "src/main.rs"))
            .unwrap_or(fallback_fid)
    }

    fn ensure_snapshot(&mut self, ws: &WorkspaceIndex, r_idx: usize, fallback_fid: FileId) {
        if self.snapshots.contains_key(&r_idx) {
            return;
        }
        let krate = CrateId(r_idx as u32);
        let has_v5 = !ws.readers()[r_idx].fn_sigs().is_empty();
        if !has_v5 {
            let mut db_mut = RidxV5ItemDb::view_mut(ws, &mut self.src, &mut self.state);
            db_mut.ensure_crate_sources(r_idx, &mut self.loaded_dirs, &mut self.src_loaded);
        }
        let root_fid = self.resolve_root_fid(r_idx, fallback_fid);
        let db = RidxV5ItemDb::view(ws, &self.src, &self.state);
        let snap = if has_v5 {
            CrateSnapshot::build_from_index(&db, krate).unwrap_or_else(|| CrateSnapshot::build_clean_for_query(&db, &self.src, krate, root_fid))
        } else {
            CrateSnapshot::build_clean_for_query(&db, &self.src, krate, root_fid)
        };
        self.snapshots.insert(r_idx, snap);
    }

    fn ensure_file_id(&mut self, abs: &Path, canon: &Path, manual: &HashMap<PathBuf, String>, overlays: Option<&OverlayManager>) -> Option<FileId> {
        if let Some(&fid) = self.src.by_abs.get(canon).or_else(|| self.src.by_abs.get(abs)) {
            return Some(fid);
        }
        let disk_text = fs::read_to_string(canon).ok().or_else(|| fs::read_to_string(abs).ok());
        let clean_text = match disk_text {
            Some(t) => t,
            None if lookup_live_text(abs, canon, manual, overlays).is_some() => String::new(),
            None => return None,
        };
        Some(self.src.upsert_file_text(canon, clean_text))
    }

    pub fn ensure_method_index(&mut self, ws: &WorkspaceIndex) {
        if self.pending_method_shards.is_empty() {
            return;
        }
        let newly = std::mem::take(&mut self.pending_method_shards);
        super::query_unit::extend_method_index(ws, &self.src, &self.state, &mut self.trait_method_names, &mut self.inherent_names_by_head, &newly);
    }

    fn collect_crate_dirty_files(&mut self, r_idx: usize, manual: &HashMap<PathBuf, String>, overlays: Option<&OverlayManager>, out: &mut Vec<(FileId, String)>, sig: &mut u64) {
        let sr = self.state.metas[r_idx].source_root.clone().unwrap_or_else(|| self.src.ws_root.clone());
        let sr_canon = sr.canonicalize().unwrap_or(sr);
        let mut check_one = |p: &Path, txt: &str, engine: &mut Self| {
            let pc = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
            if !pc.starts_with(&sr_canon) {
                return;
            }
            let dfid = match engine.src.by_abs.get(&pc).copied() {
                Some(id) => id,
                None => engine.src.upsert_file_text(&pc, String::new()),
            };
            let clean_txt = &engine.src.files[dfid.0 as usize].text;
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

    pub fn ensure_fn_inferred(&mut self, ws: &WorkspaceIndex, r_idx: usize, fid: FileId, env_salt: u64, byte_offset: u32) -> Option<(FileId, u64, u64)> {
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
        let db = RidxV5ItemDb::view(ws, &self.src, &self.state);
        let has_v5_sigs = ws.readers().get(r_idx).is_some_and(|r| !r.fn_sigs().is_empty());
        let mut file_res = FileNameRes::new();
        if has_v5_sigs && mt_opt.is_none() && !dm.is_dirty_file(fid) {
            snap.resolve_fn_paths_into(&db, fid, &st, fn_node, &mut file_res);
        } else {
            snap.resolve_dirty_file_paths_into(&db, mt_opt, dm, fid, &st, &mut file_res);
        }
        let mut local_defs = LocalCstDefs::new();
        if has_v5_sigs {
            local_defs.index_target_fn_with_item_db(&st, Some(dm), Some(&db), fid, fn_node);
        } else {
            local_defs.index_file(&st, Some(dm), fid);
        }
        let mut result = FnInferResult::new();
        self.infer_ctx.infer_fn_into(&st, &file_res, &db, &local_defs, fn_node, &mut result);
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
