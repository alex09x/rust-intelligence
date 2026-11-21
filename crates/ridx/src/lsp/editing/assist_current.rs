/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Whether the index read the crates holding a file as their sources are now: what an assist may take its absence of a
//! macro from (review of f60136bc: a parent file changed since it was indexed may define one the child inherits).

use crate::overlay::OverlayManager;
use ridx_format::WorkspaceIndex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

/// By path: the modification time and length a hash was taken at, and the hash.
type Hashes = HashMap<PathBuf, (SystemTime, u64, u64)>;

/// The content hash of the file at `path` on disk, kept while its modification time and length stay.
fn disk_hash(path: &Path) -> Option<u64> {
    static CACHE: OnceLock<Mutex<Hashes>> = OnceLock::new();
    let meta = std::fs::metadata(path).ok()?;
    let stamp = (meta.modified().ok()?, meta.len());
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(&(t, l, h)) = cache.lock().ok()?.get(path) {
        if (t, l) == stamp {
            return Some(h);
        }
    }
    let h = ridx_format::normalized_source_hash(&std::fs::read(path).ok()?);
    cache.lock().ok()?.insert(path.to_path_buf(), (stamp.0, stamp.1, h));
    Some(h)
}

/// Whether every source the index read for the crates holding `file`, `file` among them, other than `file` itself (the
/// text the assist reads), is as it read it: in its open buffer, else on disk.
pub(crate) fn crates_current(ws: &WorkspaceIndex, overlays: &OverlayManager, file: &str) -> bool {
    let own = ridx_format::canonicalize_best_effort(Path::new(file));
    let readers = ws.reader_indices_for_file(file);
    let mut holds_file = false;
    for idx in readers {
        for (key, hash) in ws.readers()[idx].embedded_file_hashes() {
            let Some(path) = super::safe_delete_fresh::resolve(ws, idx, &key) else { return false };
            if ridx_format::canonicalize_best_effort(&path) == own {
                holds_file = true;
                continue;
            }
            if hash == 0 || super::safe_delete_fresh::cached(&path) {
                continue;
            }
            let now = match overlays.get(&path.to_string_lossy()) {
                Some(o) => Some(ridx_format::normalized_source_hash(o.text.as_bytes())),
                None => disk_hash(&path),
            };
            if now != Some(hash) {
                return false;
            }
        }
    }
    holds_file
}

#[cfg(test)]
#[path = "tests/assist_current_tests.rs"]
mod tests;
