/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Picks the crate unit that owns a source file for [`super::query_state::LiveEngine`].

use std::path::{Path, PathBuf};

use ridx_format::{join_resolving_overlap, WorkspaceIndex};

use super::v5_db::LiveState;

/// Whether the reader at `idx` stores `path`, answered from the reader's own file table without touching the disk.
pub(super) fn reader_stores(ws: &WorkspaceIndex, idx: usize, path: &Path) -> bool {
    path.to_str().and_then(|s| ws.target_file_for_reader(idx, s)).is_some_and(|tf| ws.readers()[idx].find_file_id(tf.as_ref()).is_some())
}

/// Whether the reader at `idx` stores `abs`/`canon` under any spelling, resolving every stored path on disk.
fn reader_stores_resolved(ws: &WorkspaceIndex, state: &LiveState, idx: usize, abs: &Path, canon: &Path) -> bool {
    let rdr = &ws.readers()[idx];
    let root = state.metas[idx].source_root.as_deref().or_else(|| ws.workspace_root());
    (0..rdr.header().files_count).any(|fid| {
        let fstr = rdr.get_file(fid);
        !fstr.is_empty() && {
            let joined = root.map_or_else(|| PathBuf::from(fstr), |r| join_resolving_overlap(r, Path::new(fstr)));
            joined == abs || joined == canon || joined.canonicalize().is_ok_and(|c| c == canon)
        }
    })
}

/// The workspace unit that compiles `abs` (`canon` is its canonical form): library targets before other targets
/// before test harnesses, the unit with more references first; failing that, the unit with the deepest source
/// root above the file.
///
/// The indexed lookup runs first; the disk-resolving scan over every stored path of every unit is kept only for
/// spellings the index cannot map (symlinked roots), because it costs one `canonicalize` per stored file.
pub(super) fn find_best_unit(ws: &WorkspaceIndex, state: &LiveState, abs: &Path, canon: &Path) -> Option<usize> {
    for resolve_on_disk in [false, true] {
        let mut best: Option<((u8, u8, u32), usize)> = None;
        for idx in 0..ws.crate_count() {
            let meta = &state.metas[idx];
            if meta.is_dependency || meta.target_kind == "custom-build" {
                continue;
            }
            let has_file =
                if resolve_on_disk { reader_stores_resolved(ws, state, idx, abs, canon) } else { reader_stores(ws, idx, abs) || (canon != abs && reader_stores(ws, idx, canon)) };
            if has_file {
                let kind_rank = match meta.target_kind.as_str() {
                    "lib" | "rlib" | "proc-macro" | "" if !meta.is_test_harness => 0,
                    _ if !meta.is_test_harness => 1,
                    _ => 2,
                };
                let key = (kind_rank, u8::from(meta.is_test_harness), u32::MAX - ws.readers()[idx].header().refs_count);
                if best.is_none_or(|(bk, _)| key < bk) {
                    best = Some((key, idx));
                }
            }
        }
        if let Some((_, idx)) = best {
            return Some(idx);
        }
    }
    let mut best_anc: Option<((usize, u8), usize)> = None;
    for (idx, meta) in state.metas.iter().enumerate() {
        if meta.is_dependency || meta.target_kind == "custom-build" {
            continue;
        }
        let Some(sr) = meta.source_root.as_deref() else { continue };
        let sr_canon = sr.canonicalize().unwrap_or_else(|_| sr.to_path_buf());
        if canon.starts_with(&sr_canon) || abs.starts_with(sr) {
            let score = (sr_canon.as_os_str().len(), u8::from(!meta.is_test_harness));
            if best_anc.is_none_or(|(bs, _)| score > bs) {
                best_anc = Some((score, idx));
            }
        }
    }
    best_anc.map(|(_, i)| i)
}
