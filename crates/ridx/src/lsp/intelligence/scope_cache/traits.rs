/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Trait supertrait and associated type bound parsing with thread-safe caching.

use super::super::self_completion::clean_trait_name;
use ridx_format::{DefKind, WorkspaceIndex};
use std::collections::{HashMap, HashSet};
use std::sync::{OnceLock, RwLock};

type TraitCacheMap = HashMap<(String, u32), Vec<String>>;

static TRAIT_SUPERTRAITS_CACHE: OnceLock<RwLock<TraitCacheMap>> = OnceLock::new();

fn trait_cache() -> &'static RwLock<TraitCacheMap> {
    TRAIT_SUPERTRAITS_CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Parses supertrait names from trait definition source header.
pub fn parse_trait_supertraits_from_source(src: &str, start_line: u32) -> Vec<String> {
    let line_idx = start_line.saturating_sub(1) as usize;
    let mut header = String::new();
    for line in src.lines().skip(line_idx).take(10) {
        if let Some(brace_pos) = line.find('{') {
            header.push(' ');
            header.push_str(&line[..brace_pos]);
            break;
        }
        header.push(' ');
        header.push_str(line.trim());
    }

    if !header.contains("trait ") {
        return Vec::new();
    }

    let Some(colon_pos) = header.find(':') else {
        return Vec::new();
    };

    let after_colon = &header[colon_pos + 1..];
    let super_chunk = after_colon.split("where").next().unwrap_or(after_colon);
    let mut supertraits = Vec::new();
    for part in super_chunk.split('+') {
        let trimmed = part.trim();
        if trimmed.starts_with('\'') || trimmed.is_empty() {
            continue;
        }
        let st_clean = clean_trait_name(trimmed);
        if !st_clean.is_empty() && !supertraits.contains(&st_clean) {
            supertraits.push(st_clean);
        }
    }
    supertraits
}

/// Retrieves or parses supertraits for a trait defined at `file:start_line`.
pub fn get_or_resolve_trait_supertraits(file: &str, start_line: u32) -> Vec<String> {
    let key = (file.to_string(), start_line);
    let cache = trait_cache();
    if let Ok(guard) = cache.read() {
        if let Some(res) = guard.get(&key) {
            return res.clone();
        }
    }

    let Ok(src) = std::fs::read_to_string(file) else {
        return Vec::new();
    };

    let supertraits = parse_trait_supertraits_from_source(&src, start_line);

    if let Ok(mut guard) = cache.write() {
        if guard.len() >= 256 {
            guard.clear();
        }
        guard.insert(key, supertraits.clone());
    }

    supertraits
}

type AssocCacheMap = HashMap<(String, u32), Vec<String>>;

static ASSOC_BOUNDS_CACHE: OnceLock<RwLock<AssocCacheMap>> = OnceLock::new();

fn assoc_cache() -> &'static RwLock<AssocCacheMap> {
    ASSOC_BOUNDS_CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Parses trait bound names from associated type declaration lines.
pub fn parse_assoc_bounds_from_source(src: &str, start_line: u32) -> Vec<String> {
    let line_idx = start_line.saturating_sub(1) as usize;
    let mut lines_iter = src.lines().skip(line_idx).take(10);
    let mut collected = String::new();
    for line in &mut lines_iter {
        collected.push(' ');
        collected.push_str(line.trim());
        if line.contains(';') || line.contains('{') {
            break;
        }
    }

    let Some(colon_pos) = collected.find(':') else {
        return Vec::new();
    };

    let after_colon = &collected[colon_pos + 1..];
    let bound_part = after_colon
        .split(';')
        .next()
        .unwrap_or(after_colon)
        .split('=')
        .next()
        .unwrap_or("")
        .split("where")
        .next()
        .unwrap_or("");

    let mut bounds = Vec::new();
    for part in bound_part.split('+') {
        let trimmed = part.trim();
        if trimmed.starts_with('\'') || trimmed.is_empty() {
            continue;
        }
        let clean = clean_trait_name(trimmed);
        if !clean.is_empty() && !bounds.contains(&clean) {
            bounds.push(clean);
        }
    }
    bounds
}

/// Retrieves or parses trait bounds for an associated type defined at `file:start_line`.
pub fn get_or_resolve_assoc_bounds(file: &str, start_line: u32) -> Vec<String> {
    let key = (file.to_string(), start_line);
    let cache = assoc_cache();
    if let Ok(guard) = cache.read() {
        if let Some(res) = guard.get(&key) {
            return res.clone();
        }
    }

    let Ok(src) = std::fs::read_to_string(file) else {
        return Vec::new();
    };

    let bounds = parse_assoc_bounds_from_source(&src, start_line);

    if let Ok(mut guard) = cache.write() {
        if guard.len() >= 256 {
            guard.clear();
        }
        guard.insert(key, bounds.clone());
    }

    bounds
}

/// Collects all trait names/paths referenced in the given file, expanding glob imports.
pub fn collect_file_referenced_traits(ws: &WorkspaceIndex, file_path: &str) -> HashSet<String> {
    let mut set = HashSet::new();
    for idx in ws.candidate_reader_indices_for_file(file_path) {
        let reader = &ws.readers()[idx];
        let target_file = ws.target_file_for_reader(idx, file_path);
        let target_str = target_file.as_deref().unwrap_or(file_path);
        if let Some(fid) = reader.find_file_id(target_str) {
            let refs = reader.references();
            let start = refs.partition_point(|r| r.span().file_id() < fid);
            let count = refs[start..].partition_point(|r| r.span().file_id() == fid);
            let file_refs = &refs[start..start + count];

            for r in file_refs {
                let r_kind = DefKind::from(r.kind);
                let path = reader.get_str(r.target_path_offset());
                if path.is_empty() {
                    continue;
                }

                if r_kind == DefKind::Use && path.ends_with("::*") {
                    let prefix = path.trim_end_matches("::*");
                    let prefix_colon = format!("{prefix}::");
                    let target_crate = prefix.split("::").next().unwrap_or(prefix);
                    let is_local = matches!(target_crate, "crate" | "self" | "super") || !prefix.contains("::");

                    for candidate_reader in ws.readers() {
                        if is_local {
                            if candidate_reader.crate_name() != reader.crate_name() {
                                continue;
                            }
                        } else if candidate_reader.crate_name() != target_crate && candidate_reader.crate_name() != reader.crate_name() {
                            continue;
                        }

                        for def in candidate_reader.definitions() {
                            if DefKind::from(def.kind) == DefKind::Trait {
                                let d_path = candidate_reader.get_str(def.path_offset());
                                if d_path.starts_with(&prefix_colon) {
                                    let d_name = candidate_reader.get_str(def.name_offset());
                                    set.insert(d_name.to_string());
                                    set.insert(d_path.to_string());
                                }
                            }
                        }
                    }
                } else if r_kind == DefKind::Trait {
                    if let Some(short) = path.rsplit("::").next() {
                        set.insert(short.to_string());
                    }
                    set.insert(path.to_string());
                }
            }
            break;
        }
    }
    set
}
