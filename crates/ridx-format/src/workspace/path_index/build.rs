/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Builder logic for `PathIndex`.

use super::{has_parent_dir, OriginMeta, PathIndex, ReaderPathMeta, UniquePathEntry};
use crate::paths::normalize_path;
use crate::workspace::WorkspaceIndex;
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

impl PathIndex {
    /// Builds the path index from the current `WorkspaceIndex` state.
    pub fn build(ws: &WorkspaceIndex) -> Self {
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

        let mut intern = |entries: &mut Vec<UniquePathEntry>,
                          origin_id: Option<u32>,
                          entry_str: &str|
         -> usize {
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

            for k in [
                PathBuf::from(entry_str),
                entry_orig.clone(),
                norm_entry.clone(),
            ] {
                push_u(by_key.entry(k).or_default(), uid);
            }
            if let Some(oid) = origin_id {
                let orig = &origins[oid as usize];
                if let Ok(rel) = norm_entry.strip_prefix(&orig.norm) {
                    let nrel = normalize_path(rel);
                    if let Some(ref co) = orig.canon {
                        push_u(
                            by_key.entry(normalize_path(&co.join(&nrel))).or_default(),
                            uid,
                        );
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
            for fname in [entry_orig.file_name(), norm_entry.file_name()]
                .into_iter()
                .flatten()
            {
                push_u(by_file_name.entry(fname.to_os_string()).or_default(), uid);
            }
            if has_parent_dir(&entry_orig)
                && (entry_orig.file_name().is_none() || norm_entry.file_name().is_none())
            {
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

        let mut bare_reader_files: Vec<String> = Vec::new();
        for (i, reader) in ws.readers.iter().enumerate() {
            let oid = readers_meta[i].origin_id;
            for fid in 0..reader.files_count() {
                let s = reader.get_file(fid);
                if !s.is_empty() {
                    if !s.contains('/')
                        && !s.contains('\\')
                        && !bare_reader_files.iter().any(|b| b == s)
                    {
                        bare_reader_files.push(s.to_string());
                    }
                    let uid = intern(&mut entries, oid, s);
                    entries[uid]
                        .phase1
                        .push((i as u32, fid, reader.file_source_hash(fid)));
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
            generation: ws.generation,
            reader_count: ws.readers.len(),
            origins,
            readers_meta,
            entries,
            by_key,
            by_file_name,
            dotdot_uids,
            bare_reader_files,
            blast_radius: crate::workspace::navigation::BlastRadiusIndex::build(ws),
        }
    }
}
