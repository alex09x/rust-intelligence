/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! On-demand per-crate shard activation and lazy workspace source loading for [`RidxV5ItemDb`].

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use ridx_format::{join_resolving_overlap, DefKind, ItemVis, TyDefId, TyKindView, UseOwner, WorkspaceIndex, USE_FLAG_REEXPORT};
use ridx_syntax::name_res::{join_norm_path, FileId, GlobalDefId};

use super::meta::{collect_rs_files, load_reader_meta_at, CorpusFile, CorpusSourceDb, LineIndex};
use super::v5_db::{LiveState, RidxV5ItemDb};
pub use super::v5_refs::{AtomicDefCell, LiveStateRef, PoolStr, SourceStore};
use super::v5_shard::{build_shard_tables, ShardTables};
use super::v5_ty_tables::V5TyTables;

impl CorpusSourceDb {
    pub fn empty(ws_root: &Path) -> Self {
        let canon_root = ws_root.canonicalize().unwrap_or_else(|_| ws_root.to_path_buf());
        Self { ws_root: ws_root.to_path_buf(), canon_root, files: Vec::new(), by_rel: HashMap::new(), by_abs: HashMap::new() }
    }

    pub fn upsert_file_text(&mut self, abs_path: &Path, text: String) -> FileId {
        if let Some(&fid) = self.by_abs.get(abs_path) {
            let cf = &mut self.files[fid.0 as usize];
            if *cf.text != text {
                cf.lines = LineIndex::new(&text);
                cf.text = std::sync::Arc::new(text);
            }
            return fid;
        }
        let canon = abs_path.canonicalize().unwrap_or_else(|_| abs_path.to_path_buf());
        self.upsert_canonical_file(abs_path, &canon, text)
    }

    pub fn upsert_canonical_file(&mut self, abs_path: &Path, canon: &Path, text: String) -> FileId {
        if let Some(&fid) = self.by_abs.get(canon).or_else(|| (abs_path != canon).then(|| self.by_abs.get(abs_path)).flatten()) {
            if abs_path != canon {
                self.by_abs.entry(abs_path.to_path_buf()).or_insert(fid);
            }
            let cf = &mut self.files[fid.0 as usize];
            if *cf.text != text {
                cf.lines = LineIndex::new(&text);
                cf.text = std::sync::Arc::new(text);
            }
            return fid;
        }
        let rel_raw = abs_path.strip_prefix(&self.ws_root).or_else(|_| canon.strip_prefix(&self.canon_root)).unwrap_or(abs_path).to_string_lossy();
        let rel_path = join_norm_path("", &rel_raw);
        let id = FileId(self.files.len() as u32);
        let lines = LineIndex::new(&text);
        self.by_rel.entry(rel_path.clone()).or_insert(id);
        self.by_abs.insert(canon.to_path_buf(), id);
        if abs_path != canon || self.ws_root != self.canon_root {
            if abs_path != canon {
                self.by_abs.insert(abs_path.to_path_buf(), id);
            }
            if let Ok(crel) = canon.strip_prefix(&self.canon_root) {
                self.by_rel.entry(join_norm_path("", &crel.to_string_lossy())).or_insert(id);
            }
        }
        self.by_rel.entry(join_norm_path("", &canon.to_string_lossy())).or_insert(id);
        self.files.push(CorpusFile { rel_path, text: std::sync::Arc::new(text), lines });
        id
    }

    pub fn load_dir_once(&mut self, dir: &Path, loaded_dirs: &mut HashSet<PathBuf>) -> usize {
        let canon = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
        if !loaded_dirs.insert(canon) || !dir.is_dir() {
            return 0;
        }
        let mut raw_paths = Vec::new();
        let mut ancestors = Vec::new();
        collect_rs_files(dir, &mut ancestors, &mut raw_paths);
        raw_paths.sort();
        let start_len = self.files.len();
        for abs in raw_paths {
            if !self.by_abs.contains_key(&abs) {
                if let Ok(text) = fs::read_to_string(&abs) {
                    let _ = self.upsert_file_text(&abs, text);
                }
            }
        }
        self.files.len() - start_len
    }
}

impl LiveState {
    /// Creates an unpopulated `LiveState` + empty `CorpusSourceDb` without parsing external `.meta.json`s upfront.
    pub fn new_lazy(ws: &WorkspaceIndex) -> (CorpusSourceDb, Self) {
        let ws_root = ws
            .workspace_root()
            .map(Path::to_path_buf)
            .or_else(|| (0..ws.crate_count()).filter(|&i| !ws.is_reader_dependency(i)).find_map(|i| ws.source_root_at(i).map(Path::to_path_buf)))
            .unwrap_or_else(|| PathBuf::from("."));
        let n = ws.crate_count();
        let has_core = ws.readers().iter().any(|r| r.crate_name() == "core");
        let metas: Vec<_> = (0..n).map(|idx| load_reader_meta_at(ws, idx, has_core, false)).collect();
        let meta_loaded: Vec<bool> = (0..n).map(|idx| ws.meta_path_at(idx).is_none()).collect();
        let mut sysroot_by_name = HashMap::new();
        let mut units_by_file: HashMap<PathBuf, Vec<(usize, u32)>> = HashMap::new();
        for (idx, meta) in metas.iter().enumerate() {
            if matches!(meta.crate_name.as_str(), "core" | "alloc" | "std" | "proc_macro") {
                sysroot_by_name.entry(meta.crate_name.clone()).or_insert(idx);
            } else if !meta.is_sysroot && (!meta.is_dependency || meta.source_root.as_deref().is_some_and(|sr| sr.starts_with(&ws_root))) {
                let rdr = &ws.readers()[idx];
                let sr = meta.source_root.as_deref().unwrap_or(ws_root.as_path());
                for fid in 0..rdr.header().files_count {
                    let fstr = rdr.get_file(fid);
                    if !fstr.is_empty() {
                        let joined = join_resolving_overlap(sr, Path::new(fstr));
                        units_by_file.entry(joined).or_default().push((idx, fid));
                    }
                }
            }
        }
        let mut ty_tables = V5TyTables::default();
        ty_tables.raw_ty_def_gids.resize_with(n, Vec::new);
        ty_tables.ty_def_gids.resize_with(n, Vec::new);
        let state = Self {
            bench_mode: false,
            metas,
            meta_loaded,
            shards: (0..n).map(|_| ShardTables::default()).collect(),
            shard_loaded: vec![false; n],
            shard_expanded_depth: vec![u8::MAX; n],
            units_by_file,
            sysroot_by_name,
            impls_arena: Vec::new(),
            impls_by_adt: HashMap::new(),
            impls_by_other: HashMap::new(),
            trait_supertraits: HashMap::new(),
            impl_assoc_parent: HashMap::new(),
            impl_self_target: HashMap::new(),
            valid_alias_leaf_pairs: HashSet::new(),
            ty_tables,
            active_owner_def: AtomicDefCell::new(GlobalDefId::NONE),
        };
        (CorpusSourceDb::empty(&ws_root), state)
    }
}

impl RidxV5ItemDb<'_> {
    #[inline]
    fn ensure_meta_at(&mut self, idx: usize) {
        self.state.as_mut().ensure_meta(self.ws, idx);
    }

    pub(crate) fn ensure_crate_sources(&mut self, r_idx: usize, loaded_dirs: &mut HashSet<PathBuf>, src_loaded: &mut [bool]) {
        if src_loaded.get(r_idx).copied().unwrap_or(true) {
            return;
        }
        self.ensure_meta_at(r_idx);
        if self.metas[r_idx].is_dependency {
            return;
        }
        src_loaded[r_idx] = true;
        let ws = self.ws;
        let sr = self.metas[r_idx].source_root.clone().unwrap_or_else(|| self.src.ws_root.clone());
        let out_dir_opt = self.metas[r_idx].env.get("OUT_DIR").map(PathBuf::from);
        let root_rs = self.metas[r_idx].root_rs_abs.clone();
        if let Some(src_mut) = self.src.owned_mut() {
            let before = src_mut.files.len();
            let rdr = &ws.readers()[r_idx];
            let fcount = rdr.header().files_count;
            for fid in 0..fcount {
                let fstr = rdr.get_file(fid);
                if !fstr.is_empty() {
                    let abs = join_resolving_overlap(&sr, Path::new(fstr));
                    if !src_mut.by_abs.contains_key(&abs) {
                        if let Ok(text) = fs::read_to_string(&abs) {
                            let _ = src_mut.upsert_file_text(&abs, text);
                        }
                    }
                }
            }
            if fcount == 0
                || (fcount == 1
                    && src_mut.files[before..].iter().any(|f| f.text.contains("mod ") || f.text.contains("mod\n") || f.text.contains("mod\t") || f.text.contains("include")))
            {
                let scan_dir = root_rs.as_deref().and_then(Path::parent).unwrap_or(sr.as_path());
                src_mut.load_dir_once(scan_dir, loaded_dirs);
            }
            if let Some(od) = &out_dir_opt {
                src_mut.load_dir_once(od, loaded_dirs);
            }
            if src_mut.files[before..].iter().any(|f| f.text.contains("../") || f.text.contains("..\\")) {
                src_mut.load_dir_once(&sr, loaded_dirs);
            }
        }
        if self.shard_loaded[r_idx] {
            self.backfill_shard_file_spans(r_idx);
        }
    }

    fn backfill_shard_file_spans(&mut self, r_idx: usize) {
        let reader = &self.ws.readers()[r_idx];
        let sr = self.metas[r_idx].source_root.as_deref();
        let files_count = reader.header().files_count as usize;
        let mut file_map = vec![FileId::NONE; files_count];
        for (fid, slot) in file_map.iter_mut().enumerate() {
            let fstr = reader.get_file(fid as u32);
            if !fstr.is_empty() {
                if let Some(cfid) = self.src.lookup_abs_or_rel(sr, fstr) {
                    *slot = cfid;
                }
            }
        }
        let mut span_to_def_idx = HashMap::new();
        for (i, d) in reader.definitions().iter().enumerate() {
            let dk = DefKind::from(d.kind);
            if d.is_reexport() || d.is_macro_expansion() || matches!(dk, DefKind::Use | DefKind::Impl) {
                continue;
            }
            let sp = d.span();
            if let Some(&cfid) = file_map.get(sp.file_id() as usize).filter(|f| f.is_some()) {
                let cf = &self.src.files[cfid.0 as usize];
                if let (Some(sb), Some(eb)) = (cf.lines.to_byte(&cf.text, sp.start_line(), sp.start_col()), cf.lines.to_byte(&cf.text, sp.end_line(), sp.end_col())) {
                    if sb < eb {
                        span_to_def_idx.entry((cfid, sb, eb)).or_insert(i as u32);
                    }
                }
            }
        }
        let st = self.state.as_mut();
        st.shards[r_idx].file_map = file_map;
        st.shards[r_idx].span_to_def_idx = span_to_def_idx;
    }

    /// Activates `root_r_idx`, sysroot crates (`core`/`alloc`/`std`), `root_r_idx`'s direct/type-referenced
    /// crates, and transitive re-export/alias targets (excluding proc-macro host deps and sysroot OS internals).
    pub(crate) fn activate_reachable_shards(&mut self, root_r_idx: usize, loaded_dirs: &mut HashSet<PathBuf>, src_loaded: &mut [bool]) -> Vec<usize> {
        self.ensure_meta_at(root_r_idx);
        let root_needs_cst = self.ws.readers()[root_r_idx].fn_sigs().is_empty()
            || self.metas[root_r_idx].target_kind.contains("proc-macro")
            || (self.metas[root_r_idx].target_kind.is_empty() && self.ws.meta_path_at(root_r_idx).is_none() && !self.metas[root_r_idx].is_sysroot);
        if root_needs_cst {
            self.ensure_crate_sources(root_r_idx, loaded_dirs, src_loaded);
        }
        let newly = self.collect_reachable_unloaded(root_r_idx);
        self.materialize_shard_batch(newly, loaded_dirs, src_loaded)
    }

    fn collect_reachable_unloaded(&mut self, root_r_idx: usize) -> Vec<usize> {
        let n = self.ws.crate_count();
        let mut visited = vec![false; n];
        let mut q: VecDeque<(usize, u8)> = VecDeque::new();
        let want_pm = self.metas[root_r_idx].target_kind.contains("proc-macro") || self.metas[root_r_idx].extern_units.contains_key("proc_macro");
        for start in std::iter::once(root_r_idx).chain(self.sysroot_by_name.iter().filter(|(k, _)| want_pm || k.as_str() != "proc_macro").map(|(_, &i)| i)) {
            if start < n && !visited[start] {
                visited[start] = true;
                q.push_back((start, 0));
            }
        }
        let mut newly = Vec::new();
        while let Some((u, depth)) = q.pop_front() {
            if !self.shard_loaded[u] {
                newly.push(u);
            }
            if self.metas[u].is_sysroot || depth >= 6 || (u != root_r_idx && self.shard_loaded[u] && depth >= self.shard_expanded_depth[u]) {
                continue;
            }
            self.ensure_meta_at(u);
            if u != root_r_idx && self.metas[u].target_kind.contains("proc-macro") {
                continue;
            }
            let next_d = depth + 1;
            let ws = self.ws;
            if u == root_r_idx {
                let uks: Vec<String> = self.metas[u].extern_units.values().cloned().collect();
                for uk in uks {
                    if let Some(v) = self.resolve_unit_to_reader(u, &uk).filter(|&v| !visited[v]) {
                        visited[v] = true;
                        q.push_back((v, next_d));
                    }
                }
                let reader = &ws.readers()[u];
                let arena = reader.ty_arena();
                for i in 0..reader.ty_defs().len() {
                    if let Some(td) = arena.def(TyDefId(i as u32)) {
                        self.enqueue_td(u, td, next_d, &mut visited, &mut q);
                    }
                }
            } else {
                self.state.as_mut().shard_expanded_depth[u] = depth;
                let reader = &ws.readers()[u];
                let arena = reader.ty_arena();
                for (didx, d) in reader.definitions().iter().enumerate() {
                    if d.visibility != ItemVis::Public as u8 {
                        continue;
                    }
                    if d.is_reexport() {
                        if depth >= 3 {
                            continue;
                        }
                        let first = reader.get_str(d.sig_offset()).split("::").next().unwrap_or("");
                        if let Some(v) = (!first.is_empty()).then(|| self.resolve_extern_name_to_reader(u, first)).flatten().filter(|&v| !visited[v]) {
                            visited[v] = true;
                            q.push_back((v, next_d));
                        }
                    } else if matches!(DefKind::from(d.kind), DefKind::TyAlias | DefKind::Struct | DefKind::Enum | DefKind::Union | DefKind::Trait) {
                        if DefKind::from(d.kind) == DefKind::TyAlias {
                            if let Some(TyKindView::Adt { def, .. }) = reader.def_ty_for_def(didx as u32).map(|dt| arena.kind(dt.ty())) {
                                if let Some(td) = arena.def(def) {
                                    self.enqueue_td(u, td, next_d, &mut visited, &mut q);
                                }
                            }
                        }
                        if depth < 3 {
                            for gp in reader.generic_params_for_def(didx as u32) {
                                if let Some(TyKindView::Adt { def, .. }) = gp.default_ty().map(|dt| arena.kind(dt)) {
                                    if let Some(td) = arena.def(def) {
                                        self.enqueue_td(u, td, next_d, &mut visited, &mut q);
                                    }
                                }
                            }
                        }
                    }
                }
                if depth < 3 {
                    for ue in reader.uses_for_owner(UseOwner::CrateRoot) {
                        if (ue.flags & USE_FLAG_REEXPORT) != 0 && ue.vis == ItemVis::Public {
                            for td_id in [ue.type_def, ue.value_def, ue.macro_def].into_iter().flatten() {
                                if let Some(td) = arena.def(td_id) {
                                    self.enqueue_td(u, td, next_d, &mut visited, &mut q);
                                }
                            }
                        }
                    }
                }
            }
        }
        newly
    }

    fn materialize_shard_batch(&mut self, mut newly: Vec<usize>, loaded_dirs: &mut HashSet<PathBuf>, src_loaded: &mut [bool]) -> Vec<usize> {
        if newly.is_empty() {
            return newly;
        }
        newly.sort_unstable();
        for &idx in &newly {
            self.ensure_meta_at(idx);
            if !self.metas[idx].is_dependency && self.metas[idx].target_kind.contains("proc-macro") {
                self.ensure_crate_sources(idx, loaded_dirs, src_loaded);
            }
        }
        let ws = self.ws;
        for &idx in &newly {
            let rdr = &ws.readers()[idx];
            let built = build_shard_tables(idx, rdr, &self.state.metas[idx], &self.src, false);
            let st = self.state.as_mut();
            st.shards[idx] = built;
            st.shard_loaded[idx] = true;
        }
        self.index_proc_macro_exports_for(&newly);
        self.populate_raw_ty_def_gids_for(&newly);
        self.index_impls_and_traits_for(&newly);
        self.build_ty_tables_for(&newly);
        newly
    }

    #[inline]
    fn enqueue_td(&mut self, u: usize, td: ridx_format::TyDefView<'_>, next_d: u8, visited: &mut [bool], q: &mut VecDeque<(usize, u8)>) {
        if td.local_def_idx.is_some() {
            return;
        }
        let target = td.extern_unit.and_then(|uk| self.resolve_unit_to_reader(u, uk)).or_else(|| {
            let first = td.path.split("::").next().unwrap_or(td.path);
            self.resolve_extern_name_to_reader(u, first)
        });
        if let Some(v) = target.filter(|&v| !visited[v]) {
            visited[v] = true;
            q.push_back((v, next_d));
        }
    }
}
