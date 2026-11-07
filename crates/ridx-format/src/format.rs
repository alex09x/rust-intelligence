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
pub const CURRENT_VERSION: u32 = 5;
pub const CURRENT_FORMAT_VERSION: u32 = 5;
pub const EXTRACTOR_VERSION: &str = "0.1.0";

/// Whether a manifest of `format_version` can be opened: versions 3 and 4 read with empty newer tables.
pub fn is_readable_format_version(format_version: u32) -> bool {
    (3..=CURRENT_FORMAT_VERSION).contains(&format_version)
}

pub use crate::driver_hash::*;

mod defs;
mod entries;
mod field_item;
mod header;
mod header_legacy;
mod impl_item;
mod notes;
mod ty;
mod ty_prim;

pub use self::defs::{DefEntry, DefKind, Span, FLAG_MACRO_EXPANSION, FLAG_REEXPORT};
pub use self::entries::{DerefEntry, HashIndexEntry, OwnerIndexEntry, RefEntry, TypeEntry};
pub use self::field_item::{
    CtorShape, DefTyEntry, FieldEntry, ItemVis, DEF_TY_FLAG_FUNDAMENTAL, DEF_TY_FLAG_MUT_STATIC, DEF_TY_FLAG_NON_EXHAUSTIVE, DEF_TY_FLAG_UNSAFE_STATIC, FIELD_FLAG_TUPLE,
    FIELD_FLAG_UNSAFE,
};
pub use self::header::{FileEntry, Header};
pub use self::header_legacy::{HeaderV2, HEADER_V3_SIZE, HEADER_V4_SIZE};
pub use self::impl_item::{AssocTyEntry, ImplEntry, SimplifiedSelfKind, IMPL_FLAG_CONST, IMPL_FLAG_NEGATIVE, IMPL_FLAG_RESERVATION, IMPL_FLAG_TRAIT, IMPL_FLAG_UNSAFE};
pub use self::notes::{NoteEntry, NOTE_CALL_ARGS, NOTE_LAYOUT};
pub use self::ty::{FnSigEntry, TyBoundEntry, TyDefEntry, TyEntry, TyTag};
pub use self::ty_prim::{
    ConstArg, FloatTyWidth, FnAbiKind, GenericArgWord, IntTyWidth, TyDefId, TyId, TyMutability, UintTyWidth, BOUND_FLAG_MAYBE, FN_FLAG_ASYNC, FN_FLAG_CONST, FN_FLAG_HAS_SELF,
    FN_FLAG_UNSAFE, FN_FLAG_VARIADIC, LIFETIME_ELIDED, LIFETIME_STATIC,
};

/// Resolves the global RIDX shared cache directory for external crate dependencies.
/// Uses `$RIDX_CACHE_DIR` if set, or platform default:
/// - Unix: `$XDG_CACHE_HOME/ridx/v{CURRENT_VERSION}` or `$HOME/.cache/ridx/v{CURRENT_VERSION}`
/// - Windows: `%LOCALAPPDATA%/ridx/v{CURRENT_VERSION}` or `$USERPROFILE/.cache/ridx/v{CURRENT_VERSION}`
pub fn resolve_cache_dir() -> std::path::PathBuf {
    resolve_cache_dir_with(std::env::var("RIDX_CACHE_DIR").ok().as_deref(), std::env::var("XDG_CACHE_HOME").ok().as_deref(), std::env::var("HOME").ok().as_deref())
}

pub fn resolve_cache_dir_with(ridx_cache_dir: Option<&str>, xdg_cache_home: Option<&str>, home: Option<&str>) -> std::path::PathBuf {
    if let Some(dir) = ridx_cache_dir {
        let p = std::path::PathBuf::from(dir.trim());
        if !p.as_os_str().is_empty() {
            return p;
        }
    }
    if let Some(xdg) = xdg_cache_home {
        let p = std::path::PathBuf::from(xdg.trim());
        if !p.as_os_str().is_empty() {
            return p.join("ridx").join(format!("v{}", CURRENT_VERSION));
        }
    }
    if let Some(home) = home {
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
    let normalize_if_dot = |p: std::path::PathBuf| -> std::path::PathBuf {
        let rel_str = rel.to_string_lossy();
        if rel_str.contains("..") || rel_str.contains("/.") || rel_str.starts_with("./") {
            if !crate::workspace::provenance::path_traverses_symlink_parent(&p) {
                return crate::paths::normalize_path(&p);
            }
            if let Ok(canon) = p.canonicalize() {
                return canon;
            }
        }
        p
    };
    let direct = root.join(rel);
    if direct.exists() {
        return normalize_if_dot(direct);
    }
    let rel_comps: Vec<_> = rel.components().collect();
    for i in (1..rel_comps.len()).rev() {
        let prefix: std::path::PathBuf = rel_comps[..i].iter().collect();
        if root.ends_with(&prefix) {
            let stripped: std::path::PathBuf = rel_comps[i..].iter().collect();
            let cand = root.join(stripped);
            return normalize_if_dot(cand);
        }
    }
    normalize_if_dot(direct)
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
    assert!(std::mem::size_of::<Header>() == 320);
    assert!(std::mem::size_of::<NoteEntry>() == 28);
    assert!(std::mem::align_of::<NoteEntry>() == 4);
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
    assert!(std::mem::size_of::<TyDefEntry>() == 16);
    assert!(std::mem::align_of::<TyDefEntry>() == 8);
    assert!(std::mem::size_of::<TyEntry>() == 16);
    assert!(std::mem::align_of::<TyEntry>() == 4);
    assert!(std::mem::size_of::<TyBoundEntry>() == 16);
    assert!(std::mem::align_of::<TyBoundEntry>() == 4);
    assert!(std::mem::size_of::<GenericArgWord>() == 4);
    assert!(std::mem::align_of::<GenericArgWord>() == 4);
    assert!(std::mem::size_of::<ImplEntry>() == 32);
    assert!(std::mem::align_of::<ImplEntry>() == 4);
    assert!(std::mem::size_of::<AssocTyEntry>() == 16);
    assert!(std::mem::align_of::<AssocTyEntry>() == 4);
    assert!(std::mem::size_of::<FieldEntry>() == 20);
    assert!(std::mem::align_of::<FieldEntry>() == 4);
    assert!(std::mem::size_of::<DefTyEntry>() == 16);
    assert!(std::mem::align_of::<DefTyEntry>() == 4);
    assert!(std::mem::size_of::<FnSigEntry>() == 16);
    assert!(std::mem::align_of::<FnSigEntry>() == 4);
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
    let mut canon_dirs: std::collections::HashMap<std::path::PathBuf, Option<std::path::PathBuf>> = std::collections::HashMap::new();
    let mut resolve_canon = |abs: &std::path::Path, rel_path: &str| -> Option<std::path::PathBuf> {
        let is_symlink = std::fs::symlink_metadata(abs).is_ok_and(|m| m.file_type().is_symlink());
        if is_symlink || rel_path.contains("..") {
            return abs.canonicalize().ok();
        }
        let parent = abs.parent()?;
        let file_name = abs.file_name()?;
        let canon_parent = canon_dirs.entry(parent.to_path_buf()).or_insert_with(|| parent.canonicalize().ok()).as_ref()?;
        Some(canon_parent.join(file_name))
    };

    let mut resolved_embedded: Vec<(&String, u64, Option<std::path::PathBuf>, Option<std::path::PathBuf>)> = Vec::with_capacity(embedded.len());
    let mut authoritative_by_path: std::collections::HashMap<String, u64> = std::collections::HashMap::with_capacity(embedded.len() * 3);
    for (rel_path, &h) in embedded {
        authoritative_by_path.insert(rel_path.clone(), h);
        if let Some(root) = source_root {
            let abs = root.join(rel_path);
            let canon = resolve_canon(&abs, rel_path);
            authoritative_by_path.insert(abs.to_string_lossy().to_string(), h);
            if let Some(ref c) = canon {
                authoritative_by_path.insert(c.to_string_lossy().to_string(), h);
            }
            resolved_embedded.push((rel_path, h, Some(abs), canon));
        } else {
            resolved_embedded.push((rel_path, h, None, None));
        }
    }

    // 1. Drop any conflicting aliases in file_hashes
    file_hashes.retain(|k, &mut v| {
        if let Some(&emb_h) = authoritative_by_path.get(k) {
            return v == emb_h;
        }
        let p = std::path::Path::new(k);
        if let Some(root) = source_root {
            if let Ok(rel) = p.strip_prefix(root) {
                if let Some(&emb_h) = embedded.get(rel.to_string_lossy().as_ref()) {
                    return v == emb_h;
                }
            }
        }
        true
    });

    // 2. Authoritatively insert embedded relative paths and their absolute/canonical aliases under source_root
    for (rel_path, h, abs_opt, canon_opt) in resolved_embedded {
        file_hashes.insert(rel_path.clone(), h);
        if let Some(abs) = abs_opt {
            file_hashes.insert(abs.to_string_lossy().to_string(), h);
            if let Some(canon) = canon_opt {
                if canon != abs {
                    file_hashes.insert(canon.to_string_lossy().to_string(), h);
                }
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
