/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Phase `2.5` 7-corpus reference-parity & warm throughput/allocation harness.
//!
//! Compares `ridx_syntax::name_res` path/name resolution against `ridx-driver` (`v5` `.ridx`)
//! across unchanged workspace `.rs` files of `serde`, `ripgrep`, `regex`, `clap`, `tokio`,
//! `rust-analyzer`, and `bevy` without modifying any workspace `Cargo.toml` or `Cargo.lock`.

mod compare;
mod meta;
mod site_classify;
mod target_match;
mod v5_db;
mod v5_db_queries;
mod v5_shard;

use std::alloc::{GlobalAlloc, Layout, System};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use ridx_format::WorkspaceIndex;

use crate::compare::run_corpus;
use crate::meta::{load_reader_metas, CorpusSourceDb};
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
    let mut v5_root = PathBuf::from("/tmp/ridx-v5-2_5");
    let mut src_root = PathBuf::from("/tmp/ridx-bench-wt");
    let mut filter_corpus: Option<String> = None;
    let mut verbose_samples: usize = 8;

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
        "| Corpus | Units | Files | CST Refs (P2) | Matched | CST % | Miss | Wrong | MacroTT Match | Excl MacroExp | Excl Dot (P3) | Excl Synth | ns/file | allocs/file | bytes/file |"
    );
    println!("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|");

    let mut grand_refs = 0usize;
    let mut grand_matched = 0usize;
    let mut grand_missing = 0usize;
    let mut grand_wrong = 0usize;
    let mut grand_mtt_refs = 0usize;
    let mut grand_mtt_matched = 0usize;
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
        let src_db = CorpusSourceDb::load(&ws_src);
        let metas = load_reader_metas(&ws);
        let item_db = RidxV5ItemDb::build(&ws, &src_db, metas);

        let rep = run_corpus(cname, &src_db, &item_db, verbose_samples);
        let pct = if rep.phase2_driver_refs == 0 { 100.0 } else { (rep.matched_refs as f64) * 100.0 / (rep.phase2_driver_refs as f64) };
        let fdiv = rep.files.max(1) as f64;
        let ns_per_file = (rep.total_ns as f64) / fdiv;
        let allocs_per_file = (rep.warm_allocs as f64) / fdiv;
        let bytes_per_file = (rep.warm_bytes as f64) / fdiv;

        println!(
            "| `{}` | {} | {} | {} | {} | **{:.2}%** | {} | {} | {}/{} | {} | {} | {} | {:.1} | {:.2} | {:.1} |",
            rep.corpus,
            rep.units,
            rep.files,
            rep.phase2_driver_refs,
            rep.matched_refs,
            pct,
            rep.missing_refs,
            rep.wrong_target_refs,
            rep.matched_macro_arg_refs,
            rep.macro_arg_refs,
            rep.excluded_macro_refs,
            rep.excluded_dot_refs,
            rep.excluded_synth_refs,
            ns_per_file,
            allocs_per_file,
            bytes_per_file
        );

        grand_refs += rep.phase2_driver_refs;
        grand_matched += rep.matched_refs;
        grand_missing += rep.missing_refs;
        grand_wrong += rep.wrong_target_refs;
        grand_mtt_refs += rep.macro_arg_refs;
        grand_mtt_matched += rep.matched_macro_arg_refs;
        all_reports.push(rep);
    }

    if all_reports.len() > 1 && grand_refs > 0 {
        let gpct = (grand_matched as f64) * 100.0 / (grand_refs as f64);
        println!(
            "| **TOTAL** | — | — | **{}** | **{}** | **{:.2}%** | **{}** | **{}** | **{}/{}** | — | — | — | — | — | — |",
            grand_refs, grand_matched, gpct, grand_missing, grand_wrong, grand_mtt_matched, grand_mtt_refs
        );
    }

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
        for (k, v) in buckets.iter().take(15) {
            println!("  {:>6}  {}", v, k);
        }
        let mut mbuckets: Vec<(&String, &usize)> = rep.macro_miss_buckets.iter().collect();
        mbuckets.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
        for (k, v) in mbuckets.iter().take(6) {
            println!("  {:>6}  {} (Checkbox 3.6)", v, k);
        }
        for s in &rep.samples {
            println!("    sample: {s}");
        }
    }
}
