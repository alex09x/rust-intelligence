/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Throughput, per-file latency (`ns/file`), and heap-allocation benchmark for `ModuleTree` (2.1).
//!
//! Usage:
//! - `mod_tree_bench [repo_dir ...]` (also scans subdirectories of `$RIDX_BENCH_REPOS` if set).

use ridx_syntax::name_res::{CrateId, Edition, FileId, MemItemDb, MemSourceDb, ModuleTree, SourceDb};
use ridx_syntax::{parse, SyntaxTree};
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
    let out = f();
    let stats = STATS.with(|c| {
        let mut s = c.get();
        s.enabled = false;
        c.set(s);
        s
    });
    (out, stats)
}

struct CrateEntry {
    id: CrateId,
    root_file: FileId,
}

struct CorpusFixture {
    label: String,
    db: MemItemDb,
    sources: MemSourceDb,
    crates: Vec<CrateEntry>,
    total_rs_files: usize,
}

fn collect_repo(dir: &Path, out_rs: &mut Vec<PathBuf>, out_manifests: &mut Vec<PathBuf>) {
    if dir.parent().is_none() {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_symlink() {
            continue;
        }
        let p = e.path();
        let name = e.file_name();
        let s = name.to_string_lossy();
        if ft.is_dir() {
            if matches!(s.as_ref(), "target" | ".git" | "node_modules" | "fixtures" | "proc" | "sys" | "dev") {
                continue;
            }
            collect_repo(&p, out_rs, out_manifests);
        } else if ft.is_file() && s == "Cargo.toml" {
            out_manifests.push(p);
        } else if ft.is_file() && p.extension().and_then(|x| x.to_str()) == Some("rs") {
            out_rs.push(p);
        }
    }
}

fn parse_manifest_meta(toml_text: &str) -> (String, Edition, Option<String>, Vec<String>) {
    let mut section = "";
    let mut name = String::from("crate");
    let mut edition = Edition::Edition2021;
    let mut lib_path: Option<String> = None;
    let mut features = Vec::new();
    for raw in toml_text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = &line[1..line.len() - 1];
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let k = k.trim().trim_matches('"');
        let v = v.trim().trim_matches('"');
        match section {
            "package" if k == "name" => name = v.replace('-', "_"),
            "package" if k == "edition" => {
                edition = match v {
                    "2015" => Edition::Edition2015,
                    "2018" => Edition::Edition2018,
                    "2024" => Edition::Edition2024,
                    _ => Edition::Edition2021,
                };
            }
            "lib" if k == "path" => lib_path = Some(v.to_string()),
            "features" => features.push(k.to_string()),
            _ => {}
        }
    }
    (name, edition, lib_path, features)
}

fn load_corpus(root: &Path) -> Option<CorpusFixture> {
    let mut rs_paths = Vec::new();
    let mut manifests = Vec::new();
    collect_repo(root, &mut rs_paths, &mut manifests);
    if rs_paths.is_empty() {
        return None;
    }
    let mut sources = MemSourceDb::new();
    for p in &rs_paths {
        let Ok(text) = std::fs::read_to_string(p) else {
            continue;
        };
        let rel = p.strip_prefix(root).unwrap_or(p).to_string_lossy().replace('\\', "/");
        sources.add_file(&rel, &text);
    }

    let mut db = MemItemDb::new();
    let mut crates = Vec::new();
    for mp in &manifests {
        let Ok(toml_text) = std::fs::read_to_string(mp) else {
            continue;
        };
        if !toml_text.contains("[package]") {
            continue;
        }
        let (pkg_name, ed, explicit_lib, feats) = parse_manifest_meta(&toml_text);
        let crate_dir = mp.parent().unwrap_or(root);
        let rel_dir = crate_dir.strip_prefix(root).unwrap_or(Path::new("")).to_string_lossy().replace('\\', "/");
        let join_rel = |sub: &str| {
            if rel_dir.is_empty() {
                sub.to_string()
            } else {
                format!("{rel_dir}/{sub}")
            }
        };
        let mut roots = Vec::new();
        if let Some(lp) = explicit_lib {
            if let Some(fid) = sources.resolve_path(&join_rel(&lp)) {
                roots.push(fid);
            }
        } else if let Some(fid) = sources.resolve_path(&join_rel("src/lib.rs")) {
            roots.push(fid);
        }
        if let Some(fid) = sources.resolve_path(&join_rel("src/main.rs")) {
            if !roots.contains(&fid) {
                roots.push(fid);
            }
        }
        for root_file in roots {
            let cid = db.add_crate(&pkg_name, ed);
            for flag in ["unix", "test"] {
                db.add_cfg_flag(cid, flag);
            }
            for (k, v) in [
                ("target_os", "linux"),
                ("target_family", "unix"),
                ("target_arch", "x86_64"),
                ("target_pointer_width", "64"),
                ("target_endian", "little"),
                ("target_has_atomic", "64"),
                ("target_has_atomic", "ptr"),
            ] {
                db.add_cfg_kv(cid, k, v);
            }
            for f in &feats {
                db.add_cfg_kv(cid, "feature", f.as_str());
            }
            crates.push(CrateEntry { id: cid, root_file });
        }
    }
    if crates.is_empty() {
        return None;
    }
    let label = root.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "repo".into());
    let total_rs_files = sources.len();
    Some(CorpusFixture { label, db, sources, crates, total_rs_files })
}

fn bench_corpus(fx: &CorpusFixture) {
    let parsed: Vec<SyntaxTree<'_>> = (0..fx.sources.len()).map(|i| parse(fx.sources.file_text(FileId(i as u32)))).collect();

    let ((mod_count, reached_files, unresolved), alloc_cached) = measure_allocs(|| {
        let mut mods = 0usize;
        let mut reached = 0usize;
        let mut unres = 0usize;
        for c in &fx.crates {
            let t = ModuleTree::build_cached(&fx.db, &fx.sources, c.id, c.root_file, |fid| parsed.get(fid.0 as usize));
            for m in &t.modules {
                if m.is_unresolved() {
                    unres += 1;
                } else if !m.is_inline() {
                    reached += 1;
                }
            }
            mods += t.modules.len();
            black_box(&t);
        }
        (mods, reached, unres)
    });

    let mut best_cached = Duration::MAX;
    let iters = if reached_files > 1500 { 7 } else { 15 };
    for _ in 0..iters {
        let t0 = Instant::now();
        for c in &fx.crates {
            let t = ModuleTree::build_cached(&fx.db, &fx.sources, c.id, c.root_file, |fid| parsed.get(fid.0 as usize));
            black_box(t);
        }
        best_cached = best_cached.min(t0.elapsed());
    }

    let mut best_cold = Duration::MAX;
    let cold_iters = if reached_files > 1500 { 2 } else { 4 };
    for _ in 0..cold_iters {
        let t0 = Instant::now();
        for c in &fx.crates {
            let t = ModuleTree::build(&fx.db, &fx.sources, c.id, c.root_file);
            black_box(t);
        }
        best_cold = best_cold.min(t0.elapsed());
    }

    let denom = reached_files.max(1) as f64;
    let cached_ns_per_file = best_cached.as_nanos() as f64 / denom;
    let cold_us_per_file = best_cold.as_nanos() as f64 / 1000.0 / denom;
    let allocs_per_file = (alloc_cached.allocs + alloc_cached.reallocs) as f64 / denom;
    let bytes_per_file = alloc_cached.total_bytes as f64 / denom;

    println!(
        "{:<18} crates={:>4} rs_all={:>5} reached={:>5} mods={:>5} unres={:>3} | tree_only={:>7.1} ns/file ({:>6.2} ms) | parse+tree={:>6.2} us/file | allocs/file={:>4.2} bytes/file={:>6.1}",
        fx.label,
        fx.crates.len(),
        fx.total_rs_files,
        reached_files,
        mod_count,
        unresolved,
        cached_ns_per_file,
        best_cached.as_secs_f64() * 1000.0,
        cold_us_per_file,
        allocs_per_file,
        bytes_per_file,
    );
}

fn main() {
    let mut dirs: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    if dirs.is_empty() {
        if let Ok(ws) = std::env::var("RIDX_WS_SRC") {
            let p = PathBuf::from(ws);
            if p.parent().is_some() && p.join("Cargo.toml").is_file() {
                dirs.push(p);
            }
        } else if let Some(w) = Path::new(env!("CARGO_MANIFEST_DIR")).parent().and_then(Path::parent) {
            if w.parent().is_some() && w.join("Cargo.toml").is_file() {
                dirs.push(w.to_path_buf());
            }
        }
        if dirs.is_empty() {
            dirs.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")));
        }
        if let Ok(bench_root) = std::env::var("RIDX_BENCH_REPOS") {
            if let Ok(rd) = std::fs::read_dir(bench_root) {
                let mut sub: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
                sub.sort();
                dirs.extend(sub);
            }
        }
    }
    for d in dirs {
        if let Some(fx) = load_corpus(&d) {
            bench_corpus(&fx);
        }
    }
}
