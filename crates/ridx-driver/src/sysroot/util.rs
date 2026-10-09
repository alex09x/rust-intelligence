/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use rustc_middle::ty::TyCtxt;
use rustc_span::def_id::DefId;
use std::path::{Path, PathBuf};

/// Remaps a compiler-internal span path (e.g. `/rustc/<commit>/library/...`)
/// to a local `rust-src` source path if available, or returns the relative library path.
pub fn remap_sysroot_path(raw_path: &str, sysroot_src: Option<&Path>) -> String {
    if let Some(idx) = raw_path.find("/library/") {
        let rel_part = &raw_path[idx + 1..];
        if let Some(src_root) = sysroot_src {
            let candidate = src_root.join(rel_part);
            if candidate.exists() {
                return candidate.to_string_lossy().into_owned();
            }
        }
        return rel_part.to_string();
    }
    raw_path.to_string()
}

/// Discovers the path to `library` inside `rust-src` for the current sysroot if installed.
pub fn discover_rust_src_path(tcx: TyCtxt<'_>) -> Option<PathBuf> {
    let candidate = tcx
        .sess
        .opts
        .sysroot
        .path()
        .join("lib/rustlib/src/rust/library");
    if candidate.is_dir() {
        Some(candidate)
    } else {
        None
    }
}

/// Checks if a definition is public and not doc(hidden).
#[inline]
pub fn is_publicly_visible(tcx: TyCtxt<'_>, did: DefId) -> bool {
    tcx.visibility(did).is_public() && !tcx.is_doc_hidden(did)
}

/// Checks if a definition is stable (or unmarked), skipping unstable compiler features.
#[inline]
pub fn is_stable_api(tcx: TyCtxt<'_>, did: DefId) -> bool {
    if let Some(stab) = tcx.lookup_stability(did) {
        !stab.level.is_unstable()
    } else {
        true
    }
}
