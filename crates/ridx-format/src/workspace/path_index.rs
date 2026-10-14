/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Precomputed path provenance index for `WorkspaceIndex`.

use super::provenance::{normalize_path, path_traverses_symlink_parent};
use super::WorkspaceIndex;
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::hash::{Hash, Hasher};
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

#[inline]
fn has_parent_dir(path: &Path) -> bool {
    path.components().any(|c| matches!(c, Component::ParentDir))
}

#[derive(Debug, Clone)]
struct OriginMeta {
    raw: PathBuf,
    norm: PathBuf,
    canon: Option<PathBuf>,
    has_dotdot: bool,
}

#[derive(Debug, Clone)]
struct ReaderPathMeta {
    origin_id: Option<u32>,
    norm_source_root: Option<PathBuf>,
    canon_source_root: Option<PathBuf>,
    is_dep: bool,
}

#[derive(Debug, Clone)]
struct EntryFsMeta {
    canon: Option<PathBuf>,
    traverses_symlink: bool,
}

#[derive(Debug, Clone)]
struct UniquePathEntry {
    entry_str: String,
    origin_id: Option<u32>,
    entry_orig: PathBuf,
    norm_entry: PathBuf,
    fs_meta: OnceLock<EntryFsMeta>,
    phase1: Vec<(u32, u32, u64)>,
    phase2: Vec<(u32, u32, u64)>,
}

/// Precomputed lookup index for resolving file provenance and shard mtimes.
#[derive(Debug, Clone, Default)]
pub struct PathIndex {
    fingerprint: u64,
    origins: Vec<OriginMeta>,
    readers_meta: Vec<ReaderPathMeta>,
    entries: Vec<UniquePathEntry>,
    by_key: HashMap<PathBuf, Vec<u32>>,
    by_file_name: HashMap<OsString, Vec<u32>>,
    dotdot_uids: Vec<u32>,
}

impl PathIndex {
    fn compute_fingerprint(ws: &WorkspaceIndex) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (ws.readers.len(), &ws.workspace_root, &ws.root_dir).hash(&mut h);
        (&ws.source_roots, &ws.reader_is_dependency).hash(&mut h);
        for (i, r) in ws.readers.iter().enumerate() {
            (r.crate_name(), r.files_count()).hash(&mut h);
            ws.reader_file_hashes.get(i).map(HashMap::len).hash(&mut h);
        }
        h.finish()
    }

    /// Builds the path index from the current `WorkspaceIndex` state.
    pub fn build(ws: &WorkspaceIndex) -> Self {
        let fingerprint = Self::compute_fingerprint(ws);
        let mut canon_cache: HashMap<PathBuf, Option<PathBuf>> = HashMap::new();
        let mut canon_of = |p: &Path| -> Option<PathBuf> {
            canon_cache
                .entry(p.to_path_buf())
                .or_insert_with(|| p.canonicalize().ok())
                .clone()
        };

        let mut origins: Vec<OriginMeta> = Vec::new();
        let mut origin_map: HashMap<PathBuf, u32> = HashMap::new();
        let mut readers_meta: Vec<ReaderPathMeta> = Vec::with_capacity(ws.readers.len());

        for i in 0..ws.readers.len() {
            let origin_path = ws
                .source_roots
                .get(i)
                .and_then(|r| r.as_deref())
                .or(ws.workspace_root.as_deref())
                .or(ws.root_dir.as_deref());
            let origin_id = origin_path.map(|op| {
                if let Some(&id) = origin_map.get(op) {
                    return id;
                }
                let id = origins.len() as u32;
                let raw = op.to_path_buf();
                origins.push(OriginMeta {
                    norm: normalize_path(&raw),
                    canon: canon_of(&raw),
                    has_dotdot: has_parent_dir(&raw),
                    raw: raw.clone(),
                });
                origin_map.insert(raw, id);
                id
            });
            let sroot = ws.source_roots.get(i).and_then(|r| r.as_deref());
            readers_meta.push(ReaderPathMeta {
                origin_id,
                norm_source_root: sroot.map(normalize_path),
                canon_source_root: sroot.and_then(&mut canon_of),
                is_dep: ws.reader_is_dependency.get(i).copied().unwrap_or(false) || sroot.is_some(),
            });
        }

        let mut entries: Vec<UniquePathEntry> = Vec::new();
        let mut entry_map: HashMap<(Option<u32>, String), u32> = HashMap::new();
        let mut by_key: HashMap<PathBuf, Vec<u32>> = HashMap::new();
        let mut by_file_name: HashMap<OsString, Vec<u32>> = HashMap::new();
        let mut dotdot_uids: Vec<u32> = Vec::new();

        let push_u = |vec: &mut Vec<u32>, uid: u32| {
            if vec.last().copied() != Some(uid) && !vec.contains(&uid) {
                vec.push(uid);
            }
        };

        let mut intern = |entries: &mut Vec<UniquePathEntry>, origin_id: Option<u32>, entry_str: &str| -> usize {
            let key = (origin_id, entry_str.to_string());
            if let Some(&uid) = entry_map.get(&key) {
                return uid as usize;
            }
            let uid = entries.len() as u32;
            let entry_p = Path::new(entry_str);
            let entry_orig = match (entry_p.is_absolute(), origin_id) {
                (true, _) | (false, None) => entry_p.to_path_buf(),
                (false, Some(oid)) => origins[oid as usize].raw.join(entry_p),
            };
            let norm_entry = normalize_path(&entry_orig);

            for k in [PathBuf::from(entry_str), entry_orig.clone(), norm_entry.clone()] {
                push_u(by_key.entry(k).or_default(), uid);
            }
            if let Some(oid) = origin_id {
                let orig = &origins[oid as usize];
                if let Ok(rel) = norm_entry.strip_prefix(&orig.norm) {
                    let nrel = normalize_path(rel);
                    if let Some(ref co) = orig.canon {
                        push_u(by_key.entry(normalize_path(&co.join(&nrel))).or_default(), uid);
                    }
                    push_u(by_key.entry(rel.to_path_buf()).or_default(), uid);
                    push_u(by_key.entry(nrel).or_default(), uid);
                }
                if entry_p.is_relative() {
                    if let Some(ref co) = orig.canon {
                        let cj = normalize_path(&co.join(entry_p));
                        if let Ok(rel) = cj.strip_prefix(co) {
                            push_u(by_key.entry(rel.to_path_buf()).or_default(), uid);
                            push_u(by_key.entry(normalize_path(rel)).or_default(), uid);
                        }
                        push_u(by_key.entry(cj).or_default(), uid);
                    }
                }
            }
            for fname in [entry_orig.file_name(), norm_entry.file_name()].into_iter().flatten() {
                push_u(by_file_name.entry(fname.to_os_string()).or_default(), uid);
            }
            if has_parent_dir(&entry_orig) {
                push_u(&mut dotdot_uids, uid);
            }
            entries.push(UniquePathEntry {
                entry_str: key.1.clone(),
                origin_id,
                entry_orig,
                norm_entry,
                fs_meta: OnceLock::new(),
                phase1: Vec::new(),
                phase2: Vec::new(),
            });
            entry_map.insert(key, uid);
            uid as usize
        };

        for (i, reader) in ws.readers.iter().enumerate() {
            let oid = readers_meta[i].origin_id;
            for fid in 0..reader.files_count() {
                let s = reader.get_file(fid);
                if !s.is_empty() {
                    let uid = intern(&mut entries, oid, s);
                    entries[uid].phase1.push((i as u32, fid, reader.file_source_hash(fid)));
                }
            }
            if let Some(map) = ws.reader_file_hashes.get(i) {
                for (ord, (k, &h)) in map.iter().enumerate() {
                    if !k.is_empty() {
                        let uid = intern(&mut entries, oid, k);
                        entries[uid].phase2.push((i as u32, ord as u32, h));
                    }
                }
            }
        }
        Self {
            fingerprint,
            origins,
            readers_meta,
            entries,
            by_key,
            by_file_name,
            dotdot_uids,
        }
    }

    /// Resolves `shard_mtime_for_file` when the in-memory fast path does not match.
    pub fn shard_mtime_slow(&self, ws: &WorkspaceIndex, file_path: &str) -> Option<std::time::SystemTime> {
        let (idx, _) = self.resolve_checked(ws, file_path, false)?;
        ws.mtimes.get(idx).copied().flatten()
    }

    /// Resolves `source_hash_for_file` when the in-memory fast path does not match.
    pub fn source_hash_slow(&self, ws: &WorkspaceIndex, file_path: &str) -> Option<u64> {
        self.resolve_checked(ws, file_path, true).map(|(_, h)| h)
    }

    fn resolve_checked(&self, ws: &WorkspaceIndex, file_path: &str, want_hash: bool) -> Option<(usize, u64)> {
        if self.fingerprint != Self::compute_fingerprint(ws) {
            return Self::build(ws).resolve_slow(ws, file_path, want_hash);
        }
        self.resolve_slow(ws, file_path, want_hash)
    }

    fn resolve_slow(&self, ws: &WorkspaceIndex, file_path: &str, want_hash: bool) -> Option<(usize, u64)> {
        let p = Path::new(file_path);
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
        let norm_p = normalize_path(p);
        let norm_ws_abs = ws_abs.as_deref().map(normalize_path);

        let mut candidate_uids: Vec<u32> = Vec::new();
        if self.entries.len() <= 64 {
            candidate_uids.extend(0..self.entries.len() as u32);
        } else {
            let mut seen = HashSet::new();
            let mut add = |slice: Option<&Vec<u32>>| {
                for &uid in slice.into_iter().flatten() {
                    if seen.insert(uid) {
                        candidate_uids.push(uid);
                    }
                }
            };
            for key in [Some(p), Some(norm_p.as_path()), req_canon.as_deref(), ws_abs.as_deref(), norm_ws_abs.as_deref()].into_iter().flatten() {
                add(self.by_key.get(key));
            }
            add(Some(&self.dotdot_uids));
            if req_canon.is_some() || p.is_relative() {
                for fname in [p.file_name(), norm_p.file_name(), req_canon.as_deref().and_then(Path::file_name)].into_iter().flatten() {
                    add(self.by_file_name.get(fname));
                }
            }
        }
        if candidate_uids.is_empty() {
            return None;
        }

        let p_trav = has_parent_dir(p) && path_traverses_symlink_parent(p);
        let abs_trav = ws_abs.as_deref().is_some_and(|a| has_parent_dir(a) && path_traverses_symlink_parent(a));

        let matching_uids: Vec<u32> = candidate_uids
            .into_iter()
            .filter(|&uid| {
                self.entry_matches_single(uid, p, &norm_p, req_canon.as_deref(), p_trav)
                    || ws_abs.as_deref().is_some_and(|abs_req| {
                        self.entry_matches_single(uid, abs_req, norm_ws_abs.as_deref().unwrap_or(abs_req), ws_canon.as_deref(), abs_trav)
                    })
            })
            .collect();
        if matching_uids.is_empty() {
            return None;
        }

        let tier_of = |i: usize| -> u8 {
            let rmeta = &self.readers_meta[i];
            if req_canon.as_ref().zip(rmeta.canon_source_root.as_ref()).is_some_and(|(c_p, c_root)| c_p.starts_with(c_root)) {
                return 0;
            }
            if let Some(ref n_abs) = norm_ws_abs {
                if rmeta.norm_source_root.as_ref().is_some_and(|n_root| n_abs.starts_with(n_root)) {
                    return 1;
                }
            } else if p.is_absolute() && rmeta.norm_source_root.as_ref().is_some_and(|n_root| norm_p.starts_with(n_root)) {
                return 1;
            }
            if !rmeta.is_dep { 2 } else { 3 }
        };

        let pick_best = |phase1: bool| -> Option<(usize, u64)> {
            let mut best = None;
            for &uid in &matching_uids {
                let list = if phase1 { &self.entries[uid as usize].phase1 } else { &self.entries[uid as usize].phase2 };
                for &(r_idx, ord, h) in list {
                    if want_hash && h == 0 {
                        continue;
                    }
                    let rank = (tier_of(r_idx as usize), r_idx, ord);
                    if best.is_none_or(|(best_rank, _)| rank < best_rank) {
                        best = Some((rank, (r_idx as usize, h)));
                    }
                }
            }
            best.map(|(_, res)| res)
        };
        pick_best(true).or_else(|| pick_best(false))
    }

    fn entry_matches_single(&self, uid: u32, req_path: &Path, norm_req: &Path, req_canon: Option<&Path>, req_traverses_symlink: bool) -> bool {
        let entry = &self.entries[uid as usize];
        let req_str = req_path.to_str().unwrap_or("");
        if entry.entry_str.is_empty() {
            return false;
        }
        if entry.entry_str == req_str || entry.entry_orig == req_path {
            return true;
        }

        let compute_fs = || EntryFsMeta {
            canon: entry.entry_orig.canonicalize().ok(),
            traverses_symlink: has_parent_dir(&entry.entry_orig) && path_traverses_symlink_parent(&entry.entry_orig),
        };
        let owned_fs;
        let fs_meta = if entry.entry_orig.is_absolute() {
            entry.fs_meta.get_or_init(compute_fs)
        } else {
            owned_fs = compute_fs();
            &owned_fs
        };

        let effective_req_canon = req_canon
            .map(Path::to_path_buf)
            .or_else(|| req_path.is_absolute().then(|| req_path.canonicalize().ok())?);
        if let (Some(ref c_entry), Some(ref c_req)) = (&fs_meta.canon, &effective_req_canon) {
            return c_entry == c_req;
        }

        let orig_meta = entry.origin_id.map(|oid| &self.origins[oid as usize]);
        let req_has_dotdot = has_parent_dir(req_path);
        if fs_meta.traverses_symlink
            || req_traverses_symlink
            || orig_meta.is_some_and(|orig| (orig.has_dotdot || req_has_dotdot) && path_traverses_symlink_parent(&orig.raw.join(req_path)))
        {
            return false;
        }

        if req_path.is_absolute() || entry.norm_entry.is_relative() {
            return entry.norm_entry == norm_req;
        }
        let Some(orig) = orig_meta else { return false };
        let matches_rel = |base: &Path, full: &Path| {
            full.strip_prefix(base).is_ok_and(|rel| rel == norm_req || normalize_path(rel) == norm_req)
        };
        matches_rel(&orig.norm, &entry.norm_entry)
            || fs_meta.canon.as_ref().zip(orig.canon.as_ref()).is_some_and(|(ce, co)| matches_rel(co, ce))
    }
}
