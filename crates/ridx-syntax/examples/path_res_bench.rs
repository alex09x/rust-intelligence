/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! File path-resolution benchmark (`CrateSnapshot::resolve_file_paths_into`, Step 2.4)
//! measuring `ns/file`, total resolved references/locals, and heap allocations across
//! workspace + `$RIDX_BENCH_REPOS`.

use ridx_syntax::name_res::{CrateId, CrateSnapshot, Edition, FileId, FileNameRes, GlobalDefId, ItemKind, MemItemDb, MemSourceDb, OwnedDef, OwnedUse, SourceDb, VisKind};
use ridx_syntax::{parse, SyntaxTree};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

struct CountingAlloc;

#[derive(Clone, Copy, Default)]
struct AllocStats {
    enabled: bool,
    allocs: u64,
    total_bytes: u64,
}

thread_local! {
    static STATS: Cell<AllocStats> = const { Cell::new(AllocStats { enabled: false, allocs: 0, total_bytes: 0 }) };
}

// SAFETY: Delegates directly to `std::alloc::System` while counting heap allocations on this thread.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            let _ = STATS.try_with(|c| {
                let mut s = c.get();
                if s.enabled {
                    s.allocs += 1;
                    s.total_bytes += layout.size() as u64;
                    c.set(s);
                }
            });
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let out = unsafe { System.realloc(ptr, layout, new_size) };
        if !out.is_null() {
            let _ = STATS.try_with(|c| {
                let mut s = c.get();
                if s.enabled {
                    s.allocs += 1;
                    if new_size > layout.size() {
                        s.total_bytes += (new_size - layout.size()) as u64;
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
    STATS.with(|c| c.set(AllocStats { enabled: true, allocs: 0, total_bytes: 0 }));
    let out = f();
    let stats = STATS.with(|c| {
        let mut s = c.get();
        s.enabled = false;
        c.set(s);
        s
    });
    (out, stats)
}

fn seed_minimal_sysroot(db: &mut MemItemDb) -> (CrateId, CrateId, CrateId) {
    let core_k = db.add_crate("core", Edition::Edition2021);
    let alloc_k = db.add_crate("alloc", Edition::Edition2021);
    let std_k = db.add_crate("std", Edition::Edition2021);
    for (idx, k) in [core_k, alloc_k, std_k].into_iter().enumerate() {
        let root = GlobalDefId::new(k, 0);
        let prel = GlobalDefId::new(k, 1);
        db.set_crate_root_def(k, root);
        db.set_prelude_module(k, prel);
        let opt = GlobalDefId::new(k, 10 + idx as u32 * 4);
        let some_d = GlobalDefId::new(k, 11 + idx as u32 * 4);
        let none_d = GlobalDefId::new(k, 12 + idx as u32 * 4);
        db.add_module_def(prel, OwnedDef { id: opt, name: "Option".into(), kind: ItemKind::Enum, vis: VisKind::Public, has_value_ctor: false });
        db.add_assoc_item(opt, OwnedDef { id: some_d, name: "Some".into(), kind: ItemKind::Variant, vis: VisKind::Public, has_value_ctor: true });
        db.add_assoc_item(opt, OwnedDef { id: none_d, name: "None".into(), kind: ItemKind::Variant, vis: VisKind::Public, has_value_ctor: true });
        for (nm, vd) in [("Some", some_d), ("None", none_d)] {
            db.add_module_use(
                prel,
                OwnedUse {
                    name: nm.into(),
                    type_def: vd,
                    value_def: vd,
                    macro_def: GlobalDefId::NONE,
                    vis: VisKind::Public,
                    is_glob: false,
                    is_reexport: true,
                    is_prelude: false,
                    is_underscore: false,
                    prim_ty_code: 0,
                },
            );
        }
    }
    (core_k, alloc_k, std_k)
}

fn collect_repo(dir: &Path, out_rs: &mut Vec<PathBuf>, out_manifests: &mut Vec<PathBuf>) {
    if dir.parent().is_none() {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_symlink() {
            continue;
        }
        let p = e.path();
        let s = e.file_name().to_string_lossy().into_owned();
        if ft.is_dir() {
            if !matches!(s.as_str(), "target" | ".git" | "node_modules" | "fixtures" | "proc" | "sys" | "dev") {
                collect_repo(&p, out_rs, out_manifests);
            }
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
    let mut lib_path = None;
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
        let (k, v) = (k.trim().trim_matches('"'), v.trim().trim_matches('"'));
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

fn bench_repo(root: &Path) {
    let mut rs_paths = Vec::new();
    let mut manifests = Vec::new();
    collect_repo(root, &mut rs_paths, &mut manifests);
    if rs_paths.is_empty() {
        return;
    }
    let mut sources = MemSourceDb::new();
    for p in &rs_paths {
        if let Ok(text) = std::fs::read_to_string(p) {
            let rel = p.strip_prefix(root).unwrap_or(p).to_string_lossy().replace('\\', "/");
            sources.add_file(&rel, text);
        }
    }
    let mut db = MemItemDb::new();
    let (core_k, alloc_k, std_k) = seed_minimal_sysroot(&mut db);
    let mut crates = Vec::new();
    for mp in &manifests {
        let Ok(t) = std::fs::read_to_string(mp) else {
            continue;
        };
        if !t.contains("[package]") {
            continue;
        }
        let (pkg, ed, explicit_lib, feats) = parse_manifest_meta(&t);
        let cdir = mp.parent().unwrap_or(root);
        let rel = cdir.strip_prefix(root).unwrap_or(Path::new("")).to_string_lossy().replace('\\', "/");
        let join_r = |s: &str| {
            if rel.is_empty() {
                s.to_string()
            } else {
                format!("{rel}/{s}")
            }
        };
        let lib_fid = explicit_lib
            .as_deref()
            .and_then(|lp| sources.resolve_path(&join_r(lp)))
            .or_else(|| sources.resolve_path(&join_r("src/lib.rs")))
            .or_else(|| sources.resolve_path(&join_r("src/main.rs")));
        let Some(rf) = lib_fid else { continue };
        let cid = db.add_crate(&pkg, ed);
        db.set_crate_root_def(cid, GlobalDefId::new(cid, 0));
        for ext in [("core", core_k), ("alloc", alloc_k), ("std", std_k)] {
            db.add_extern_crate(cid, ext.0, ext.1);
        }
        db.add_cfg_flag(cid, "unix");
        for f in &feats {
            db.add_cfg_kv(cid, "feature", f.as_str());
        }
        crates.push((cid, pkg, rf));
    }
    if crates.is_empty() {
        return;
    }
    for i in 0..crates.len() {
        for j in 0..crates.len() {
            if i != j {
                db.add_extern_crate(crates[i].0, crates[j].1.clone(), crates[j].0);
            }
        }
    }

    let parsed: Vec<SyntaxTree<'_>> = (0..sources.len()).map(|i| parse(sources.file_text(FileId(i as u32)))).collect();
    let mut snapshots = Vec::with_capacity(crates.len());
    let mut crate_files: Vec<Vec<FileId>> = Vec::with_capacity(crates.len());
    let mut total_files = 0usize;
    for &(cid, _, rf) in &crates {
        let snap = CrateSnapshot::build_clean_cached(&db, &sources, cid, rf, |fid| parsed.get(fid.0 as usize));
        let mut fids = Vec::new();
        for m in &snap.mod_tree.modules {
            if m.file.is_some() && !m.is_inline() && !m.is_unresolved() && !fids.contains(&m.file) {
                fids.push(m.file);
            }
        }
        total_files += fids.len();
        crate_files.push(fids);
        snapshots.push(snap);
    }
    if total_files == 0 {
        return;
    }

    // Warm up `ExtScopeCache` and reusable `FileNameRes` buffer once so steady-state per-file cost is measured cleanly:
    let mut out_buf = FileNameRes::new();
    for (ci, snap) in snapshots.iter().enumerate() {
        for &fid in &crate_files[ci] {
            snap.resolve_file_paths_into(&db, fid, &parsed[fid.0 as usize], &mut out_buf);
        }
    }

    let mut total_refs = 0usize;
    let mut total_locals = 0usize;
    let (_, allocs) = measure_allocs(|| {
        for (ci, snap) in snapshots.iter().enumerate() {
            for &fid in &crate_files[ci] {
                snap.resolve_file_paths_into(&db, fid, &parsed[fid.0 as usize], &mut out_buf);
                total_refs += out_buf.refs.len();
                total_locals += out_buf.locals.len();
                black_box(&out_buf);
            }
        }
    });

    let rounds = if total_files > 400 { 5usize } else { 11usize };
    let mut best = Duration::MAX;
    for _ in 0..rounds {
        let t0 = Instant::now();
        for (ci, snap) in snapshots.iter().enumerate() {
            for &fid in &crate_files[ci] {
                snap.resolve_file_paths_into(&db, fid, &parsed[fid.0 as usize], &mut out_buf);
                black_box(out_buf.refs.len());
            }
        }
        best = best.min(t0.elapsed());
    }

    let files_f = total_files as f64;
    let label = root.file_name().and_then(|s| s.to_str()).unwrap_or("repo");
    println!(
        "{label:<16} crates={:>3} files={:>4} refs={:>7} locals={:>6} | {:>8.1} ns/file ({:>5.2} ms total) | allocs/file={:>4.1} bytes/file={:>7.1}",
        snapshots.len(),
        total_files,
        total_refs,
        total_locals,
        best.as_nanos() as f64 / files_f,
        best.as_secs_f64() * 1000.0,
        allocs.allocs as f64 / files_f,
        allocs.total_bytes as f64 / files_f,
    );
}

fn default_ws_root() -> PathBuf {
    if let Ok(ws) = std::env::var("RIDX_WS_SRC") {
        let p = PathBuf::from(ws);
        if p.parent().is_some() && p.join("Cargo.toml").is_file() {
            return p;
        }
    }
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    if let Some(w) = manifest_dir.parent().and_then(Path::parent) {
        if w.parent().is_some() && w.join("Cargo.toml").is_file() {
            return w.to_path_buf();
        }
    }
    manifest_dir.to_path_buf()
}

fn main() {
    let mut dirs: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    if dirs.is_empty() {
        dirs.push(default_ws_root());
        if let Ok(dir) = std::env::var("RIDX_BENCH_REPOS") {
            if let Ok(rd) = std::fs::read_dir(dir) {
                let mut sub: Vec<_> = rd.filter_map(|e| e.ok().map(|x| x.path())).filter(|p| p.is_dir()).collect();
                sub.sort();
                dirs.extend(sub);
            }
        }
    }
    for p in dirs {
        bench_repo(&p);
    }
}
