/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Snapshot initialization, source file ID interning, method index warming, and dirty
//! overlay fingerprinting helpers for [`super::query_state::LiveEngine`].

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use ridx_format::WorkspaceIndex;
use ridx_syntax::name_res::{CrateId, CrateSnapshot, FileId};
use ridx_syntax::parse;
use ridx_syntax::ty_infer::extract_file_fn_summaries;

use crate::overlay::OverlayManager;

use super::query_loc::lookup_live_text;
use super::query_state::LiveEngine;
use super::v5_db::RidxV5ItemDb;

impl LiveEngine {
    pub(super) fn resolve_root_fid(&self, r_idx: usize, fallback_fid: FileId) -> FileId {
        let meta = &self.state.metas[r_idx];
        meta.root_rs_abs
            .as_ref()
            .and_then(|p| self.src.lookup_abs_or_rel(meta.source_root.as_deref(), &p.to_string_lossy()))
            .or_else(|| self.src.lookup_abs_or_rel(meta.source_root.as_deref(), "src/lib.rs"))
            .or_else(|| self.src.lookup_abs_or_rel(meta.source_root.as_deref(), "src/main.rs"))
            .unwrap_or(fallback_fid)
    }

    pub(super) fn ensure_snapshot(&mut self, ws: &WorkspaceIndex, r_idx: usize, fallback_fid: FileId) {
        if self.snapshots.contains_key(&r_idx) {
            return;
        }
        let krate = CrateId(r_idx as u32);
        let has_v5 = !ws.readers()[r_idx].fn_sigs().is_empty();
        if !has_v5 {
            RidxV5ItemDb::view_mut(ws, &mut self.src, &mut self.state).ensure_crate_sources(r_idx, &mut self.loaded_dirs, &mut self.src_loaded);
        }
        let root_fid = self.resolve_root_fid(r_idx, fallback_fid);
        let ext_c = std::mem::take(&mut self.shared_ext_cache);
        let db = RidxV5ItemDb::view(ws, &self.src, &self.state);
        let snap = if has_v5 {
            CrateSnapshot::build_from_index_with_cache(&db, krate, ext_c).unwrap_or_else(|| CrateSnapshot::build_clean_for_query(&db, &self.src, krate, root_fid))
        } else {
            CrateSnapshot::build_clean_for_query(&db, &self.src, krate, root_fid)
        };
        self.shared_ext_cache = snap.take_ext_cache();
        self.snapshots.insert(r_idx, snap);
    }

    pub(super) fn ensure_file_id(
        &mut self,
        abs: &Path,
        canon: &Path,
        manual: &HashMap<PathBuf, String>,
        overlays: Option<&OverlayManager>,
        clean_hint: Option<&str>,
    ) -> Option<FileId> {
        if let Some(&fid) = self.src.by_abs.get(canon).or_else(|| self.src.by_abs.get(abs)) {
            return Some(fid);
        }
        let disk_text = clean_hint.map(str::to_owned).or_else(|| fs::read_to_string(canon).ok()).or_else(|| fs::read_to_string(abs).ok());
        let clean_text = match disk_text {
            Some(t) => t,
            None if lookup_live_text(abs, canon, manual, overlays).is_some() => String::new(),
            None => return None,
        };
        Some(self.src.upsert_canonical_file(abs, canon, clean_text))
    }

    pub fn ensure_method_index(&mut self, ws: &WorkspaceIndex) {
        if !self.pending_method_shards.is_empty() {
            let newly = std::mem::take(&mut self.pending_method_shards);
            super::query_unit::extend_method_index(ws, &self.src, &self.state, &mut self.trait_method_names, &mut self.inherent_names_by_head, &newly);
        }
    }

    pub(super) fn collect_crate_dirty_files(
        &mut self,
        r_idx: usize,
        manual: &HashMap<PathBuf, String>,
        overlays: Option<&OverlayManager>,
        out: &mut Vec<(FileId, String)>,
        sig: &mut u64,
    ) {
        let sr = self.state.metas[r_idx].source_root.clone().unwrap_or_else(|| self.src.ws_root.clone());
        let sr_canon = sr.canonicalize().unwrap_or(sr);
        let mut check_one = |p: &Path, txt: &str, engine: &mut Self| {
            let pc = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
            if !pc.starts_with(&sr_canon) {
                return;
            }
            let dfid = engine.src.by_abs.get(&pc).copied().unwrap_or_else(|| engine.src.upsert_file_text(&pc, String::new()));
            let clean_txt = &engine.src.files[dfid.0 as usize].text;
            if **clean_txt == txt {
                return;
            }
            let clean_eh = *engine.clean_env_hashes.entry(dfid).or_insert_with(|| extract_file_fn_summaries(&parse(clean_txt), &mut Vec::new()));
            let live_eh = extract_file_fn_summaries(&parse(txt), &mut Vec::new());
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
                let p = Path::new(&pstr);
                if !manual.contains_key(p) {
                    if let Some(doc) = ov.get(&pstr).filter(|d| d.is_dirty()) {
                        check_one(p, &doc.text, self);
                    }
                }
            }
        }
    }
}
