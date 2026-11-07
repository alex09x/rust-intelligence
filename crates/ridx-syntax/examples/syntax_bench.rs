/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Throughput, latency, memory-allocation, and `xref live profile` benchmark harness for `ridx-syntax`.
//!
//! Modes:
//! - `syntax_bench [dir ...]` — runs allocation + multi-iteration throughput benchmarks on workspace + dirs.
//! - `syntax_bench --profile <lex|parse|sema|all> <secs> [dir ...]` — runs a steady hot loop for `xref live profile`.

use ridx_syntax::sema_fn::{own_returns, own_returns_into, own_try_ops, own_try_ops_into, param_uses, param_uses_into, params, params_into, tail_expr, ParamInfo, ParamUses};
use ridx_syntax::tree::Node;
use ridx_syntax::{lex, parse, NodeId, SyntaxKind as K, SyntaxTree, Token};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

struct CountingAlloc;

#[derive(Clone, Copy, Default, Debug)]
struct AllocStats {
    enabled: bool,
    allocs: u64,
    reallocs: u64,
    deallocs: u64,
    total_bytes: u64,
    live_bytes: i64,
    peak_live_bytes: i64,
}

thread_local! {
    static STATS: Cell<AllocStats> = const { Cell::new(AllocStats {
        enabled: false,
        allocs: 0,
        reallocs: 0,
        deallocs: 0,
        total_bytes: 0,
        live_bytes: 0,
        peak_live_bytes: 0,
    }) };
}

// SAFETY: Delegates directly to `std::alloc::System` while updating thread-local counters.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            let _ = STATS.try_with(|c| {
                let mut s = c.get();
                if s.enabled {
                    s.allocs += 1;
                    s.total_bytes += layout.size() as u64;
                    s.live_bytes += layout.size() as i64;
                    if s.live_bytes > s.peak_live_bytes {
                        s.peak_live_bytes = s.live_bytes;
                    }
                    c.set(s);
                }
            });
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let _ = STATS.try_with(|c| {
            let mut s = c.get();
            if s.enabled {
                s.deallocs += 1;
                s.live_bytes -= layout.size() as i64;
                c.set(s);
            }
        });
        unsafe { System.dealloc(ptr, layout) };
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let out = unsafe { System.realloc(ptr, layout, new_size) };
        if !out.is_null() {
            let _ = STATS.try_with(|c| {
                let mut s = c.get();
                if s.enabled {
                    s.reallocs += 1;
                    if new_size > layout.size() {
                        s.total_bytes += (new_size - layout.size()) as u64;
                    }
                    s.live_bytes += new_size as i64 - layout.size() as i64;
                    if s.live_bytes > s.peak_live_bytes {
                        s.peak_live_bytes = s.live_bytes;
                    }
                    c.set(s);
                }
            });
        }
        out
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn measure_allocs<R>(f: impl FnOnce() -> R) -> (R, AllocStats) {
    STATS.with(|c| {
        c.set(AllocStats { enabled: true, ..AllocStats::default() });
    });
    let res = f();
    let snap = STATS.with(|c| {
        let mut s = c.get();
        s.enabled = false;
        c.set(s);
        s
    });
    (res, snap)
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        let name = e.file_name();
        let name = name.to_string_lossy();
        if p.is_dir() {
            if name != "target" && name != ".git" && name != "test_data" {
                rs_files(&p, out);
            }
        } else if name.ends_with(".rs") {
            out.push(p);
        }
    }
}

fn load_sources(dirs: &[PathBuf]) -> Vec<String> {
    let mut paths = Vec::new();
    for d in dirs {
        rs_files(d, &mut paths);
    }
    paths.into_iter().filter_map(|p| std::fs::read_to_string(p).ok()).collect()
}

#[inline(never)]
fn run_sema_on_tree(tree: &SyntaxTree<'_>) -> (usize, usize) {
    let mut fns = 0usize;
    let mut hits = 0usize;
    for id in tree.descendants(tree.root()) {
        if tree.kind(id) != K::Fn {
            continue;
        }
        fns += 1;
        let ps = params(tree, id);
        for p in &ps {
            for &b in &p.bindings {
                let u = param_uses(tree, id, b);
                hits += u.uses.len() + u.macro_uses.len();
            }
        }
        hits += own_returns(tree, id).len();
        hits += own_try_ops(tree, id).len();
        if tail_expr(tree, id).is_some() {
            hits += 1;
        }
    }
    (fns, hits)
}

fn run_profile_loop(mode: &str, secs: u64, sources: &[String]) {
    let trees: Vec<(SyntaxTree<'_>, Vec<NodeId>)> = if mode == "sema" {
        sources
            .iter()
            .map(|s| {
                let t = parse(s);
                let fns: Vec<NodeId> = t.descendants(t.root()).filter(|&n| t.kind(n) == K::Fn).collect();
                (t, fns)
            })
            .collect()
    } else {
        Vec::new()
    };
    eprintln!("PROFILE_READY pid={} mode={mode} files={}", std::process::id(), sources.len());
    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut iters = 0u64;
    while Instant::now() < deadline {
        match mode {
            "lex" => {
                for s in sources {
                    black_box(lex(black_box(s.as_str())));
                }
            }
            "parse" => {
                for s in sources {
                    black_box(parse(black_box(s.as_str())));
                }
            }
            "sema" => {
                for (t, _) in &trees {
                    black_box(run_sema_on_tree(black_box(t)));
                }
            }
            _ => {
                for s in sources {
                    let t = parse(black_box(s.as_str()));
                    black_box(run_sema_on_tree(&t));
                }
            }
        }
        iters += 1;
    }
    eprintln!("PROFILE_DONE pid={} mode={mode} iters={iters}", std::process::id());
}

fn bench_corpus(label: &str, sources: &[String], iters: usize) {
    let total_bytes: usize = sources.iter().map(String::len).sum();
    let mib = total_bytes as f64 / (1024.0 * 1024.0);
    println!("\n=== Corpus: {label} ({} files, {:.2} MiB) | sizeof(Token)={}B sizeof(Node)={}B ===", sources.len(), mib, std::mem::size_of::<Token>(), std::mem::size_of::<Node>(),);

    // 1. Lexer allocation + best-of-N timing
    let (_, lex_alloc) = measure_allocs(|| {
        for s in sources {
            black_box(lex(s));
        }
    });
    let mut best_lex = f64::MAX;
    for _ in 0..iters {
        let t0 = Instant::now();
        for s in sources {
            black_box(lex(black_box(s.as_str())));
        }
        best_lex = best_lex.min(t0.elapsed().as_secs_f64());
    }
    println!(
        "  lex      : {:>7.2} ms ({:>7.1} MiB/s) | allocs={:<7} reallocs={:<7} total={:>6.2} MiB ({:.2}x src) peak={:>6.2} KiB",
        best_lex * 1e3,
        mib / best_lex,
        lex_alloc.allocs,
        lex_alloc.reallocs,
        lex_alloc.total_bytes as f64 / (1024.0 * 1024.0),
        lex_alloc.total_bytes as f64 / total_bytes.max(1) as f64,
        lex_alloc.peak_live_bytes as f64 / 1024.0,
    );

    // 2. Parser allocation + best-of-N timing
    let (_, parse_alloc) = measure_allocs(|| {
        for s in sources {
            black_box(parse(s));
        }
    });
    let mut best_parse = f64::MAX;
    for _ in 0..iters {
        let t0 = Instant::now();
        for s in sources {
            black_box(parse(black_box(s.as_str())));
        }
        best_parse = best_parse.min(t0.elapsed().as_secs_f64());
    }
    println!(
        "  parse    : {:>7.2} ms ({:>7.1} MiB/s) | allocs={:<7} reallocs={:<7} total={:>6.2} MiB ({:.2}x src) peak={:>6.2} KiB",
        best_parse * 1e3,
        mib / best_parse,
        parse_alloc.allocs,
        parse_alloc.reallocs,
        parse_alloc.total_bytes as f64 / (1024.0 * 1024.0),
        parse_alloc.total_bytes as f64 / total_bytes.max(1) as f64,
        parse_alloc.peak_live_bytes as f64 / 1024.0,
    );

    // 3. Sema_fn on pre-parsed trees: allocation + best-of-N timing
    let trees: Vec<SyntaxTree<'_>> = sources.iter().map(|s| parse(s)).collect();
    let ((fn_count, hit_count), sema_alloc) = measure_allocs(|| {
        let (mut f, mut h) = (0, 0);
        for t in &trees {
            let (df, dh) = run_sema_on_tree(t);
            f += df;
            h += dh;
        }
        (f, h)
    });
    let mut best_sema = f64::MAX;
    for _ in 0..iters {
        let t0 = Instant::now();
        for t in &trees {
            black_box(run_sema_on_tree(black_box(t)));
        }
        best_sema = best_sema.min(t0.elapsed().as_secs_f64());
    }
    let ns_per_fn = (best_sema * 1e9) / fn_count.max(1) as f64;
    let allocs_per_fn = sema_alloc.allocs as f64 / fn_count.max(1) as f64;
    let bytes_per_fn = sema_alloc.total_bytes as f64 / fn_count.max(1) as f64;
    println!(
        "  sema_fn  : {:>7.2} ms ({:>7.1} ns/fn, {fn_count} fns, {hit_count} hits) | allocs={:<7} ({allocs_per_fn:.2}/fn) reallocs={:<6} total={:>6.2} MiB ({bytes_per_fn:.1} B/fn) peak={:>5.1} KiB",
        best_sema * 1e3,
        ns_per_fn,
        sema_alloc.allocs,
        sema_alloc.reallocs,
        sema_alloc.total_bytes as f64 / (1024.0 * 1024.0),
        sema_alloc.peak_live_bytes as f64 / 1024.0,
    );

    // 4. Zero-alloc buffer-reusing `*_into` pass
    let mut p_buf: Vec<ParamInfo<'_>> = Vec::with_capacity(16);
    let mut u_buf = ParamUses { uses: Vec::with_capacity(64), macro_uses: Vec::with_capacity(32) };
    let mut cf_buf: Vec<NodeId> = Vec::with_capacity(32);
    let mut run_into = || {
        let mut h = 0usize;
        for t in &trees {
            for id in t.descendants(t.root()) {
                if t.kind(id) != K::Fn {
                    continue;
                }
                params_into(t, id, &mut p_buf);
                for p in &p_buf {
                    for &b in &p.bindings {
                        param_uses_into(t, id, b, &mut u_buf);
                        h += u_buf.uses.len() + u_buf.macro_uses.len();
                    }
                }
                own_returns_into(t, id, &mut cf_buf);
                h += cf_buf.len();
                own_try_ops_into(t, id, &mut cf_buf);
                h += cf_buf.len();
                if tail_expr(t, id).is_some() {
                    h += 1;
                }
            }
        }
        h
    };
    let _ = run_into(); // warm reusable scratch buffers once
    let (reuse_hits, reuse_alloc) = measure_allocs(&mut run_into);
    assert_eq!(reuse_hits, hit_count);
    let mut best_reuse = f64::MAX;
    for _ in 0..iters {
        let t0 = Instant::now();
        black_box(run_into());
        best_reuse = best_reuse.min(t0.elapsed().as_secs_f64());
    }
    println!(
        "  sema_into: {:>7.2} ms ({:>7.1} ns/fn, scratch reuse)      | allocs={:<7} reallocs={:<6} total={:>6.2} KiB peak={:>5.1} KiB",
        best_reuse * 1e3,
        (best_reuse * 1e9) / fn_count.max(1) as f64,
        reuse_alloc.allocs,
        reuse_alloc.reallocs,
        reuse_alloc.total_bytes as f64 / 1024.0,
        reuse_alloc.peak_live_bytes as f64 / 1024.0,
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--profile") {
        let mode = args.get(1).map_or("all", String::as_str);
        let secs: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(4);
        let dirs: Vec<PathBuf> = if args.len() > 3 { args[3..].iter().map(PathBuf::from).collect() } else { vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")] };
        let sources = load_sources(&dirs);
        run_profile_loop(mode, secs, &sources);
        return;
    }

    let ws_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let ws_sources = load_sources(&[ws_root]);
    bench_corpus("rust-intelligence", &ws_sources, 7);

    for arg in &args {
        let p = PathBuf::from(arg);
        if p.exists() {
            let name = p.file_name().map_or_else(|| arg.clone(), |n| n.to_string_lossy().into_owned());
            let srcs = load_sources(&[p]);
            if !srcs.is_empty() {
                bench_corpus(&name, &srcs, 5);
            }
        }
    }
}
