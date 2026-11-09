/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Per-file dirty-function tracker (`DirtyFileTracker`, Checkbox 4.1):
//! on `didChange`, reparses the file, identifies changed `fn`/`const`/`static` bodies relative to
//! the compiled baseline, reuses unchanged dirty-function results across line/offset shifts, and
//! runs local type inference (`BodyInferCtx`) **only** on modified function bodies.

use super::{
    body_ctx::BodyInferCtx,
    db::{EmptyTyDefDb, TyDefDb},
    dirty_fn_scan::{extract_file_fn_summaries, CachedDirtyFn, FnBodySummary},
    local_defs::LocalCstDefs,
};
use crate::{
    name_res::{CrateDefMap, CrateId, ExtScopeCache, FileId, FileNameRes, ItemDb, MemItemDb, ModuleTree, SourceDb},
    parser::ParseScratch,
    tree::SyntaxTree,
};

struct SingleFileSource<'a>(&'a str);

impl SourceDb for SingleFileSource<'_> {
    #[inline]
    fn resolve_path(&self, normalized_path: &str) -> Option<FileId> {
        (normalized_path == "lib.rs").then_some(FileId(0))
    }

    #[inline]
    fn file_path(&self, _file: FileId) -> &str {
        "lib.rs"
    }

    #[inline]
    fn file_text(&self, _file: FileId) -> &str {
        self.0
    }
}

/// Telemetry returned by [`DirtyFileTracker::on_did_change`] / [`DirtyFileTracker::on_did_change_with_db`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DirtyFnUpdateStats {
    /// Total body-bearing `fn` / `const` / `static` items in the live file.
    pub total_fns: u32,
    /// Number of functions whose signature or body differs from the compiled baseline.
    pub dirty_fns_vs_compiled: u32,
    /// Number of dirty functions that required running [`BodyInferCtx::infer_fn_into`] on this edit.
    pub inferred_now: u32,
    /// Number of dirty functions whose cached inference result was shifted and reused in `O(1)`.
    pub reused_cached: u32,
    /// `true` when non-trivia tokens outside function bodies (`use`, `struct`, `fn` sigs) changed vs baseline.
    pub env_changed: bool,
}

/// Zero-allocation incremental dirty-function tracker for a single open document (`Checkbox 4.1`).
#[derive(Debug)]
pub struct DirtyFileTracker {
    has_baseline: bool,
    compiled_env_hash: u64,
    compiled_fns: Vec<FnBodySummary>,
    live_env_hash: u64,
    live_fns: Vec<FnBodySummary>,
    active_dirty: Vec<CachedDirtyFn>,
    scratch_dirty: Vec<CachedDirtyFn>,
    scratch_to_infer: Vec<FnBodySummary>,
    free_pool: Vec<CachedDirtyFn>,
    last_stats: DirtyFnUpdateStats,
    standalone_env_hash: Option<u64>,
    standalone_def_map: Option<CrateDefMap>,
    standalone_item_db: MemItemDb,
    ext_cache: ExtScopeCache,
    parse_scratch: ParseScratch,
    file_res: FileNameRes,
    local_defs: LocalCstDefs,
    infer_ctx: BodyInferCtx,
}

impl Default for DirtyFileTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl DirtyFileTracker {
    pub fn new() -> Self {
        Self {
            has_baseline: false,
            compiled_env_hash: 0,
            compiled_fns: Vec::with_capacity(32),
            live_env_hash: 0,
            live_fns: Vec::with_capacity(32),
            active_dirty: Vec::with_capacity(4),
            scratch_dirty: Vec::with_capacity(4),
            scratch_to_infer: Vec::with_capacity(4),
            free_pool: Vec::with_capacity(4),
            last_stats: DirtyFnUpdateStats::default(),
            standalone_env_hash: None,
            standalone_def_map: None,
            standalone_item_db: MemItemDb::new(),
            ext_cache: ExtScopeCache::new(),
            parse_scratch: ParseScratch::with_capacity(),
            file_res: FileNameRes::new(),
            local_defs: LocalCstDefs::new(),
            infer_ctx: BodyInferCtx::new(),
        }
    }

    /// Resets active dirty state while retaining all underlying arena/vector capacities.
    pub fn clear(&mut self) {
        self.has_baseline = false;
        self.compiled_env_hash = 0;
        self.compiled_fns.clear();
        self.live_env_hash = 0;
        self.live_fns.clear();
        self.free_pool.append(&mut self.active_dirty);
        self.scratch_dirty.clear();
        self.scratch_to_infer.clear();
        self.last_stats = DirtyFnUpdateStats::default();
        self.standalone_env_hash = None;
    }

    /// Establishes or updates the compiled `.ridx` source baseline for this file (`didOpen` or index reload).
    pub fn set_baseline(&mut self, compiled_src: &str) {
        let st = self.parse_scratch.parse_source_file(compiled_src);
        self.compiled_env_hash = extract_file_fn_summaries(&st, &mut self.compiled_fns);
        self.live_env_hash = self.compiled_env_hash;
        self.live_fns.clear();
        self.live_fns.extend_from_slice(&self.compiled_fns);
        self.has_baseline = true;
        self.free_pool.append(&mut self.active_dirty);
        self.last_stats = DirtyFnUpdateStats { total_fns: self.compiled_fns.len() as u32, dirty_fns_vs_compiled: 0, inferred_now: 0, reused_cached: 0, env_changed: false };
        self.standalone_env_hash = None;
        self.parse_scratch.reclaim(st);
    }

    /// Reparses `live_src` on `didChange`, finds function bodies changed relative to baseline,
    /// and infers only the newly modified function bodies using CST-local definitions (`0` warm heap allocations).
    pub fn on_did_change(&mut self, live_src: &str) -> DirtyFnUpdateStats {
        let st = self.parse_scratch.parse_source_file(live_src);
        let live_env_hash = self.classify_dirty_fns(&st, false);
        let env_changed = !self.has_baseline || live_env_hash != self.compiled_env_hash;
        let mut inferred_now = 0u32;

        if !self.scratch_to_infer.is_empty() {
            if self.standalone_env_hash != Some(live_env_hash) || self.standalone_def_map.is_none() {
                let src_db = SingleFileSource(live_src);
                let mt = ModuleTree::build_cached(&self.standalone_item_db, &src_db, CrateId(0), FileId(0), |f| (f == FileId(0)).then_some(&st));
                let dm = CrateDefMap::build_cached(&self.standalone_item_db, &src_db, &mt, &[FileId(0)], |f| (f == FileId(0)).then_some(&st));
                self.standalone_def_map = Some(dm);
                self.standalone_env_hash = Some(live_env_hash);
            }
            let dm = self.standalone_def_map.as_ref().expect("standalone_def_map populated");
            self.local_defs.index_file(&st, Some(dm), FileId(0));
            dm.resolve_file_paths_into(&self.standalone_item_db, &dm.mod_tree, &mut self.ext_cache, FileId(0), &st, &mut self.file_res);
            for i in 0..self.scratch_to_infer.len() {
                let s = self.scratch_to_infer[i];
                let mut entry = self.free_pool.pop().unwrap_or_default();
                entry.init_from_summary(&s, live_env_hash, false);
                self.infer_ctx.infer_fn_into(&st, &self.file_res, &EmptyTyDefDb, &self.local_defs, s.fn_node, &mut entry.infer);
                entry.types.copy_from(&self.infer_ctx.table.arena);
                self.scratch_dirty.push(entry);
                inferred_now += 1;
            }
        }

        let stats = self.finish_did_change(inferred_now, env_changed);
        self.parse_scratch.reclaim(st);
        stats
    }

    /// Reparses `live_src` on `didChange` and infers only newly modified function bodies against `item_db` + `ty_db`.
    pub fn on_did_change_with_db(&mut self, live_src: &str, file: FileId, def_map: &CrateDefMap, item_db: &dyn ItemDb, ty_db: &dyn TyDefDb) -> DirtyFnUpdateStats {
        let st = self.parse_scratch.parse_source_file(live_src);
        let live_env_hash = self.classify_dirty_fns(&st, true);
        let env_changed = !self.has_baseline || live_env_hash != self.compiled_env_hash;
        let mut inferred_now = 0u32;

        if !self.scratch_to_infer.is_empty() {
            if self.scratch_to_infer.len() == 1 {
                let s = self.scratch_to_infer[0];
                self.local_defs.index_target_fn_with_item_db(&st, Some(def_map), Some(item_db), file, s.fn_node);
            } else {
                self.local_defs.index_file_with_item_db(&st, Some(def_map), Some(item_db), file);
            }
            for i in 0..self.scratch_to_infer.len() {
                let s = self.scratch_to_infer[i];
                def_map.resolve_fn_paths_into(item_db, &def_map.mod_tree, &mut self.ext_cache, file, &st, s.fn_node, &mut self.file_res);
                let mut entry = self.free_pool.pop().unwrap_or_default();
                entry.init_from_summary(&s, live_env_hash, true);
                self.infer_ctx.infer_fn_into(&st, &self.file_res, ty_db, &self.local_defs, s.fn_node, &mut entry.infer);
                entry.types.copy_from(&self.infer_ctx.table.arena);
                self.scratch_dirty.push(entry);
                inferred_now += 1;
            }
        }

        let stats = self.finish_did_change(inferred_now, env_changed);
        self.parse_scratch.reclaim(st);
        stats
    }

    fn classify_dirty_fns(&mut self, st: &SyntaxTree<'_>, has_full_db: bool) -> u64 {
        let live_env_hash = extract_file_fn_summaries(st, &mut self.live_fns);
        self.live_env_hash = live_env_hash;
        self.scratch_dirty.clear();
        self.scratch_to_infer.clear();
        let mut dirty_vs_compiled = 0u32;
        let mut reused_cached = 0u32;

        for i in 0..self.live_fns.len() {
            let s = self.live_fns[i];
            let is_clean = self.has_baseline && self.compiled_fns.iter().any(|c| c.fn_key == s.fn_key && c.sig_hash == s.sig_hash && c.body_hash == s.body_hash);
            if is_clean {
                continue;
            }
            dirty_vs_compiled += 1;
            if let Some(pos) = self
                .active_dirty
                .iter()
                .position(|c| c.fn_key == s.fn_key && c.sig_hash == s.sig_hash && c.body_hash == s.body_hash && c.env_hash == live_env_hash && (!has_full_db || c.has_full_db))
            {
                let mut cached = self.active_dirty.swap_remove(pos);
                cached.shift_to(&s);
                self.scratch_dirty.push(cached);
                reused_cached += 1;
            } else {
                self.scratch_to_infer.push(s);
            }
        }
        self.free_pool.append(&mut self.active_dirty);
        self.last_stats.dirty_fns_vs_compiled = dirty_vs_compiled;
        self.last_stats.reused_cached = reused_cached;
        live_env_hash
    }

    fn finish_did_change(&mut self, inferred_now: u32, env_changed: bool) -> DirtyFnUpdateStats {
        self.scratch_dirty.sort_unstable_by_key(|c| (c.fn_start, c.fn_end));
        core::mem::swap(&mut self.active_dirty, &mut self.scratch_dirty);
        let stats = DirtyFnUpdateStats {
            total_fns: self.live_fns.len() as u32,
            dirty_fns_vs_compiled: self.last_stats.dirty_fns_vs_compiled,
            inferred_now,
            reused_cached: self.last_stats.reused_cached,
            env_changed,
        };
        self.last_stats = stats;
        stats
    }

    #[inline]
    pub fn last_stats(&self) -> DirtyFnUpdateStats {
        self.last_stats
    }

    #[inline]
    pub fn has_baseline(&self) -> bool {
        self.has_baseline
    }

    #[inline]
    pub fn live_fns(&self) -> &[FnBodySummary] {
        &self.live_fns
    }

    #[inline]
    pub fn dirty_fns(&self) -> &[CachedDirtyFn] {
        &self.active_dirty
    }

    /// Returns the innermost [`CachedDirtyFn`] whose span covers `byte_offset`, if that function is dirty.
    pub fn dirty_fn_at_offset(&self, byte_offset: u32) -> Option<&CachedDirtyFn> {
        let mut best: Option<&CachedDirtyFn> = None;
        for c in &self.active_dirty {
            if c.fn_start <= byte_offset && byte_offset <= c.fn_end {
                let len = c.fn_end - c.fn_start;
                if best.is_none_or(|b| len <= (b.fn_end - b.fn_start)) {
                    best = Some(c);
                }
            }
        }
        best
    }

    #[inline]
    pub fn is_offset_in_dirty_fn(&self, byte_offset: u32) -> bool {
        self.dirty_fn_at_offset(byte_offset).is_some()
    }

    /// Executes `f` with a zero-allocation [`SyntaxTree`] view over `live_src` backed by the most recent parse buffers.
    #[inline]
    pub fn with_live_tree<R>(&mut self, live_src: &str, f: impl FnOnce(&Self, &SyntaxTree<'_>) -> R) -> R {
        let st = SyntaxTree { src: live_src, tokens: core::mem::take(&mut self.parse_scratch.tokens), nodes: core::mem::take(&mut self.parse_scratch.nodes) };
        let res = f(self, &st);
        self.parse_scratch.reclaim(st);
        res
    }

    #[inline]
    pub fn file_res(&self) -> &FileNameRes {
        &self.file_res
    }

    #[inline]
    pub fn local_defs(&self) -> &LocalCstDefs {
        &self.local_defs
    }

    /// Total heap memory capacity (in bytes) retained by cached dirty functions and reusable scratch buffers.
    pub fn heap_bytes(&self) -> usize {
        let active_b: usize = self.active_dirty.iter().map(CachedDirtyFn::heap_bytes).sum();
        let pool_b: usize = self.free_pool.iter().map(CachedDirtyFn::heap_bytes).sum();
        active_b
            + pool_b
            + self.compiled_fns.capacity() * core::mem::size_of::<FnBodySummary>()
            + self.live_fns.capacity() * core::mem::size_of::<FnBodySummary>()
            + self.active_dirty.capacity() * core::mem::size_of::<CachedDirtyFn>()
            + self.free_pool.capacity() * core::mem::size_of::<CachedDirtyFn>()
    }
}
