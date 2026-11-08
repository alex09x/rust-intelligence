/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Single-dirty-file incremental overlay benchmark (`CrateSnapshot::with_dirty_files`, Step 2.3)
//! measuring `ns/edit`, amortized `ns/file`, and heap allocations across workspace + `$RIDX_BENCH_REPOS`.

use ridx_syntax::name_res::{CrateId, CrateSnapshot, Edition, FileId, GlobalDefId, ItemKind, MemItemDb, MemSourceDb, OverlaySourceDb, OwnedDef, OwnedUse, SourceDb, VisKind};
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
                if s.enabled && new_size > layout.size() {
                    s.total_bytes += (new_size - layout.size()) as u64;
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
        let opt = GlobalDefId::new(k, 10 + idx as u32);
        db.add_module_def(prel, OwnedDef { id: opt, name: "Option".into(), kind: ItemKind::Enum, vis: VisKind::Public, has_value_ctor: false });
        db.add_module_use(
            root,
            OwnedUse {
                name: "option".into(),
                type_def: opt,
                value_def: GlobalDefId::NONE,
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
    let mut setups = Vec::new();
    let mut total_files = 0usize;

    for &(cid, _, rf) in &crates {
        let snap = CrateSnapshot::build_clean_cached(&db, &sources, cid, rf, |fid| parsed.get(fid.0 as usize));
        total_files += snap.mod_tree.modules.iter().filter(|m| !m.is_inline() && !m.is_unresolved()).count();
        // Pick the non-root leaf file with the largest byte length (or root_file for single-file crates):
        let dirty_fid = snap
            .mod_tree
            .modules
            .iter()
            .filter(|m| m.file.is_some() && !m.is_inline() && !m.is_unresolved())
            .max_by_key(|m| (m.id != ridx_syntax::name_res::ModuleId::ROOT, sources.file_text(m.file).len()))
            .map_or(rf, |m| m.file);
        let mut edited = String::from(sources.file_text(dirty_fid));
        edited.push_str("\npub struct __RidxOverlayProbe;\nimpl __RidxOverlayProbe { pub fn ping(&self) -> usize { 1 } }\n");
        let mut ov = OverlaySourceDb::new(&sources, sources.len() as u32);
        ov.set_file_override(dirty_fid, edited);
        setups.push((snap, ov, dirty_fid));
    }

    let edited_trees: Vec<SyntaxTree<'_>> = setups.iter().map(|(_, ov, df)| parse(ov.file_text(*df))).collect();

    // Measure 1-dirty-file overlay update across all crates in this repo:
    let mut dirty_impls = 0usize;
    let mut dirty_assocs = 0usize;
    let (_, allocs) = measure_allocs(|| {
        for (idx, (snap, ov, df)) in setups.iter().enumerate() {
            let dirty_st = &edited_trees[idx];
            let res = snap.with_dirty_files(&db, ov, ov.dirty_files(), |fid| if fid == *df { Some(dirty_st) } else { parsed.get(fid.0 as usize) });
            dirty_impls += res.1.local_impls.impls.len();
            dirty_assocs += res.1.local_impls.assoc_items.len();
            black_box(res);
        }
    });

    let rounds = if total_files > 400 { 7usize } else { 15usize };
    let mut best = Duration::MAX;
    let mut fast_hits = 0usize;
    for r in 0..rounds {
        let t0 = Instant::now();
        for (idx, (snap, ov, df)) in setups.iter().enumerate() {
            let dirty_st = &edited_trees[idx];
            let (rebuilt_mt, dm) = snap.with_dirty_files(&db, ov, ov.dirty_files(), |fid| if fid == *df { Some(dirty_st) } else { parsed.get(fid.0 as usize) });
            if r == 0 && rebuilt_mt.is_none() {
                fast_hits += 1;
            }
            black_box(dm);
        }
        best = best.min(t0.elapsed());
    }

    let edits = setups.len().max(1) as f64;
    let files_f = total_files.max(1) as f64;
    let label = root.file_name().and_then(|s| s.to_str()).unwrap_or("repo");
    println!(
        "{label:<16} crates={:>3} files={:>4} dirty_impls={:>4} dirty_assocs={:>4} fast={:>3}/{:<3} | {:>8.1} ns/edit ({:>6.1} ns/file, {:>5.2} ms) | allocs/edit={:>5.1} ({:>4.1} allocs/file) bytes/edit={:>8.1}",
        setups.len(),
        total_files,
        dirty_impls,
        dirty_assocs,
        fast_hits,
        setups.len(),
        best.as_nanos() as f64 / edits,
        best.as_nanos() as f64 / files_f,
        best.as_secs_f64() * 1000.0,
        allocs.allocs as f64 / edits,
        allocs.allocs as f64 / files_f,
        allocs.total_bytes as f64 / edits,
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
