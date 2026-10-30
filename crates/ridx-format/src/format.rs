/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Binary format specifications for the RIDX (.ridx) index file.
//!
//! All integer values are stored in little-endian format.
//! Structures are designed for zero-copy memory-mapped reading with natural alignment.

pub const MAGIC: [u8; 4] = *b"RIDX";
pub const CURRENT_VERSION: u32 = 3;
pub const CURRENT_FORMAT_VERSION: u32 = 3;
pub const EXTRACTOR_VERSION: &str = "0.1.0";

pub use crate::driver_hash::*;

mod defs;
mod entries;
mod header;

pub use self::defs::{DefEntry, DefKind, Span, FLAG_MACRO_EXPANSION, FLAG_REEXPORT};
pub use self::entries::{DerefEntry, HashIndexEntry, OwnerIndexEntry, RefEntry, TypeEntry};
pub use self::header::{FileEntry, Header, HeaderV2};

/// Resolves the global RIDX shared cache directory for external crate dependencies.
/// Uses `$RIDX_CACHE_DIR` if set, or platform default:
/// - Unix: `$XDG_CACHE_HOME/ridx/v{CURRENT_VERSION}` or `$HOME/.cache/ridx/v{CURRENT_VERSION}`
/// - Windows: `%LOCALAPPDATA%/ridx/v{CURRENT_VERSION}` or `$USERPROFILE/.cache/ridx/v{CURRENT_VERSION}`
pub fn resolve_cache_dir() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("RIDX_CACHE_DIR") {
        let p = std::path::PathBuf::from(dir.trim());
        if !p.as_os_str().is_empty() {
            return p;
        }
    }
    if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        let p = std::path::PathBuf::from(xdg.trim());
        if !p.as_os_str().is_empty() {
            return p.join("ridx").join(format!("v{}", CURRENT_VERSION));
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        let p = std::path::PathBuf::from(home.trim());
        if !p.as_os_str().is_empty() {
            return p.join(".cache").join("ridx").join(format!("v{}", CURRENT_VERSION));
        }
    }
    #[cfg(windows)]
    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        let p = std::path::PathBuf::from(local_app_data.trim());
        if !p.as_os_str().is_empty() {
            return p.join("ridx").join(format!("v{}", CURRENT_VERSION));
        }
    }
    std::path::PathBuf::from(".ridx_cache").join(format!("v{}", CURRENT_VERSION))
}

/// Returns the path to an object shard inside the global cache for a given unit key.
pub fn cache_object_path(cache_dir: &std::path::Path, unit_key: &str) -> std::path::PathBuf {
    let prefix = if unit_key.len() >= 2 { &unit_key[..2] } else { "00" };
    cache_dir.join("objects").join(prefix).join(format!("{}.ridx", unit_key))
}

/// Joins a base root directory with a relative path, resolving any overlapping path prefix.
///
/// For instance, if `root` is `/workspace/crates/app` and `rel` is `crates/app/src/lib.rs`,
/// naive joining yields `/workspace/crates/app/crates/app/src/lib.rs`.
/// This helper strips the overlapping `crates/app` prefix to yield `/workspace/crates/app/src/lib.rs`.
pub fn join_resolving_overlap(root: &std::path::Path, rel: &std::path::Path) -> std::path::PathBuf {
    if rel.is_absolute() {
        return rel.to_path_buf();
    }
    let direct = root.join(rel);
    if direct.exists() {
        return direct;
    }
    let rel_comps: Vec<_> = rel.components().collect();
    for i in (1..rel_comps.len()).rev() {
        let prefix: std::path::PathBuf = rel_comps[..i].iter().collect();
        if root.ends_with(&prefix) {
            let stripped: std::path::PathBuf = rel_comps[i..].iter().collect();
            let cand = root.join(stripped);
            return cand;
        }
    }
    direct
}

/// Computes a 64-bit FNV-1a hash over source bytes after stripping UTF-8 BOM and normalizing CRLF to LF,
/// exactly matching rustc's internal SourceFile.src normalization.
pub fn normalized_source_hash(bytes: &[u8]) -> u64 {
    let b = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) { &bytes[3..] } else { bytes };
    let mut normalized = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\r' && i + 1 < b.len() && b[i + 1] == b'\n' {
            i += 1;
            continue;
        }
        normalized.push(b[i]);
        i += 1;
    }
    fnv1a_64(&normalized)
}

/// Compile-time layout guarantees to ensure unambiguous on-disk structure sizes.
const _: () = {
    assert!(std::mem::size_of::<Header>() == 160);
    assert!(std::mem::align_of::<Header>() == 8);
    assert!(std::mem::size_of::<HeaderV2>() == 128);
    assert!(std::mem::align_of::<HeaderV2>() == 8);
    assert!(std::mem::size_of::<Span>() == 20);
    assert!(std::mem::align_of::<Span>() == 4);
    assert!(std::mem::size_of::<DefEntry>() == 40);
    assert!(std::mem::align_of::<DefEntry>() == 4);
    assert!(std::mem::size_of::<RefEntry>() == 48);
    assert!(std::mem::align_of::<RefEntry>() == 4);
    assert!(std::mem::size_of::<TypeEntry>() == 28);
    assert!(std::mem::align_of::<TypeEntry>() == 4);
    assert!(std::mem::size_of::<HashIndexEntry>() == 16);
    assert!(std::mem::align_of::<HashIndexEntry>() == 8);
    assert!(std::mem::size_of::<OwnerIndexEntry>() == 16);
    assert!(std::mem::align_of::<OwnerIndexEntry>() == 8);
    assert!(std::mem::size_of::<DerefEntry>() == 16);
    assert!(std::mem::align_of::<DerefEntry>() == 8);
    assert!(std::mem::size_of::<FileEntry>() == 16);
    assert!(std::mem::align_of::<FileEntry>() == 8);
};

/// Computes an aggregate 64-bit digest over a map of file paths to their 64-bit source hashes.
pub fn compute_source_snapshot_digest(file_hashes: &std::collections::HashMap<String, u64>) -> u64 {
    let mut entries: Vec<(&String, &u64)> = file_hashes.iter().collect();
    entries.sort_by_key(|(k, _)| *k);
    let mut buf = Vec::new();
    for (k, h) in entries {
        buf.extend_from_slice(k.as_bytes());
        buf.extend_from_slice(&h.to_le_bytes());
    }
    fnv1a_64(&buf)
}

/// Merges embedded file hashes into a file_hashes map, ensuring embedded hashes are authoritative
/// across both package-relative and absolute/canonical aliases under `source_root`, and dropping
/// any conflicting aliases.
pub fn merge_embedded_hashes_authoritative(
    file_hashes: &mut std::collections::HashMap<String, u64>,
    embedded: &std::collections::HashMap<String, u64>,
    source_root: Option<&std::path::Path>,
) {
    // 1. Drop any conflicting aliases in file_hashes
    file_hashes.retain(|k, &mut v| {
        let p = std::path::Path::new(k);
        let rel_match = if let Some(root) = source_root { p.strip_prefix(root).ok().map(|r| r.to_string_lossy().to_string()) } else { None };
        let target_rel = rel_match.as_deref().unwrap_or(k);
        if let Some(&emb_h) = embedded.get(target_rel).or_else(|| embedded.get(k)) {
            v == emb_h
        } else {
            true
        }
    });

    // 2. Authoritatively insert embedded relative paths and their absolute aliases under source_root
    for (rel_path, &h) in embedded {
        file_hashes.insert(rel_path.clone(), h);
        if let Some(root) = source_root {
            let abs = root.join(rel_path);
            file_hashes.insert(abs.to_string_lossy().to_string(), h);
            if let Ok(canon) = abs.canonicalize() {
                file_hashes.insert(canon.to_string_lossy().to_string(), h);
            }
        }
    }
}

/// FNV-1a 64-bit string hash used across the RIDX format.
#[inline]
pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests;
