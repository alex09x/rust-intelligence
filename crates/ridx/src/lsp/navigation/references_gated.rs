/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! References in target roots their own `#![cfg(…)]` switches off (`#![cfg(feature = "full")]` on a test file). The
//! compiler builds them empty, so the index holds nothing there; rust-analyzer still resolves the paths that start at
//! another crate (`#[tokio::test]`, `tokio::spawn(…)`), and so do these references, through the index's definitions
//! and re-exports. Bare names stay unresolved, as the file's own items and imports are not built.

use ridx_format::{LocationReferences, WorkspaceIndex};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

type GatedFiles = Arc<Vec<(PathBuf, String)>>;
/// An index's `instance_id`, `generation` (bumped by a reload) and shard count.
pub(super) type WsKey = (u64, u64, usize);
/// Per index: a reload builds the list afresh.
static GATED: OnceLock<Mutex<HashMap<WsKey, GatedFiles>>> = OnceLock::new();

/// The key a workspace's derived reference tables are kept under, changed by every reload of its index.
pub(super) fn ws_cache_key(ws: &WorkspaceIndex) -> WsKey {
    (ws.instance_id(), ws.generation(), ws.readers().len())
}

/// Keeps a few workspaces' tables: an older generation is never asked again, so the map is emptied when full.
pub(super) fn insert_bounded<V>(map: &mut HashMap<WsKey, V>, key: WsKey, v: V) {
    if map.len() >= 8 {
        map.clear();
    }
    map.insert(key, v);
}

pub(crate) fn clear_gated_cache() {
    if let Some(mut c) = GATED.get().and_then(|m| m.lock().ok()) {
        c.clear();
    }
}

fn is_id(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// Whether `text` switches itself off with an inner `#![cfg(…)]` before its first item.
fn gated_off(masked: &str) -> bool {
    masked
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .take_while(|l| l.starts_with("#!"))
        .any(|l| l.trim_start_matches("#!").trim_start_matches('[').trim_start().starts_with("cfg("))
}

/// The workspace's files with an inner `#![cfg(…)]` on which no reader has a reference, with their masked text.
fn gated_files(ws: &WorkspaceIndex) -> GatedFiles {
    let key = ws_cache_key(ws);
    let cache = GATED.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(v) = cache.lock().ok().and_then(|c| c.get(&key).cloned()) {
        return v;
    }
    let mut referenced: HashSet<PathBuf> = HashSet::new();
    let mut candidates: Vec<PathBuf> = Vec::new();
    for (i, reader) in ws.readers().iter().enumerate() {
        if ws.is_reader_external(i) {
            continue;
        }
        let Some(root) = ws.source_root_at(i) else { continue };
        let mut with_refs = vec![false; reader.files_count() as usize];
        for r in reader.references() {
            if let Some(f) = with_refs.get_mut(r.span.file_id as usize) {
                *f = true;
            }
        }
        for fid in 0..reader.files_count() {
            let path = ridx_format::join_resolving_overlap(root, std::path::Path::new(reader.get_file(fid)));
            if with_refs[fid as usize] {
                referenced.insert(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                candidates.push(path);
            }
        }
    }
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for p in candidates {
        if referenced.contains(&p) || !seen.insert(p.clone()) {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(&p) {
            let masked = crate::lsp::editing::action_parse::mask_comments_and_strings(&text);
            if gated_off(&masked) {
                out.push((p, masked));
            }
        }
    }
    let v = Arc::new(out);
    if let Ok(mut c) = cache.lock() {
        insert_bounded(&mut c, key, Arc::clone(&v));
    }
    v
}

/// The paths of `scan` that start at a crate and end in `name`: `(path, byte start of name)`.
pub(super) fn crate_paths_ending_in<'a>(scan: &'a str, name: &str) -> Vec<(&'a str, usize)> {
    let b = scan.as_bytes();
    let mut out = Vec::new();
    for (at, _) in scan.match_indices(name) {
        let end = at + name.len();
        if (at > 0 && is_id(b[at - 1])) || (end < b.len() && is_id(b[end])) || !scan[..at].ends_with("::") {
            continue;
        }
        let mut start = at;
        while scan[..start].ends_with("::") {
            let seg_end = start - 2;
            let seg_start = scan[..seg_end].bytes().rposition(|c| !is_id(c)).map_or(0, |p| p + 1);
            if seg_start == seg_end {
                break;
            }
            start = seg_start;
        }
        let path = &scan[start..end];
        let root = path.split("::").next().unwrap_or("");
        let preceded = scan[..start].trim_end().ends_with("::");
        if !preceded && !matches!(root, "self" | "super" | "crate" | "Self") && !root.is_empty() {
            out.push((path, at));
        }
    }
    out
}

/// References to the target of `refs` in switched-off target roots: `(file, 0-based line, utf16 start, utf16 end)`.
pub(super) fn gated_file_refs(ws: &WorkspaceIndex, refs: &LocationReferences<'_>) -> Vec<(PathBuf, u32, u32, u32)> {
    let name = refs.target_symbol;
    // An associated item (`Builder::new_current_thread`) resolves through the type, in bodies the build left out.
    let assoc = matches!(refs.target_kind, ridx_format::DefKind::AssocFn | ridx_format::DefKind::AssocConst | ridx_format::DefKind::AssocTy);
    if name.is_empty() || refs.target_file.is_empty() || assoc {
        return Vec::new();
    }
    let target = (refs.target_resolved_path(), refs.target_line);
    let mut resolved: HashMap<&str, bool> = HashMap::new();
    let mut out = Vec::new();
    let files = gated_files(ws);
    for (path, masked) in files.iter() {
        if !masked.contains(name) {
            continue;
        }
        for (line_idx, scan) in masked.lines().enumerate() {
            for (p, at) in crate_paths_ending_in(scan, name) {
                let hit = *resolved.entry(p).or_insert_with(|| ws.find_definition_by_path(p).is_some_and(|d| (d.resolved_path(), d.start_line) == target));
                if hit {
                    let col = scan[..at].encode_utf16().count() as u32;
                    out.push((path.clone(), line_idx as u32, col, col + name.encode_utf16().count() as u32));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{crate_paths_ending_in, insert_bounded, ws_cache_key};

    #[test]
    fn a_reload_changes_the_cache_key_and_the_cache_stays_bounded() {
        // The key was the index's address: a reload in place kept it, and the old tables were served.
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("target/ridx")).unwrap();
        let mut writer = ridx_format::IndexWriter::new("gated_demo", "0.1.0");
        writer.write_to(&mut std::fs::File::create(dir.path().join("target/ridx/gated_demo.ridx")).unwrap()).unwrap();
        let mut ws = ridx_format::WorkspaceIndex::open(dir.path().join("target/ridx")).unwrap();
        let before = ws_cache_key(&ws);
        ws.bump_generation();
        assert_ne!(ws_cache_key(&ws), before, "a reload bumps the generation");
        let other = ridx_format::WorkspaceIndex::open(dir.path().join("target/ridx")).unwrap();
        assert_ne!(ws_cache_key(&other).0, before.0, "another index never shares the key");
        let mut map = std::collections::HashMap::new();
        for g in 0..20u64 {
            insert_bounded(&mut map, (1, g, 1), g);
        }
        assert!(map.len() <= 8 && map.get(&(1, 19, 1)) == Some(&19), "{map:?}");
    }

    #[test]
    fn finds_crate_rooted_paths_to_a_name() {
        assert_eq!(crate_paths_ending_in("#[tokio::test(flavor = \"x\")]", "test"), vec![("tokio::test", 9)]);
        assert_eq!(crate_paths_ending_in("    let h = tokio::task::spawn(f);", "spawn"), vec![("tokio::task::spawn", 25)]);
        assert!(crate_paths_ending_in("spawn(f); self::spawn(g); crate::a::spawn(h)", "spawn").is_empty());
        assert!(crate_paths_ending_in("tokio::spawner(f)", "spawn").is_empty());
    }
}
