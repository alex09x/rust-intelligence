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
static GATED: OnceLock<Mutex<HashMap<(usize, usize), GatedFiles>>> = OnceLock::new();

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
    let key = (ws as *const WorkspaceIndex as usize, ws.readers().len());
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
        c.insert(key, Arc::clone(&v));
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
    use super::crate_paths_ending_in;

    #[test]
    fn finds_crate_rooted_paths_to_a_name() {
        assert_eq!(crate_paths_ending_in("#[tokio::test(flavor = \"x\")]", "test"), vec![("tokio::test", 9)]);
        assert_eq!(crate_paths_ending_in("    let h = tokio::task::spawn(f);", "spawn"), vec![("tokio::task::spawn", 25)]);
        assert!(crate_paths_ending_in("spawn(f); self::spawn(g); crate::a::spawn(h)", "spawn").is_empty());
        assert!(crate_paths_ending_in("tokio::spawner(f)", "spawn").is_empty());
    }
}
