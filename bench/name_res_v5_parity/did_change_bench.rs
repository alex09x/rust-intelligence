/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Realistic LSP `didChange` latency benchmark on the largest compilation unit of a corpus:
//! 1) Body-only keystroke (`parse(dirty_text)` + `snap.resolve_file_paths_into`),
//! 2) Item / `use`-import edit (`parse(dirty_text)` + `snap.with_dirty_files` + `snap.resolve_dirty_file_paths_into`),
//! 3) Checkbox 4.1 dirty-function tracking (`DirtyFileTracker::on_did_change_with_db`: reparse file,
//!    find changed `fn` bodies, infer only those; plus `O(1)` shift-reuse when editing above a dirty fn).

use std::time::Instant;

use ridx_syntax::name_res::{CrateId, CrateSnapshot, FileId, FileNameRes, SourceDb};
use ridx_syntax::parse;
use ridx_syntax::ty_infer::{extract_file_fn_summaries, DirtyFileTracker, FnBodySummary};

use crate::meta::CorpusSourceDb;
use crate::v5_db::RidxV5ItemDb;
use crate::{reset_alloc_counters, snapshot_alloc_counters};

struct DirtyFileOverlay<'a> {
    base: &'a CorpusSourceDb,
    dirty_fid: FileId,
    dirty_text: String,
}

impl SourceDb for DirtyFileOverlay<'_> {
    #[inline]
    fn resolve_path(&self, normalized_path: &str) -> Option<FileId> {
        self.base.resolve_path(normalized_path)
    }

    #[inline]
    fn file_path(&self, file: FileId) -> &str {
        self.base.file_path(file)
    }

    #[inline]
    fn file_text(&self, file: FileId) -> &str {
        if file == self.dirty_fid {
            self.dirty_text.as_str()
        } else {
            self.base.file_text(file)
        }
    }
}

pub struct DidChangeReport {
    pub corpus: String,
    pub unit_crate: String,
    pub unit_files: usize,
    pub sampled_files: usize,
    pub body_p50_us: f64,
    pub body_p95_us: f64,
    pub body_mean_us: f64,
    pub item_p50_us: f64,
    pub item_p95_us: f64,
    pub item_mean_us: f64,
    pub dirty_fn_probes: usize,
    pub dirty_fn_total_fns: usize,
    pub dirty_fn_inferred_total: usize,
    pub dirty_fn_p50_us: f64,
    pub dirty_fn_p95_us: f64,
    pub dirty_fn_p99_us: f64,
    pub dirty_fn_max_us: f64,
    pub dirty_fn_mean_us: f64,
    pub shift_reuse_p50_us: f64,
    pub shift_reuse_p95_us: f64,
    pub warm_allocs_per_edit: f64,
    pub tracker_heap_kib: f64,
}

fn percentile_us(sorted_ns: &[u128], num: usize, den: usize) -> f64 {
    if sorted_ns.is_empty() {
        return 0.0;
    }
    let idx = ((sorted_ns.len() - 1) * num) / den;
    (sorted_ns[idx] as f64) / 1_000.0
}

pub fn bench_did_change(corpus: &str, src: &CorpusSourceDb, db: &RidxV5ItemDb<'_>, unit_indices: &[usize]) -> Option<DidChangeReport> {
    let mut best: Option<(usize, CrateSnapshot, Vec<FileId>)> = None;
    for &r_idx in unit_indices {
        let meta = &db.metas[r_idx];
        if meta.is_test_harness {
            continue;
        }
        let Some(root_abs) = meta.root_rs_abs.as_ref() else {
            continue;
        };
        let Some(root_fid) = src.lookup_abs_or_rel(meta.source_root.as_deref(), &root_abs.to_string_lossy()) else {
            continue;
        };
        let snap = CrateSnapshot::build_clean(db, src, CrateId(r_idx as u32), root_fid);
        let mut fids: Vec<FileId> = snap.mod_tree.modules.iter().map(|m| m.file).filter(|f| f.is_some()).collect();
        fids.sort_unstable_by_key(|f| f.0);
        fids.dedup();
        if best.as_ref().is_none_or(|(_, _, prev)| fids.len() > prev.len()) {
            best = Some((r_idx, snap, fids));
        }
    }
    let (r_idx, snap, fids) = best?;
    if fids.is_empty() {
        return None;
    }

    let mut res_buf = FileNameRes::default();
    // Warm up external scope & trait caches once (represents steady-state open workspace).
    for &fid in &fids {
        let st = parse(&src.files[fid.0 as usize].text);
        snap.resolve_file_paths_into(db, fid, &st, &mut res_buf);
    }

    let max_probes = 64usize.min(fids.len());
    let stride = (fids.len() / max_probes).max(1);
    let probed: Vec<FileId> = fids.iter().copied().step_by(stride).take(max_probes).collect();

    let mut body_ns = Vec::with_capacity(probed.len());
    let mut item_ns = Vec::with_capacity(probed.len());
    let mut dirty_fn_ns = Vec::with_capacity(probed.len());
    let mut shift_ns = Vec::with_capacity(probed.len());
    let mut tracker = DirtyFileTracker::new();
    let mut summaries_buf: Vec<FnBodySummary> = Vec::with_capacity(64);
    let mut dirty_fn_total_fns = 0usize;
    let mut dirty_fn_inferred_total = 0usize;
    let mut warm_allocs_total = 0u64;

    for &fid in &probed {
        let orig = &src.files[fid.0 as usize].text;
        let mut body_edit = String::with_capacity(orig.len() + 32);
        body_edit.push_str(orig);
        body_edit.push_str("\n// keystroke\n");

        let t0 = Instant::now();
        let st_body = parse(&body_edit);
        snap.resolve_file_paths_into(db, fid, &st_body, &mut res_buf);
        body_ns.push(t0.elapsed().as_nanos());

        let mut item_edit = String::with_capacity(orig.len() + 96);
        item_edit.push_str(orig);
        item_edit.push_str("\n#[allow(dead_code)]\nfn __ridx_did_change_probe() -> usize { 42 }\n");
        let overlay = DirtyFileOverlay { base: src, dirty_fid: fid, dirty_text: item_edit };

        let t1 = Instant::now();
        let st_item = parse(overlay.file_text(fid));
        let (new_mt, dm) = snap.with_dirty_files(db, &overlay, &[fid], |f| (f == fid).then_some(&st_item));
        snap.resolve_dirty_file_paths_into(db, new_mt.as_ref(), &dm, fid, &st_item, &mut res_buf);
        item_ns.push(t1.elapsed().as_nanos());

        let st_orig = parse(orig);
        let _ = extract_file_fn_summaries(&st_orig, &mut summaries_buf);
        if !summaries_buf.is_empty() {
            let target_fn = summaries_buf[summaries_buf.len() / 2];
            let insert_at = (target_fn.body_start as usize + 1).min(orig.len());
            if orig.is_char_boundary(insert_at) {
                let mut fn_edit = String::with_capacity(orig.len() + 48);
                fn_edit.push_str(&orig[..insert_at]);
                fn_edit.push_str(" let __ridx_dirty_local = 1i32; ");
                fn_edit.push_str(&orig[insert_at..]);

                let mut shifted_edit = String::with_capacity(fn_edit.len() + 16);
                shifted_edit.push_str("// shift\n");
                shifted_edit.push_str(&fn_edit);

                // Warm up tracker capacities on this file
                tracker.set_baseline(orig);
                let _ = tracker.on_did_change_with_db(&fn_edit, fid, &snap.clean_def_map, db, db);
                let _ = tracker.on_did_change_with_db(&shifted_edit, fid, &snap.clean_def_map, db, db);
                let _ = tracker.on_did_change_with_db(orig, fid, &snap.clean_def_map, db, db);

                reset_alloc_counters();
                let td = Instant::now();
                let st_d = tracker.on_did_change_with_db(&fn_edit, fid, &snap.clean_def_map, db, db);
                let elapsed_d = td.elapsed().as_nanos();
                let (allocs_d, _) = snapshot_alloc_counters();
                let _ = allocs_d;

                reset_alloc_counters();
                let ts = Instant::now();
                let st_s = tracker.on_did_change_with_db(&shifted_edit, fid, &snap.clean_def_map, db, db);
                let elapsed_s = ts.elapsed().as_nanos();
                let (allocs_s, _) = snapshot_alloc_counters();
                let _ = st_s;

                dirty_fn_ns.push(elapsed_d);
                shift_ns.push(elapsed_s);
                dirty_fn_total_fns += st_d.total_fns as usize;
                dirty_fn_inferred_total += st_d.inferred_now as usize;
                warm_allocs_total += allocs_d + allocs_s;
            }
        }
    }

    body_ns.sort_unstable();
    item_ns.sort_unstable();
    dirty_fn_ns.sort_unstable();
    shift_ns.sort_unstable();
    let body_sum: u128 = body_ns.iter().sum();
    let item_sum: u128 = item_ns.iter().sum();
    let df_sum: u128 = dirty_fn_ns.iter().sum();
    let n = probed.len().max(1) as f64;
    let df_n = dirty_fn_ns.len().max(1) as f64;

    Some(DidChangeReport {
        corpus: corpus.to_string(),
        unit_crate: db.metas[r_idx].crate_name.clone(),
        unit_files: fids.len(),
        sampled_files: probed.len(),
        body_p50_us: percentile_us(&body_ns, 50, 100),
        body_p95_us: percentile_us(&body_ns, 95, 100),
        body_mean_us: (body_sum as f64) / (n * 1_000.0),
        item_p50_us: percentile_us(&item_ns, 50, 100),
        item_p95_us: percentile_us(&item_ns, 95, 100),
        item_mean_us: (item_sum as f64) / (n * 1_000.0),
        dirty_fn_probes: dirty_fn_ns.len(),
        dirty_fn_total_fns,
        dirty_fn_inferred_total,
        dirty_fn_p50_us: percentile_us(&dirty_fn_ns, 50, 100),
        dirty_fn_p95_us: percentile_us(&dirty_fn_ns, 95, 100),
        dirty_fn_p99_us: percentile_us(&dirty_fn_ns, 99, 100),
        dirty_fn_max_us: dirty_fn_ns.last().copied().unwrap_or(0) as f64 / 1_000.0,
        dirty_fn_mean_us: (df_sum as f64) / (df_n * 1_000.0),
        shift_reuse_p50_us: percentile_us(&shift_ns, 50, 100),
        shift_reuse_p95_us: percentile_us(&shift_ns, 95, 100),
        warm_allocs_per_edit: (warm_allocs_total as f64) / df_n,
        tracker_heap_kib: (tracker.heap_bytes() as f64) / 1024.0,
    })
}
