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
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ridx_format::WorkspaceIndex;
use ridx_syntax::name_res::{CrateDefMap, CrateId, CrateSnapshot, ExtScopeCache, FileId, FileNameRes, GlobalDefId, ImplTargetKey, ModuleTree, OverlaySourceDb};
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
    pub(super) loaded_dirs: HashSet<PathBuf>,
    pub(super) src_loaded: Vec<bool>,
    activated_roots: Vec<bool>,
    path_cache: HashMap<PathBuf, (usize, FileId, PathBuf)>,
    file_to_unit: HashMap<PathBuf, Option<usize>>,
    pub(super) snapshots: HashMap<usize, CrateSnapshot>,
    pub(crate) shared_ext_cache: ExtScopeCache,
    pub(super) clean_env_hashes: HashMap<FileId, u64>,
    dirty_crates: HashMap<usize, CachedDirtyCrate>,
    pub(super) pending_method_shards: Vec<usize>,
    pub inherent_names_by_head: HashMap<ImplTargetKey, Vec<PoolStr>>,
    pub trait_method_names: HashMap<GlobalDefId, Vec<PoolStr>>,
    pub(crate) hidden: super::query_hidden::HiddenCache,
    pub parsed: Option<Arc<CachedParsed>>,
    parsed_cache: HashMap<FileId, Arc<CachedParsed>>,
    pub fn_cache: HashMap<(FileId, u64, u64), CachedFnInfer>,
    fn_fifo: VecDeque<(FileId, u64, u64)>,
    pub infer_ctx: BodyInferCtx,
    pub shared_macro_defs: LocalCstDefs,
    pub shared_file_res: FileNameRes,
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
            path_cache: HashMap::new(),
            file_to_unit: HashMap::new(),
            snapshots: HashMap::new(),
            shared_ext_cache: ExtScopeCache::new(),
            clean_env_hashes: HashMap::new(),
            dirty_crates: HashMap::new(),
            pending_method_shards: Vec::new(),
            inherent_names_by_head: HashMap::new(),
            trait_method_names: HashMap::new(),
            hidden: Default::default(),
            parsed: None,
            parsed_cache: HashMap::new(),
            fn_cache: HashMap::with_capacity(64),
            fn_fifo: VecDeque::with_capacity(64),
            infer_ctx: BodyInferCtx::new(),
            shared_macro_defs: LocalCstDefs::new(),
            shared_file_res: FileNameRes::new(),
        }
    }

    #[inline]
    pub fn locate_and_parse_file(&mut self, ws: &WorkspaceIndex, p: &Path, m: &HashMap<PathBuf, String>, o: Option<&OverlayManager>) -> Option<(usize, FileId, PathBuf)> {
        let (r_idx, fid, canon) = self.resolve_file_entry_with(ws, p, m, o, None, true)?;
        self.parse_resolved_file(ws, r_idx, fid, &canon, m, o, None);
        Some((r_idx, fid, canon))
    }

    pub fn resolve_file_entry(
        &mut self,
        ws: &WorkspaceIndex,
        raw_path: &Path,
        manual: &HashMap<PathBuf, String>,
        overlays: Option<&OverlayManager>,
        clean_hint: Option<&str>,
    ) -> Option<(usize, FileId, PathBuf)> {
        self.resolve_file_entry_with(ws, raw_path, manual, overlays, clean_hint, false)
    }

    fn resolve_file_entry_with(
        &mut self,
        ws: &WorkspaceIndex,
        raw_path: &Path,
        manual: &HashMap<PathBuf, String>,
        overlays: Option<&OverlayManager>,
        clean_hint: Option<&str>,
        allow_ancestor_clean: bool,
    ) -> Option<(usize, FileId, PathBuf)> {
        if let Some(hit) = self.path_cache.get(raw_path) {
            return Some(hit.clone());
        }
        let no_overlays = manual.is_empty() && overlays.is_none_or(|o| o.open_count() == 0);
        let abs = if raw_path.is_absolute() { raw_path.to_path_buf() } else { self.src.ws_root.join(raw_path) };
        let canon =
            if self.src.canon_root == self.src.ws_root && self.state.units_by_file.contains_key(&abs) { abs.clone() } else { abs.canonicalize().unwrap_or_else(|_| abs.clone()) };
        let allow_unindexed = allow_ancestor_clean || (!no_overlays && lookup_live_text(&abs, &canon, manual, overlays).is_some());
        let r_idx = if let Some(&cached) = self.file_to_unit.get(&canon).filter(|c| c.is_some() || !allow_unindexed) {
            cached?
        } else {
            let found = super::query_unit::find_best_unit(ws, &mut self.state, &abs, &canon, allow_unindexed);
            self.file_to_unit.insert(canon.clone(), found);
            found?
        };
        let fid = self.ensure_file_id(&abs, &canon, manual, overlays, clean_hint)?;
        self.path_cache.insert(raw_path.to_path_buf(), (r_idx, fid, canon.clone()));
        Some((r_idx, fid, canon))
    }

    pub fn locate_and_parse_file_with_hint(
        &mut self,
        ws: &WorkspaceIndex,
        raw_path: &Path,
        manual: &HashMap<PathBuf, String>,
        overlays: Option<&OverlayManager>,
        clean_hint: Option<&str>,
        target_offset: Option<u32>,
    ) -> Option<(usize, FileId, PathBuf)> {
        let (r_idx, fid, canon) = self.resolve_file_entry(ws, raw_path, manual, overlays, clean_hint)?;
        self.parse_resolved_file(ws, r_idx, fid, &canon, manual, overlays, target_offset);
        Some((r_idx, fid, canon))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn parse_resolved_file(
        &mut self,
        ws: &WorkspaceIndex,
        r_idx: usize,
        fid: FileId,
        canon: &Path,
        manual: &HashMap<PathBuf, String>,
        overlays: Option<&OverlayManager>,
        target_offset: Option<u32>,
    ) {
        let no_overlays = manual.is_empty() && overlays.is_none_or(|o| o.open_count() == 0);
        let valid_focus = |p: &CachedParsed| p.focused_span.is_none_or(|(s, e)| target_offset.is_some_and(|o| s <= o && o <= e));
        if no_overlays && self.parsed.as_ref().is_some_and(|p| p.file == fid && p.text_hash == u64::MAX && valid_focus(p)) {
            return;
        }
        match (!no_overlays).then(|| lookup_live_text(canon, canon, manual, overlays)).flatten() {
            None => {
                if let Some(cached) = self.parsed_cache.get(&fid).filter(|c| valid_focus(c)).cloned() {
                    self.parsed = Some(cached);
                } else {
                    let had_focused = self.parsed_cache.contains_key(&fid);
                    let cf = &self.src.files[fid.0 as usize];
                    let (owned_src, lines) = (Arc::clone(&cf.text), cf.lines.clone());
                    let skip_ok = !had_focused && ws.readers().get(r_idx).is_some_and(|r| !r.fn_sigs().is_empty()) && owned_src.len() >= 1024;
                    let (st, focused_span) = target_offset.filter(|_| skip_ok).map_or_else(|| (parse(&owned_src), None), |off| ridx_syntax::parse_for_offset(&owned_src, off));
                    let ridx_syntax::SyntaxTree { tokens, nodes, .. } = st;
                    let arc =
                        Arc::new(CachedParsed { file: fid, text_hash: u64::MAX, src: owned_src, lines, bufs: Mutex::new((tokens, nodes)), fn_summaries: Vec::new(), focused_span });
                    if self.parsed_cache.insert(fid, Arc::clone(&arc)).is_some() {
                        self.fn_cache.retain(|&(f, _, _), _| f != fid);
                    }
                    self.parsed = Some(arc);
                }
            }
            Some(owned_src) => {
                let th = fnv1a_str(&owned_src);
                self.clean_env_hashes.entry(fid).or_insert_with(|| {
                    let mut tmp = Vec::new();
                    extract_file_fn_summaries(&parse(&self.src.files[fid.0 as usize].text), &mut tmp)
                });
                if self.parsed.as_ref().is_none_or(|p| p.file != fid || p.text_hash != th) {
                    let (lines, st) = (LineIndex::new(&owned_src), parse(&owned_src));
                    let mut fn_summaries = Vec::new();
                    extract_file_fn_summaries(&st, &mut fn_summaries);
                    let ridx_syntax::SyntaxTree { tokens, nodes, .. } = st;
                    self.parsed = Some(Arc::new(CachedParsed {
                        file: fid,
                        text_hash: th,
                        src: Arc::new(owned_src),
                        lines,
                        bufs: Mutex::new((tokens, nodes)),
                        fn_summaries,
                        focused_span: None,
                    }));
                }
            }
        }
    }

    pub fn ensure_unit_shards(&mut self, ws: &WorkspaceIndex, r_idx: usize) {
        if !self.activated_roots[r_idx] {
            self.activated_roots[r_idx] = true;
            let newly = RidxV5ItemDb::view_mut(ws, &mut self.src, &mut self.state).activate_reachable_shards(r_idx, &mut self.loaded_dirs, &mut self.src_loaded);
            self.pending_method_shards.extend_from_slice(&newly);
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
        self.activate_unit_and_def_map_at(ws, r_idx, fid, canon, manual, overlays, None)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn activate_unit_and_def_map_at(
        &mut self,
        ws: &WorkspaceIndex,
        r_idx: usize,
        fid: FileId,
        canon: &Path,
        manual: &HashMap<PathBuf, String>,
        overlays: Option<&OverlayManager>,
        byte_offset: Option<u32>,
    ) -> Option<u64> {
        let no_overlays = manual.is_empty() && overlays.is_none_or(|o| o.open_count() == 0);
        self.ensure_unit_shards(ws, r_idx);
        self.ensure_snapshot(ws, r_idx, fid);
        if no_overlays {
            self.dirty_crates.remove(&r_idx);
            self.bind_clean_file_if_indexed(ws, r_idx, fid, canon, byte_offset);
            return Some(0);
        }
        let mut dirty_overrides: Vec<(FileId, String)> = Vec::new();
        let mut sig = 0xcbf2_9ce4_8422_2325u64;
        self.collect_crate_dirty_files(r_idx, manual, overlays, &mut dirty_overrides, &mut sig);
        if dirty_overrides.is_empty() {
            self.dirty_crates.remove(&r_idx);
            self.bind_clean_file_if_indexed(ws, r_idx, fid, canon, byte_offset);
            return Some(0);
        }
        if self.dirty_crates.get(&r_idx).is_none_or(|c| c.sig != sig) {
            if self.snapshots.get(&r_idx).is_some_and(CrateSnapshot::is_indexed_only) {
                RidxV5ItemDb::view_mut(ws, &mut self.src, &mut self.state).ensure_crate_sources(r_idx, &mut self.loaded_dirs, &mut self.src_loaded);
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

    fn bind_clean_file_if_indexed(&mut self, ws: &WorkspaceIndex, r_idx: usize, fid: FileId, canon: &Path, byte_offset: Option<u32>) {
        let Some((unbound, true)) = self.snapshots.get(&r_idx).filter(|s| s.is_indexed_only()).map(|s| {
            let u = s.mod_tree.modules_for_file(fid).next().is_none();
            let need_scope = s.mod_tree.modules_for_file(fid).any(|m| {
                let md = &s.mod_tree.modules[m.0 as usize];
                s.clean_def_map.scopes[m.0 as usize].visible_traits_hash == 0 && (!md.is_inline() || byte_offset.is_none_or(|off| md.decl_start <= off && off <= md.decl_end))
            });
            (u, u || need_scope)
        }) else {
            return;
        };
        let mod_path = if unbound { super::query_unit::register_single_file_spans(ws, &self.src, &mut self.state, r_idx, fid, canon) } else { String::new() };
        if let (Some(snap), Some(parsed)) = (self.snapshots.get_mut(&r_idx), self.parsed.as_ref()) {
            let st = parsed.tree();
            snap.put_ext_cache(std::mem::take(&mut self.shared_ext_cache));
            let db = RidxV5ItemDb::view(ws, &self.src, &self.state);
            snap.bind_indexed_file_at(&db, fid, &st, &mod_path, byte_offset);
            self.shared_ext_cache = snap.take_ext_cache();
        }
    }

    pub fn active_def_map(&self, r_idx: usize) -> Option<(&CrateSnapshot, Option<&ModuleTree>, &CrateDefMap)> {
        let snap = self.snapshots.get(&r_idx)?;
        Some(match self.dirty_crates.get(&r_idx) {
            Some(dc) => (snap, dc.new_mod_tree.as_ref(), &dc.def_map),
            None => (snap, None, &snap.clean_def_map),
        })
    }

    pub fn ensure_fn_inferred(&mut self, ws: &WorkspaceIndex, r_idx: usize, fid: FileId, env_salt: u64, byte_offset: u32) -> Option<(FileId, u64, u64)> {
        let parsed = self.parsed.as_ref()?;
        let st = parsed.tree();
        let (fn_node, fn_key, body_hash) = find_enclosing_fn_summary(&st, &parsed.fn_summaries, byte_offset)?;
        let key = (fid, fn_key, body_hash ^ env_salt.rotate_left(13));
        if self.fn_cache.contains_key(&key) {
            return Some(key);
        }
        let (mut file_res, mut local_defs, mut result, mut arena) = if self.fn_cache.len() >= MAX_CACHED_FNS {
            if let Some(old) = self.fn_fifo.pop_front().and_then(|k| self.fn_cache.remove(&k)) {
                (old.file_res, old.local_defs, old.result, old.arena)
            } else {
                (FileNameRes::new(), LocalCstDefs::new(), FnInferResult::new(), FrozenTyArena::with_capacity(64))
            }
        } else {
            let mut fr = FileNameRes::new();
            fr.refs.reserve(32);
            fr.locals.reserve(8);
            let mut res = FnInferResult::new();
            res.expr_tys.reserve(32);
            res.pat_tys.reserve(8);
            res.local_tys.reserve(8);
            res.dot_refs.reserve(8);
            (fr, LocalCstDefs::new(), res, FrozenTyArena::with_capacity(64))
        };
        let snap = self.snapshots.get(&r_idx)?;
        snap.put_ext_cache(std::mem::take(&mut self.shared_ext_cache));
        let (mt_opt, dm) = match self.dirty_crates.get(&r_idx) {
            Some(dc) => (dc.new_mod_tree.as_ref(), &dc.def_map),
            None => (None, &snap.clean_def_map),
        };
        let db = RidxV5ItemDb::view(ws, &self.src, &self.state);
        let has_v5_sigs = ws.readers().get(r_idx).is_some_and(|r| !r.fn_sigs().is_empty());
        file_res.swap_scratch(&mut self.shared_file_res);
        if has_v5_sigs && mt_opt.is_none() && !dm.is_dirty_file(fid) {
            snap.resolve_fn_paths_into(&db, fid, &st, fn_node, &mut file_res);
        } else {
            snap.resolve_dirty_file_paths_into(&db, mt_opt, dm, fid, &st, &mut file_res);
        }
        file_res.swap_scratch(&mut self.shared_file_res);
        self.shared_ext_cache = snap.take_ext_cache();
        local_defs.swap_macro_caches(&mut self.shared_macro_defs);
        if has_v5_sigs {
            local_defs.index_target_fn_with_item_db(&st, Some(dm), Some(&db), fid, fn_node);
        } else {
            local_defs.index_file(&st, Some(dm), fid);
        }
        local_defs.swap_macro_caches(&mut self.shared_macro_defs);
        self.infer_ctx.infer_fn_into(&st, &file_res, &db, &local_defs, fn_node, &mut result);
        arena.copy_from(&self.infer_ctx.table.arena);
        self.fn_fifo.push_back(key);
        self.fn_cache.insert(key, CachedFnInfer { fn_node, file_res, local_defs, result, arena });
        Some(key)
    }
}
