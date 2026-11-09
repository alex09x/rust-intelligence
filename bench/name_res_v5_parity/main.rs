/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Phase `2.5` 7-corpus reference-parity & warm throughput/allocation/`didChange` harness.
//!
//! Compares `ridx_syntax::name_res` path/name resolution against `ridx-driver` (`v5` `.ridx`)
//! across unchanged workspace `.rs` files of `serde`, `ripgrep`, `regex`, `clap`, `tokio`,
//! `rust-analyzer`, and `bevy` without modifying any workspace `Cargo.toml` or `Cargo.lock`.

mod compare;
mod did_change_bench;
mod fault_bench;
mod fault_completion;
mod fault_report;
mod meta;
mod site_classify;
mod target_match;
mod ty_funnel;
mod ty_parity;
mod ty_report;
mod v5_db;
mod v5_db_queries;
mod v5_macro_res;
mod v5_shard;
mod v5_trait_solve;
mod v5_ty_bounds;
mod v5_ty_db;
mod v5_ty_defaults;
mod v5_ty_lower;
mod v5_ty_proj;
mod v5_ty_tables;

use std::alloc::{GlobalAlloc, Layout, System};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use ridx_format::WorkspaceIndex;

use crate::compare::run_corpus;
use crate::fault_report::print_fault_bench_tables;
use crate::meta::{load_reader_metas, CorpusSourceDb};
use crate::site_classify::SynthReason;
use crate::ty_report::print_ty_parity_tables;
use crate::v5_db::RidxV5ItemDb;

struct CountingAlloc;

static ALLOC_CALLS: AtomicU64 = AtomicU64::new(0);
static ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOC_CALLS.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOC_CALLS.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(new_size as u64, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

#[inline]
pub fn reset_alloc_counters() {
    ALLOC_CALLS.store(0, Ordering::Relaxed);
    ALLOC_BYTES.store(0, Ordering::Relaxed);
}

#[inline]
pub fn snapshot_alloc_counters() -> (u64, u64) {
    (ALLOC_CALLS.load(Ordering::Relaxed), ALLOC_BYTES.load(Ordering::Relaxed))
}

const DEFAULT_CORPORA: &[&str] = &["serde", "ripgrep", "regex", "clap", "tokio", "rust-analyzer", "bevy"];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut v5_root = PathBuf::from(std::env::var("RIDX_V5_ROOT").unwrap_or_else(|_| "/tmp/ridx-v5-2_5".to_string()));
    let mut src_root = PathBuf::from(std::env::var("RIDX_SRC_ROOT").unwrap_or_else(|_| "/tmp/ridx-bench-wt".to_string()));
    let mut filter_corpus: Option<String> = None;
    let mut verbose_samples: usize = 8;
    let mut run_did_change = false;
    let mut fault_n: usize = 0;

    let mut i = 1usize;
    while i < args.len() {
        match args[i].as_str() {
            "--v5-root" if i + 1 < args.len() => {
                v5_root = PathBuf::from(&args[i + 1]);
                i += 1;
            }
            "--src-root" if i + 1 < args.len() => {
                src_root = PathBuf::from(&args[i + 1]);
                i += 1;
            }
            "--corpus" if i + 1 < args.len() => {
                filter_corpus = Some(args[i + 1].clone());
                i += 1;
            }
            "--samples" if i + 1 < args.len() => {
                verbose_samples = args[i + 1].parse().unwrap_or(8);
                i += 1;
            }
            "--bench-did-change" => {
                run_did_change = true;
            }
            "--bench-fault" => {
                if fault_n == 0 {
                    fault_n = 10;
                }
            }
            "--fault-n" if i + 1 < args.len() => {
                fault_n = args[i + 1].parse().unwrap_or(10);
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }

    let cache_dir = v5_root.join("cache");
    std::env::set_var("RIDX_CACHE_DIR", &cache_dir);
    let corpora: Vec<&str> = match filter_corpus.as_deref() {
        Some("all") | None => DEFAULT_CORPORA.to_vec(),
        Some(one) => vec![one],
    };

    println!(
        "| Corpus | Units | Files | CST Refs (P2) | Matched | CST % | Strict % | LenLoc | LenAssoc | Miss | Wrong | MacroTT Match | Excl Dot (P3) | Excl Synth | Unified % | ns/file | allocs/file |"
    );
    println!("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|");

    let mut grand_refs = 0usize;
    let mut grand_matched = 0usize;
    let mut grand_strict = 0usize;
    let mut grand_strict_loc_ref_mut = 0usize;
    let mut grand_strict_loc_ref_self = 0usize;
    let mut grand_len_loc = 0usize;
    let mut grand_len_assoc = 0usize;
    let mut grand_missing = 0usize;
    let mut grand_wrong = 0usize;
    let mut grand_mtt_refs = 0usize;
    let mut grand_mtt_matched = 0usize;
    let mut grand_dot = 0usize;
    let mut grand_dot_matched = 0usize;
    let mut grand_synth = 0usize;
    let mut grand_synth_reasons = [0usize; 4];
    let mut grand_moved_alias = 0usize;
    let mut grand_moved_alias_matched = 0usize;
    let mut all_reports = Vec::new();

    for cname in corpora {
        let out_dir = v5_root.join(format!("{cname}-out"));
        let ws_src = src_root.join(cname);
        if !Path::new(&out_dir).exists() || !Path::new(&ws_src).exists() {
            eprintln!("SKIP {cname}: missing {} or {}", out_dir.display(), ws_src.display());
            continue;
        }
        let mut ws = WorkspaceIndex::open_with_cache_dir(&out_dir, &cache_dir).unwrap_or_else(|e| panic!("open {}: {e}", out_dir.display()));
        ws.set_workspace_root(Some(ws_src.clone()));
        let metas = load_reader_metas(&ws);
        let mut src_db = CorpusSourceDb::load(&ws_src);
        src_db.load_out_dirs(&metas);
        let item_db = RidxV5ItemDb::build(&ws, &src_db, metas);

        let rep = run_corpus(cname, &src_db, &item_db, verbose_samples, run_did_change, fault_n);
        let pct = if rep.phase2_driver_refs == 0 { 100.0 } else { (rep.matched_refs as f64) * 100.0 / (rep.phase2_driver_refs as f64) };
        let spct = if rep.phase2_driver_refs == 0 { 100.0 } else { (rep.strict_matched_refs as f64) * 100.0 / (rep.phase2_driver_refs as f64) };
        let dot_mat = rep.ty_stats.dot_cst_matched + rep.ty_stats.dot_mtt_matched;
        let uni_num = rep.matched_refs + rep.matched_macro_arg_refs + dot_mat;
        let uni_den = rep.phase2_driver_refs + rep.macro_arg_refs + rep.excluded_dot_refs;
        let upct = if uni_den == 0 { 100.0 } else { (uni_num as f64) * 100.0 / (uni_den as f64) };
        let fdiv = rep.files.max(1) as f64;
        let ns_per_file = (rep.total_ns as f64) / fdiv;
        let allocs_per_file = (rep.warm_allocs as f64) / fdiv;

        println!(
            "| `{}` | {} | {} | {} | {} | **{:.2}%** | {:.2}% | {} | {} | {} | {} | {}/{} | {} | {} | {:.2}% | {:.1} | {:.2} |",
            rep.corpus,
            rep.units,
            rep.files,
            rep.phase2_driver_refs,
            rep.matched_refs,
            pct,
            spct,
            rep.lenient_local_refs,
            rep.lenient_assoc_refs,
            rep.missing_refs,
            rep.wrong_target_refs,
            rep.matched_macro_arg_refs,
            rep.macro_arg_refs,
            rep.excluded_dot_refs,
            rep.excluded_synth_refs,
            upct,
            ns_per_file,
            allocs_per_file,
        );

        grand_refs += rep.phase2_driver_refs;
        grand_matched += rep.matched_refs;
        grand_strict += rep.strict_matched_refs;
        grand_strict_loc_ref_mut += rep.strict_local_ref_mut_refs;
        grand_strict_loc_ref_self += rep.strict_local_ref_self_refs;
        grand_len_loc += rep.lenient_local_refs;
        grand_len_assoc += rep.lenient_assoc_refs;
        grand_missing += rep.missing_refs;
        grand_wrong += rep.wrong_target_refs;
        grand_mtt_refs += rep.macro_arg_refs;
        grand_mtt_matched += rep.matched_macro_arg_refs;
        grand_dot += rep.excluded_dot_refs;
        grand_dot_matched += dot_mat;
        grand_synth += rep.excluded_synth_refs;
        for k in 0..4 {
            grand_synth_reasons[k] += rep.synth_by_reason[k];
        }
        grand_moved_alias += rep.moved_alias_refs;
        grand_moved_alias_matched += rep.moved_alias_matched;
        all_reports.push(rep);
    }

    if !all_reports.is_empty() && grand_refs > 0 {
        let gpct = (grand_matched as f64) * 100.0 / (grand_refs as f64);
        let gspct = (grand_strict as f64) * 100.0 / (grand_refs as f64);
        let uni_num = grand_matched + grand_mtt_matched + grand_dot_matched;
        let uni_den = grand_refs + grand_mtt_refs + grand_dot;
        let gupct = (uni_num as f64) * 100.0 / (uni_den as f64);
        if all_reports.len() > 1 {
            println!(
                "| **TOTAL** | — | — | **{}** | **{}** | **{:.2}%** | **{:.2}%** | **{}** | **{}** | **{}** | **{}** | **{}/{}** | **{}** | **{}** | **{:.2}%** | — | — |",
                grand_refs,
                grand_matched,
                gpct,
                gspct,
                grand_len_loc,
                grand_len_assoc,
                grand_missing,
                grand_wrong,
                grand_mtt_matched,
                grand_mtt_refs,
                grand_dot,
                grand_synth,
                gupct
            );
        }
        println!("\nUnified coverage: matched (CST + MacroTT + Dot) / (P2 + MacroTT + Dot) = {} / {} ({:.2}%)", uni_num, uni_den, gupct);
        println!(
            "Strict P2 match: {} / {} ({:.2}%); strict_local_ref_mut = {}, strict_local_ref_self = {}, lenient_local = {}, lenient_assoc_trait = {}",
            grand_strict, grand_refs, gspct, grand_strict_loc_ref_mut, grand_strict_loc_ref_self, grand_len_loc, grand_len_assoc
        );
        println!("Real renamed re-exports / type aliases kept in P2 (`leaf != ident` -> P2): {} (matched = {})", grand_moved_alias, grand_moved_alias_matched);
        let mut alias_shown = 0usize;
        for slot in 0..8 {
            for rep in all_reports.iter().rev() {
                if let Some(s) = rep.moved_alias_samples.get(slot) {
                    if alias_shown < 5 {
                        println!("  alias->P2 sample [{}]: {s}", rep.corpus);
                        alias_shown += 1;
                    }
                }
            }
        }
        println!("\n=== Disjoint `Excl Synth` Breakdown (total = {}) ===", grand_synth);
        println!("| Corpus | Excl Synth | span != token | inside Attr/Name/MacroRules | ADT TT attr/decl | driver leaf != source ident | Real Alias -> P2 (matched) |");
        println!("|---|---:|---:|---:|---:|---:|---:|");
        for r in &all_reports {
            println!(
                "| `{}` | {} | {} | {} | {} | {} | {} ({}) |",
                r.corpus, r.excluded_synth_refs, r.synth_by_reason[0], r.synth_by_reason[1], r.synth_by_reason[2], r.synth_by_reason[3], r.moved_alias_refs, r.moved_alias_matched,
            );
        }
        println!(
            "| **TOTAL** | **{}** | **{}** | **{}** | **{}** | **{}** | **{} ({})** |\n",
            grand_synth, grand_synth_reasons[0], grand_synth_reasons[1], grand_synth_reasons[2], grand_synth_reasons[3], grand_moved_alias, grand_moved_alias_matched,
        );
        for reason in SynthReason::ALL {
            let idx = reason as usize;
            println!("  * {} = {}", reason.label(), grand_synth_reasons[idx]);
            let mut shown = 0usize;
            for slot in 0..8 {
                for rep in all_reports.iter().rev() {
                    if let Some(s) = rep.synth_samples[idx].get(slot) {
                        if shown < 5 {
                            println!("      sample [{}]: {s}", rep.corpus);
                            shown += 1;
                        }
                    }
                }
            }
        }
    }

    if run_did_change {
        println!("\n=== Measured `didChange` Latency (largest crate per corpus) ===");
        println!("| Corpus | Crate | Crate Files | Probes | Body-only p50 (µs) | Body-only p95 (µs) | Body-only mean (µs) | Item/Use edit p50 (µs) | Item/Use edit p95 (µs) | Item/Use edit mean (µs) |");
        println!("|---|---|---:|---:|---:|---:|---:|---:|---:|---:|");
        for rep in &all_reports {
            if let Some(dc) = &rep.did_change {
                println!(
                    "| `{}` | `{}` | {} | {} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} |",
                    dc.corpus, dc.unit_crate, dc.unit_files, dc.sampled_files, dc.body_p50_us, dc.body_p95_us, dc.body_mean_us, dc.item_p50_us, dc.item_p95_us, dc.item_mean_us
                );
            }
        }
        println!("\n=== Checkbox 4.1 Dirty-Function Tracking on `didChange` (`DirtyFileTracker::on_did_change_with_db`) ===");
        println!("| Corpus | Crate | Probes | Total Fns | Inferred Fns | 1-Fn Edit p50 (µs) | 1-Fn Edit p95 (µs) | 1-Fn Edit p99 (µs) | 1-Fn Edit max (µs) | 1-Fn Edit mean (µs) | Shift-Reuse p50 (µs) | Shift-Reuse p95 (µs) | Warm Allocs/Edit | Tracker Heap (KiB) |");
        println!("|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
        for rep in &all_reports {
            if let Some(dc) = &rep.did_change {
                println!(
                    "| `{}` | `{}` | {} | {} | {} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} | {:.2} | {:.1} |",
                    dc.corpus,
                    dc.unit_crate,
                    dc.dirty_fn_probes,
                    dc.dirty_fn_total_fns,
                    dc.dirty_fn_inferred_total,
                    dc.dirty_fn_p50_us,
                    dc.dirty_fn_p95_us,
                    dc.dirty_fn_p99_us,
                    dc.dirty_fn_max_us,
                    dc.dirty_fn_mean_us,
                    dc.shift_reuse_p50_us,
                    dc.shift_reuse_p95_us,
                    dc.warm_allocs_per_edit,
                    dc.tracker_heap_kib,
                );
            }
        }
    }

    print_ty_parity_tables(&all_reports, verbose_samples);
    if fault_n > 0 {
        print_fault_bench_tables(&all_reports, fault_n, verbose_samples);
    }

    if verbose_samples > 0 {
        for rep in &all_reports {
            if rep.miss_buckets.is_empty() && rep.macro_miss_buckets.is_empty() {
                continue;
            }
            println!(
                "\n=== Mismatch classes for `{}` (CST miss={}, wrong={}; MacroTT unmatched={}/{}) ===",
                rep.corpus,
                rep.missing_refs,
                rep.wrong_target_refs,
                rep.macro_arg_refs.saturating_sub(rep.matched_macro_arg_refs),
                rep.macro_arg_refs
            );
            let mut buckets: Vec<(&String, &usize)> = rep.miss_buckets.iter().collect();
            buckets.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
            for (k, v) in buckets.iter().take(10) {
                println!("  {:>6}  {}", v, k);
            }
            for s in &rep.samples {
                println!("    sample: {s}");
            }
        }
    }
}
