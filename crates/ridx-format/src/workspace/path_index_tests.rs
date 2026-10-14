/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::provenance::{file_entry_matches, normalize_path};
use super::WorkspaceIndex;
use crate::format::DefKind;
use crate::writer::{IndexWriter, RawDefinition};
use std::collections::HashMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Reference oracle reproducing the legacy `shard_mtime_for_file` implementation verbatim.
pub(crate) fn shard_mtime_for_file_oracle(
    ws: &WorkspaceIndex,
    file_path: &str,
) -> Option<std::time::SystemTime> {
    for idx in ws.candidate_reader_indices_for_file(file_path) {
        if let Some(target_file) = ws.target_file_for_reader(idx, file_path) {
            if ws.readers[idx].find_file_id(target_file.as_ref()).is_some() {
                return ws.mtimes.get(idx).copied().flatten();
            }
            if ws.reader_file_hashes.get(idx).is_some_and(|h| h.contains_key(target_file.as_ref())) {
                return ws.mtimes.get(idx).copied().flatten();
            }
        }
        if ws.reader_file_hashes.get(idx).is_some_and(|h| h.contains_key(file_path)) {
            return ws.mtimes.get(idx).copied().flatten();
        }
    }

    let p = Path::new(file_path);
    let (ws_abs, req_canon, ws_canon, candidate_indices) = legacy_candidates(ws, p);
    for &i in &candidate_indices {
        let reader = &ws.readers[i];
        let origin = reader_origin(ws, i);
        for fid in 0..reader.files_count() {
            let entry_str = reader.get_file(fid);
            if entry_str.is_empty() {
                continue;
            }
            if file_entry_matches(entry_str, origin, p, req_canon.as_deref())
                || ws_abs.as_ref().is_some_and(|a| file_entry_matches(entry_str, origin, a, ws_canon.as_deref()))
            {
                return ws.mtimes.get(i).copied().flatten();
            }
        }
    }
    for &i in &candidate_indices {
        let origin = reader_origin(ws, i);
        if let Some(map) = ws.reader_file_hashes.get(i) {
            for k in map.keys() {
                if file_entry_matches(k, origin, p, req_canon.as_deref())
                    || ws_abs.as_ref().is_some_and(|a| file_entry_matches(k, origin, a, ws_canon.as_deref()))
                {
                    return ws.mtimes.get(i).copied().flatten();
                }
            }
        }
    }
    None
}

/// Reference oracle reproducing the legacy `source_hash_for_file` implementation verbatim.
pub(crate) fn source_hash_for_file_oracle(ws: &WorkspaceIndex, file_path: &str) -> Option<u64> {
    for idx in ws.candidate_reader_indices_for_file(file_path) {
        if let Some(target_file) = ws.target_file_for_reader(idx, file_path) {
            if let Some(fid) = ws.readers[idx].find_file_id(target_file.as_ref()) {
                let h = ws.readers[idx].file_source_hash(fid);
                if h != 0 {
                    return Some(h);
                }
            }
            if let Some(&h) = ws.reader_file_hashes.get(idx).and_then(|m| m.get(target_file.as_ref())) {
                if h != 0 {
                    return Some(h);
                }
            }
        }
        if let Some(&h) = ws.reader_file_hashes.get(idx).and_then(|m| m.get(file_path)) {
            if h != 0 {
                return Some(h);
            }
        }
    }

    let p = Path::new(file_path);
    let (ws_abs, req_canon, ws_canon, candidate_indices) = legacy_candidates(ws, p);
    for &i in &candidate_indices {
        let reader = &ws.readers[i];
        let origin = reader_origin(ws, i);
        for fid in 0..reader.files_count() {
            let entry_str = reader.get_file(fid);
            let h = reader.file_source_hash(fid);
            if h == 0 || entry_str.is_empty() {
                continue;
            }
            if file_entry_matches(entry_str, origin, p, req_canon.as_deref())
                || ws_abs.as_ref().is_some_and(|a| file_entry_matches(entry_str, origin, a, ws_canon.as_deref()))
            {
                return Some(h);
            }
        }
    }
    for &i in &candidate_indices {
        let origin = reader_origin(ws, i);
        if let Some(map) = ws.reader_file_hashes.get(i) {
            for (k, &h) in map {
                if h == 0 || k.is_empty() {
                    continue;
                }
                if file_entry_matches(k, origin, p, req_canon.as_deref())
                    || ws_abs.as_ref().is_some_and(|a| file_entry_matches(k, origin, a, ws_canon.as_deref()))
                {
                    return Some(h);
                }
            }
        }
    }
    None
}

fn reader_origin(ws: &WorkspaceIndex, i: usize) -> Option<&Path> {
    ws.source_roots
        .get(i)
        .and_then(|r| r.as_deref())
        .or(ws.workspace_root.as_deref())
        .or(ws.root_dir.as_deref())
}

fn legacy_candidates(
    ws: &WorkspaceIndex,
    p: &Path,
) -> (Option<PathBuf>, Option<PathBuf>, Option<PathBuf>, Vec<usize>) {
    let explicit_origin = ws
        .workspace_root
        .as_deref()
        .or(ws.root_dir.as_deref())
        .or_else(|| (ws.readers.len() == 1).then(|| ws.source_roots.first()?.as_deref())?);

    let (ws_abs, req_canon) = if p.is_absolute() {
        (None, p.canonicalize().ok())
    } else if let Some(origin) = explicit_origin {
        let abs = origin.join(p);
        let canon = abs.canonicalize().ok();
        (Some(abs), canon)
    } else {
        (None, None)
    };
    let ws_canon = req_canon.clone();

    let mut candidate_indices: Vec<usize> = Vec::with_capacity(ws.readers.len());
    if let Some(ref c_p) = req_canon {
        for i in 0..ws.readers.len() {
            if let Some(Some(ref root)) = ws.source_roots.get(i) {
                if root.canonicalize().is_ok_and(|cr| c_p.starts_with(&cr)) && !candidate_indices.contains(&i) {
                    candidate_indices.push(i);
                }
            }
        }
    }
    if let Some(ref abs) = ws_abs {
        let norm_abs = normalize_path(abs);
        for i in 0..ws.readers.len() {
            if let Some(Some(ref root)) = ws.source_roots.get(i) {
                if norm_abs.starts_with(normalize_path(root)) && !candidate_indices.contains(&i) {
                    candidate_indices.push(i);
                }
            }
        }
    } else if p.is_absolute() {
        let norm_p = normalize_path(p);
        for i in 0..ws.readers.len() {
            if let Some(Some(ref root)) = ws.source_roots.get(i) {
                if norm_p.starts_with(normalize_path(root)) && !candidate_indices.contains(&i) {
                    candidate_indices.push(i);
                }
            }
        }
    }
    for i in 0..ws.readers.len() {
        let is_dep = ws.reader_is_dependency.get(i).copied().unwrap_or(false)
            || ws.source_roots.get(i).and_then(|r| r.as_ref()).is_some();
        if !is_dep && !candidate_indices.contains(&i) {
            candidate_indices.push(i);
        }
    }
    for i in 0..ws.readers.len() {
        if !candidate_indices.contains(&i) {
            candidate_indices.push(i);
        }
    }
    (ws_abs, req_canon, ws_canon, candidate_indices)
}

#[test]
fn test_path_index_oracle_equivalence_multi_reader_and_symlinks() {
    let temp = TempDir::new().unwrap();
    let ws_root = temp.path().join("ws");
    let crate_a = ws_root.join("crates/crate_a");
    let dep_ext = temp.path().join("external_dep");
    let nested_dir = dep_ext.join("nested");
    for d in [crate_a.join("src"), dep_ext.join("src"), nested_dir.clone()] {
        fs::create_dir_all(d).unwrap();
    }
    fs::write(crate_a.join("src/lib.rs"), "pub fn a() {}\n").unwrap();
    fs::write(dep_ext.join("src/lib.rs"), "pub fn d() {}\n").unwrap();
    fs::write(ws_root.join("file.rs"), "pub fn root_f() {}\n").unwrap();
    fs::write(dep_ext.join("file.rs"), "pub fn ext_f() {}\n").unwrap();

    #[cfg(unix)]
    {
        let _ = std::os::unix::fs::symlink(&nested_dir, ws_root.join("shortcut"));
        let _ = std::os::unix::fs::symlink(&crate_a, ws_root.join("link_crate_a"));
    }

    let var_dir = ws_root.join("target/ridx/variants/v1");
    let deps_dir = var_dir.join(".deps");
    fs::create_dir_all(&deps_dir).unwrap();

    let mut w_a = IndexWriter::new("crate_a", "0.1.0");
    w_a.set_unit_key("uk_a");
    let mut hashes_a = HashMap::new();
    hashes_a.insert("src/lib.rs".to_string(), 0xA001u64);
    for idx in 0..68 {
        let rel = format!("src/mod_{idx}.rs");
        fs::write(crate_a.join(&rel), "pub fn m() {}\n").unwrap();
        w_a.get_or_insert_file(&rel);
        hashes_a.insert(rel, 0x10000 + idx as u64);
    }
    let root_file_str = ws_root.join("file.rs").display().to_string();
    hashes_a.insert(root_file_str.clone(), 0xA002u64);
    w_a.set_file_hashes(hashes_a.clone());
    w_a.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "a".to_string(),
        path: "crate_a::a".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 10,
    });
    w_a.get_or_insert_file(&root_file_str);
    w_a.add_file_hash(&root_file_str, 0xA002u64);
    let ridx_a = var_dir.join("00_crate_a.ridx");
    w_a.write_to(&mut File::create(&ridx_a).unwrap()).unwrap();
    hashes_a.insert("src/sidecar_only.rs".to_string(), 0xA999u64);
    fs::write(
        deps_dir.join("00_crate_a.json"),
        serde_json::to_string(&serde_json::json!({
            "name": "crate_a", "version": "0.1.0", "unit_key": "uk_a",
            "object_path": ridx_a.display().to_string(),
            "source_root": crate_a.display().to_string(),
            "is_dependency": false, "file_hashes": hashes_a,
        })).unwrap(),
    ).unwrap();

    let mut w_d = IndexWriter::new("external_dep", "0.1.0");
    w_d.set_unit_key("uk_d");
    let ext_file_str = dep_ext.join("file.rs").display().to_string();
    w_d.get_or_insert_file("src/lib.rs");
    w_d.add_file_hash("src/lib.rs", 0xD001u64);
    w_d.get_or_insert_file(&ext_file_str);
    w_d.add_file_hash(&ext_file_str, 0xD002u64);
    let ridx_d = var_dir.join("01_external_dep.ridx");
    w_d.write_to(&mut File::create(&ridx_d).unwrap()).unwrap();
    fs::write(
        deps_dir.join("01_external_dep.json"),
        serde_json::to_string(&serde_json::json!({
            "name": "external_dep", "version": "0.1.0", "unit_key": "uk_d",
            "object_path": ridx_d.display().to_string(),
            "source_root": dep_ext.display().to_string(),
            "is_dependency": true,
            "file_hashes": { "src/lib.rs": 0xD001u64, ext_file_str: 0xD002u64 },
        })).unwrap(),
    ).unwrap();

    fs::write(
        ws_root.join("target/ridx/ridx_manifest.json"),
        serde_json::to_string(&serde_json::json!({
            "version": "2.0",
            "format_version": crate::format::CURRENT_FORMAT_VERSION,
            "extractor_version": crate::format::EXTRACTOR_VERSION,
            "active_variant_id": "v1", "variant_dir": "variants/v1",
            "workspace_root": ws_root.display().to_string(), "dependency_shards": []
        })).unwrap(),
    ).unwrap();

    let mut ws = WorkspaceIndex::open(&ws_root).unwrap();
    let queries = vec![
        "src/lib.rs".to_string(),
        "crates/crate_a/src/lib.rs".to_string(),
        "crates/crate_a/src/mod_42.rs".to_string(),
        "crates/crate_a/src/sidecar_only.rs".to_string(),
        "crates/crate_a/src/../src/mod_15.rs".to_string(),
        "link_crate_a/src/lib.rs".to_string(),
        "link_crate_a/src/mod_10.rs".to_string(),
        "shortcut/../file.rs".to_string(),
        "file.rs".to_string(),
        "examples/unindexed.rs".to_string(),
        "examples/lib.rs".to_string(),
        "/nonexistent/abs/path.rs".to_string(),
        crate_a.join("src/lib.rs").display().to_string(),
        crate_a.join("src/mod_67.rs").display().to_string(),
        crate_a.join("src/sidecar_only.rs").display().to_string(),
        ws_root.join("link_crate_a/src/mod_10.rs").display().to_string(),
        ws_root.join("shortcut/../file.rs").display().to_string(),
        dep_ext.join("src/lib.rs").display().to_string(),
        dep_ext.join("src/../src/lib.rs").display().to_string(),
    ];

    for q in &queries {
        assert_eq!(ws.source_hash_for_file(q), source_hash_for_file_oracle(&ws, q), "hash: {q}");
        assert_eq!(ws.shard_mtime_for_file(q), shard_mtime_for_file_oracle(&ws, q), "mtime: {q}");
    }
    ws.set_workspace_root(Some(crate_a));
    for q in &queries {
        assert_eq!(ws.source_hash_for_file(q), source_hash_for_file_oracle(&ws, q), "hash2: {q}");
        assert_eq!(ws.shard_mtime_for_file(q), shard_mtime_for_file_oracle(&ws, q), "mtime2: {q}");
    }
}

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let p = entry.path();
        let name = entry.file_name();
        if name == ".git" || name == "target" {
            continue;
        }
        if p.is_dir() {
            collect_rs_files(&p, out);
        } else if p.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(p);
        }
    }
}

#[test]
fn test_corpus_oracle_equivalence() {
    let Ok(dirs_var) = std::env::var("RIDX_CORPUS_ORACLE_DIRS") else { return };
    for repo_str in dirs_var.split(':').filter(|s| !s.is_empty()) {
        let repo = Path::new(repo_str);
        let ws = WorkspaceIndex::open(repo).expect("failed to open corpus WorkspaceIndex");
        let mut files = Vec::new();
        collect_rs_files(repo, &mut files);
        files.sort();
        let mut checked = 0usize;
        let mut divergences = 0usize;
        for abs_path in &files {
            let abs_q = abs_path.to_string_lossy();
            let rel_q = abs_path.strip_prefix(repo).ok().map(|r| r.to_string_lossy());
            for q in [Some(abs_q.as_ref()), rel_q.as_deref()].into_iter().flatten() {
                checked += 1;
                let h_new = ws.source_hash_for_file(q);
                let h_old = source_hash_for_file_oracle(&ws, q);
                let m_new = ws.shard_mtime_for_file(q);
                let m_old = shard_mtime_for_file_oracle(&ws, q);
                if h_new != h_old || m_new != m_old {
                    divergences += 1;
                    eprintln!("DIVERGENCE [{repo_str}] q={q}: hash={h_new:?}/{h_old:?} mtime={m_new:?}/{m_old:?}");
                }
            }
        }
        eprintln!("[CORPUS_ORACLE] repo={repo_str} rs_files={} queries_checked={checked} divergences={divergences}", files.len());
        assert_eq!(divergences, 0, "corpus oracle had {divergences} divergences on {repo_str}");
    }
}
