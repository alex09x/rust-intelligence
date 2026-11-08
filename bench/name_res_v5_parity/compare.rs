/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Reference-by-reference comparator and benchmark harness between `ridx-driver` `RefEntry`s
//! and `ridx_syntax::name_res` `ResolvedRef`s across unchanged workspace files.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use memmap2::Mmap;
use ridx_format::{DefKind, IndexReader};
use ridx_syntax::name_res::{CrateId, CrateSnapshot, FileId, FileNameRes, SourceDb};
use ridx_syntax::{parse, SyntaxTree};

use crate::did_change_bench::{bench_did_change, DidChangeReport};
use crate::meta::{CorpusFile, CorpusSourceDb};
use crate::site_classify::{classify_ref_site, collect_dot_access_spans, collect_unit_alias_pairs, SiteKind};
use crate::target_match::{describe_our_target, ref_targets_match, MatchKind};
use crate::v5_db::RidxV5ItemDb;
use crate::{reset_alloc_counters, snapshot_alloc_counters};

#[derive(Default)]
pub struct CorpusReport {
    pub corpus: String,
    pub units: usize,
    pub files: usize,
    pub phase2_driver_refs: usize,
    pub matched_refs: usize,
    pub strict_matched_refs: usize,
    pub strict_local_ref_mut_refs: usize,
    pub strict_local_ref_self_refs: usize,
    pub lenient_local_refs: usize,
    pub lenient_assoc_refs: usize,
    pub missing_refs: usize,
    pub wrong_target_refs: usize,
    pub macro_arg_refs: usize,
    pub matched_macro_arg_refs: usize,
    pub excluded_macro_refs: usize,
    pub excluded_dot_refs: usize,
    pub excluded_synth_refs: usize,
    pub synth_by_reason: [usize; 4],
    pub synth_samples: [Vec<String>; 4],
    pub moved_alias_refs: usize,
    pub moved_alias_matched: usize,
    pub moved_alias_samples: Vec<String>,
    pub excluded_non_name_refs: usize,
    pub total_ns: u128,
    pub parse_plus_resolve_ns: u128,
    pub warm_allocs: u64,
    pub warm_bytes: u64,
    pub miss_buckets: HashMap<String, usize>,
    pub macro_miss_buckets: HashMap<String, usize>,
    pub samples: Vec<String>,
    pub did_change: Option<DidChangeReport>,
}

pub fn run_corpus(name: &str, src: &CorpusSourceDb, db: &RidxV5ItemDb<'_>, max_samples: usize, run_did_change: bool) -> CorpusReport {
    let mut rep = CorpusReport { corpus: name.to_string(), ..CorpusReport::default() };

    let mut seen_units: HashSet<(String, bool, u32)> = HashSet::new();
    let mut unit_indices: Vec<usize> = Vec::new();
    for idx in 0..db.ws.crate_count() {
        let meta = &db.metas[idx];
        if meta.is_dependency || meta.target_kind == "custom-build" {
            continue;
        }
        let Some(root_abs) = meta.root_rs_abs.as_ref() else {
            continue;
        };
        let Some(root_fid) = src.lookup_abs_or_rel(meta.source_root.as_deref(), &root_abs.to_string_lossy()) else {
            continue;
        };
        let rcount = db.ws.readers()[idx].header().refs_count;
        let ukey = (src.file_path(root_fid).to_string(), meta.is_test_harness, rcount);
        if seen_units.insert(ukey) {
            unit_indices.push(idx);
        }
    }

    let mut res_buf = FileNameRes::default();

    for &r_idx in &unit_indices {
        let meta = &db.metas[r_idx];
        let root_abs = meta.root_rs_abs.as_ref().unwrap();
        let root_fid = src.lookup_abs_or_rel(meta.source_root.as_deref(), &root_abs.to_string_lossy()).unwrap();
        let krate = CrateId(r_idx as u32);

        let snap = CrateSnapshot::build_clean(db, src, krate, root_fid);
        rep.units += 1;

        let mut reached_files: Vec<FileId> = snap.mod_tree.modules.iter().map(|m| m.file).filter(|f| f.is_some()).collect();
        reached_files.sort_unstable_by_key(|f| f.0);
        reached_files.dedup();

        let mut unit_alias_pairs: HashSet<(String, String)> = HashSet::new();
        collect_unit_alias_pairs(r_idx, &snap, db, &mut unit_alias_pairs);

        let mut fid_to_ridx: HashMap<FileId, u32> = HashMap::new();
        for (rfid, &cfid) in db.shards[r_idx].file_map.iter().enumerate() {
            if cfid.is_some() {
                fid_to_ridx.entry(cfid).or_insert(rfid as u32);
            }
        }

        // Pre-parse & warm caches once (without `.ridx` mmap comparison interleaving).
        let mut parsed_trees: Vec<(FileId, SyntaxTree<'_>)> = Vec::with_capacity(reached_files.len());
        for &fid in &reached_files {
            let tp = Instant::now();
            let st = parse(&src.files[fid.0 as usize].text);
            let parse_elapsed = tp.elapsed().as_nanos();
            snap.resolve_file_paths_into(db, fid, &st, &mut res_buf);
            rep.parse_plus_resolve_ns += parse_elapsed;
            parsed_trees.push((fid, st));
        }

        // Measure pure warm `resolve_file_paths_into` throughput + heap allocations.
        for &(fid, ref st) in &parsed_trees {
            reset_alloc_counters();
            let t0 = Instant::now();
            snap.resolve_file_paths_into(db, fid, st, &mut res_buf);
            let elapsed = t0.elapsed().as_nanos();
            let (a, b) = snapshot_alloc_counters();
            rep.total_ns += elapsed;
            rep.parse_plus_resolve_ns += elapsed;
            rep.warm_allocs += a;
            rep.warm_bytes += b;
            rep.files += 1;
        }

        // Compare against `.ridx` `RefEntry`s.
        let reader = &db.ws.readers()[r_idx];
        for &(fid, ref st) in &parsed_trees {
            let Some(&ridx_fid) = fid_to_ridx.get(&fid) else {
                continue;
            };
            snap.resolve_file_paths_into(db, fid, st, &mut res_buf);
            let cf = &src.files[fid.0 as usize];
            compare_file_refs(r_idx, reader, cf, st, &res_buf, &unit_alias_pairs, db, ridx_fid, max_samples, &mut rep);
        }
    }

    if run_did_change {
        rep.did_change = bench_did_change(name, src, db, &unit_indices);
    }

    rep
}

#[allow(clippy::too_many_arguments)]
fn compare_file_refs(
    r_idx: usize,
    reader: &IndexReader<Mmap>,
    cf: &CorpusFile,
    st: &SyntaxTree<'_>,
    out: &FileNameRes,
    unit_alias_pairs: &HashSet<(String, String)>,
    db: &RidxV5ItemDb<'_>,
    ridx_fid: u32,
    max_samples: usize,
    rep: &mut CorpusReport,
) {
    let mut our_by_span: HashMap<(u32, u32), Vec<usize>> = HashMap::with_capacity(out.refs.len());
    for (idx, rr) in out.refs.iter().enumerate() {
        our_by_span.entry((rr.start, rr.end)).or_default().push(idx);
    }

    let dot_spans = collect_dot_access_spans(st);

    for r in reader.references_in_file(ridx_fid) {
        if !reader.is_name_reference(r) {
            rep.excluded_non_name_refs += 1;
            continue;
        }
        if r.is_macro_expansion() {
            rep.excluded_macro_refs += 1;
            continue;
        }
        let sp = r.span();
        let (Some(sb), Some(eb)) = (cf.lines.to_byte(&cf.text, sp.start_line(), sp.start_col()), cf.lines.to_byte(&cf.text, sp.end_line(), sp.end_col())) else {
            continue;
        };
        if sb >= eb {
            continue;
        }
        let dkind = DefKind::from(r.kind);
        if dot_spans.contains(&(sb, eb)) && matches!(dkind, DefKind::AssocFn | DefKind::Field | DefKind::Fn) {
            rep.excluded_dot_refs += 1;
            continue;
        }

        let cls = classify_ref_site(reader, db, cf, st, unit_alias_pairs, r, sb, eb);
        if let SiteKind::Synth(reason) = cls.kind {
            rep.excluded_synth_refs += 1;
            let ridx = reason as usize;
            rep.synth_by_reason[ridx] += 1;
            if rep.synth_samples[ridx].len() < 8 {
                let snip = cf.text.get(sb as usize..eb as usize).unwrap_or("?").replace('\n', "\\n");
                let tp = reader.get_str(r.target_path_offset());
                let key_tail = format!("`{snip}` ({dkind:?}) -> `{tp}`");
                if !rep.synth_samples[ridx].iter().any(|prev| prev.ends_with(&key_tail)) {
                    rep.synth_samples[ridx].push(format!("{}:{}:{} {key_tail}", cf.rel_path, sp.start_line(), sp.start_col()));
                }
            }
            continue;
        }

        let candidates = our_by_span.get(&(sb, eb)).or_else(|| our_by_span.get(&cls.tok_span));
        let best_mk =
            candidates.and_then(|cands| cands.iter().map(|&ci| ref_targets_match(r_idx, reader, db, cf, out, out.refs[ci].target, r)).max()).unwrap_or(MatchKind::NoMatch);
        let matched = best_mk.is_matched();

        if cls.moved_real_alias && cls.kind == SiteKind::DirectCst {
            rep.moved_alias_refs += 1;
            if matched {
                rep.moved_alias_matched += 1;
            }
            if rep.moved_alias_samples.len() < 8 {
                let snip = cf.text.get(sb as usize..eb as usize).unwrap_or("?");
                let tp = reader.get_str(r.target_path_offset());
                let key_tail = format!("`{snip}` ({dkind:?}) -> `{tp}` [match={best_mk:?}]");
                if !rep.moved_alias_samples.iter().any(|prev| prev.ends_with(&key_tail)) {
                    rep.moved_alias_samples.push(format!("{}:{}:{} {key_tail}", cf.rel_path, sp.start_line(), sp.start_col()));
                }
            }
        }

        if cls.kind == SiteKind::MacroArg {
            rep.macro_arg_refs += 1;
            if matched {
                rep.matched_macro_arg_refs += 1;
            } else {
                let tag = if candidates.is_none() { "MACRO_MISSING" } else { "MACRO_WRONG" };
                *rep.macro_miss_buckets.entry(format!("{tag}:{dkind:?}")).or_insert(0) += 1;
            }
            continue;
        }

        rep.phase2_driver_refs += 1;
        match best_mk {
            MatchKind::Strict => {
                rep.matched_refs += 1;
                rep.strict_matched_refs += 1;
                continue;
            }
            MatchKind::StrictLocalRefMut => {
                rep.matched_refs += 1;
                rep.strict_matched_refs += 1;
                rep.strict_local_ref_mut_refs += 1;
                continue;
            }
            MatchKind::StrictLocalRefSelf => {
                rep.matched_refs += 1;
                rep.strict_matched_refs += 1;
                rep.strict_local_ref_self_refs += 1;
                continue;
            }
            MatchKind::LenientAssocTrait => {
                rep.matched_refs += 1;
                rep.lenient_assoc_refs += 1;
                continue;
            }
            MatchKind::NoMatch => {}
        }

        let is_missing = candidates.is_none();
        if is_missing {
            rep.missing_refs += 1;
        } else {
            rep.wrong_target_refs += 1;
        }
        let prefix = if is_missing { "MISSING" } else { "WRONG" };
        let bucket = format!("{prefix}:{dkind:?}:{}", cls.label);
        *rep.miss_buckets.entry(bucket.clone()).or_insert(0) += 1;
        if rep.samples.len() < max_samples {
            let snip = cf.text.get(sb as usize..eb as usize).unwrap_or("?");
            let tp = reader.get_str(r.target_path_offset());
            if let Some(cands) = candidates {
                let first_our = describe_our_target(db, cf, out, out.refs[cands[0]].target);
                rep.samples.push(format!(
                    "[{bucket}] {}:{}:{} `{snip}` -> driver `{tp}` ({}:{}) vs our `{first_our}`",
                    cf.rel_path,
                    sp.start_line(),
                    sp.start_col(),
                    r.target_line(),
                    r.target_col()
                ));
            } else {
                rep.samples.push(format!("[{bucket}] {}:{}:{} `{snip}` -> driver `{tp}`", cf.rel_path, sp.start_line(), sp.start_col()));
            }
        }
    }
}
