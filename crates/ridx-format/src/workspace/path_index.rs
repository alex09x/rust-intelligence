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

use super::provenance::path_traverses_symlink_parent;
use super::WorkspaceIndex;
use crate::paths::normalize_path;
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
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
    /// Cached per `PathIndex` build for absolute paths; reset whenever `PathIndex` is rebuilt.
    fs_meta: OnceLock<EntryFsMeta>,
    phase1: Vec<(u32, u32, u64)>,
    phase2: Vec<(u32, u32, u64)>,
}

/// Precomputed lookup index for resolving file provenance and shard mtimes.
#[derive(Debug, Clone, Default)]
pub struct PathIndex {
    generation: u64,
    reader_count: usize,
    origins: Vec<OriginMeta>,
    readers_meta: Vec<ReaderPathMeta>,
    entries: Vec<UniquePathEntry>,
    by_key: HashMap<PathBuf, Vec<u32>>,
    by_file_name: HashMap<OsString, Vec<u32>>,
    dotdot_uids: Vec<u32>,
    bare_reader_files: Vec<String>,
    pub(crate) blast_radius: super::navigation::BlastRadiusIndex,
}

mod build;

impl PathIndex {
    #[inline]
    pub(crate) fn is_current(&self, ws: &WorkspaceIndex) -> bool {
        (self.generation, self.reader_count) == (ws.generation, ws.readers.len())
            && self.generation > 0
    }

    /// Returns `false` when `file_path` cannot match the fast path.
    pub fn may_match_fast_path(&self, ws: &WorkspaceIndex, file_path: &str) -> bool {
        if (self.generation, self.reader_count) != (ws.generation, ws.readers.len()) {
            return true;
        }
        if self.entries.len() <= 64 {
            return true;
        }
        let p = Path::new(file_path);
        let Some(fname) = p.file_name() else {
            return false;
        };
        if self.by_file_name.contains_key(fname) {
            return true;
        }
        let fname_str = fname.to_str().unwrap_or("");
        if self
            .bare_reader_files
            .iter()
            .any(|b| fname_str.ends_with(b.as_str()))
        {
            return true;
        }
        p.parent().is_none_or(|par| {
            par.as_os_str().is_empty()
                || ws.workspace_root.as_deref() == Some(par)
                || self.origins.iter().any(|o| o.raw == par)
        })
    }

    /// Resolves `shard_mtime_for_file` when the in-memory fast path does not match.
    pub fn shard_mtime_slow(
        &self,
        ws: &WorkspaceIndex,
        file_path: &str,
    ) -> Option<std::time::SystemTime> {
        let (idx, _) = self.resolve_checked(ws, file_path, false)?;
        ws.mtimes.get(idx).copied().flatten()
    }

    /// Resolves `source_hash_for_file` when the in-memory fast path does not match.
    pub fn source_hash_slow(&self, ws: &WorkspaceIndex, file_path: &str) -> Option<u64> {
        self.resolve_checked(ws, file_path, true).map(|(_, h)| h)
    }

    fn resolve_checked(
        &self,
        ws: &WorkspaceIndex,
        file_path: &str,
        want_hash: bool,
    ) -> Option<(usize, u64)> {
        if (self.generation, self.reader_count) != (ws.generation, ws.readers.len()) {
            return Self::build(ws).resolve_slow(ws, file_path, want_hash);
        }
        self.resolve_slow(ws, file_path, want_hash)
    }

    fn resolve_slow(
        &self,
        ws: &WorkspaceIndex,
        file_path: &str,
        want_hash: bool,
    ) -> Option<(usize, u64)> {
        let p = Path::new(file_path);
        let explicit_origin = match ws.workspace_root.as_deref().or(ws.root_dir.as_deref()) {
            Some(origin) => Some(origin),
            None if ws.readers.len() == 1 => ws.source_roots.first().and_then(|r| r.as_deref()),
            None => None,
        };

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

        let p_trav = has_parent_dir(p) && path_traverses_symlink_parent(p);
        let abs_trav = ws_abs
            .as_deref()
            .is_some_and(|a| has_parent_dir(a) && path_traverses_symlink_parent(a));
        let matches_uid = |uid: u32| -> bool {
            self.entry_matches_single(uid, p, &norm_p, req_canon.as_deref(), p_trav)
                || ws_abs.as_deref().is_some_and(|abs_req| {
                    self.entry_matches_single(
                        uid,
                        abs_req,
                        norm_ws_abs.as_deref().unwrap_or(abs_req),
                        ws_canon.as_deref(),
                        abs_trav,
                    )
                })
        };

        let mut matching_uids: Vec<u32> = Vec::new();
        if self.entries.len() <= 64 {
            for uid in 0..self.entries.len() as u32 {
                if matches_uid(uid) {
                    matching_uids.push(uid);
                }
            }
        } else {
            let mut seen = HashSet::new();
            let mut check_slice = |slice: Option<&Vec<u32>>, out: &mut Vec<u32>| {
                for &uid in slice.into_iter().flatten() {
                    if seen.insert(uid) && matches_uid(uid) {
                        out.push(uid);
                    }
                }
            };
            for key in [
                Some(p),
                Some(norm_p.as_path()),
                req_canon.as_deref(),
                ws_abs.as_deref(),
                norm_ws_abs.as_deref(),
            ]
            .into_iter()
            .flatten()
            {
                check_slice(self.by_key.get(key), &mut matching_uids);
            }
            check_slice(Some(&self.dotdot_uids), &mut matching_uids);
            if matching_uids.is_empty() && (req_canon.is_some() || p.is_relative()) {
                for fname in [
                    p.file_name(),
                    norm_p.file_name(),
                    req_canon.as_deref().and_then(Path::file_name),
                ]
                .into_iter()
                .flatten()
                {
                    check_slice(self.by_file_name.get(fname), &mut matching_uids);
                }
            }
        }
        if matching_uids.is_empty() {
            return None;
        }

        let tier_of = |i: usize| -> u8 {
            let rmeta = &self.readers_meta[i];
            if req_canon
                .as_ref()
                .zip(rmeta.canon_source_root.as_ref())
                .is_some_and(|(c_p, c_root)| c_p.starts_with(c_root))
            {
                return 0;
            }
            if let Some(ref n_abs) = norm_ws_abs {
                if rmeta
                    .norm_source_root
                    .as_ref()
                    .is_some_and(|n_root| n_abs.starts_with(n_root))
                {
                    return 1;
                }
            } else if p.is_absolute()
                && rmeta
                    .norm_source_root
                    .as_ref()
                    .is_some_and(|n_root| norm_p.starts_with(n_root))
            {
                return 1;
            }
            if !rmeta.is_dep {
                2
            } else {
                3
            }
        };

        let pick_best = |phase1: bool| -> Option<(usize, u64)> {
            let mut best = None;
            for &uid in &matching_uids {
                let list = if phase1 {
                    &self.entries[uid as usize].phase1
                } else {
                    &self.entries[uid as usize].phase2
                };
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

    fn entry_matches_single(
        &self,
        uid: u32,
        req_path: &Path,
        norm_req: &Path,
        req_canon: Option<&Path>,
        req_traverses_symlink: bool,
    ) -> bool {
        let entry = &self.entries[uid as usize];
        let req_str = req_path.to_str().unwrap_or("");
        if entry.entry_str.is_empty() {
            return false;
        }
        if entry.entry_str == req_str || entry.entry_orig == req_path {
            return true;
        }

        let orig_meta = entry.origin_id.map(|oid| &self.origins[oid as usize]);
        if req_canon.is_none() {
            if req_path.is_absolute() || entry.norm_entry.is_relative() {
                if entry.norm_entry != norm_req {
                    return false;
                }
            } else if orig_meta.is_none() {
                return false;
            }
        }

        let compute_fs = || EntryFsMeta {
            canon: entry.entry_orig.canonicalize().ok(),
            traverses_symlink: has_parent_dir(&entry.entry_orig)
                && path_traverses_symlink_parent(&entry.entry_orig),
        };
        let owned_fs;
        let fs_meta = if entry.entry_orig.is_absolute() {
            entry.fs_meta.get_or_init(compute_fs)
        } else {
            owned_fs = compute_fs();
            &owned_fs
        };

        if let (Some(ref c_entry), Some(c_req)) = (&fs_meta.canon, req_canon) {
            return c_entry == c_req;
        }

        let lexical_match = if req_path.is_absolute() || entry.norm_entry.is_relative() {
            entry.norm_entry == norm_req
        } else if let Some(orig) = orig_meta {
            let matches_rel = |base: &Path, full: &Path| {
                full.strip_prefix(base)
                    .is_ok_and(|rel| rel == norm_req || normalize_path(rel) == norm_req)
            };
            matches_rel(&orig.norm, &entry.norm_entry)
                || fs_meta
                    .canon
                    .as_ref()
                    .zip(orig.canon.as_ref())
                    .is_some_and(|(ce, co)| matches_rel(co, ce))
        } else {
            false
        };
        if !lexical_match {
            return false;
        }

        let req_has_dotdot = has_parent_dir(req_path);
        !(fs_meta.traverses_symlink
            || req_traverses_symlink
            || orig_meta.is_some_and(|orig| {
                (orig.has_dotdot || req_has_dotdot)
                    && path_traverses_symlink_parent(&orig.raw.join(req_path))
            }))
    }
}
