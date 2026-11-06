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
    let bound_part = after_colon.split(';').next().unwrap_or(after_colon).split('=').next().unwrap_or("").split("where").next().unwrap_or("");

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

/// Resolves a module prefix (`crate::`, `self::`, `super::`) relative to the file's owning crate and module.
fn resolve_module_prefix(ws: &WorkspaceIndex, file_path: &str, reader_crate: &str, path: &str) -> String {
    if let Some(rest) = path.strip_prefix("crate::") {
        format!("{reader_crate}::{rest}")
    } else if path == "crate" {
        reader_crate.to_string()
    } else if let Some(rest) = path.strip_prefix("self::") {
        let mod_path = ws.module_path_for_file(file_path).unwrap_or_else(|| reader_crate.to_string());
        format!("{mod_path}::{rest}")
    } else if path == "self" {
        ws.module_path_for_file(file_path).unwrap_or_else(|| reader_crate.to_string())
    } else if path.starts_with("super::") || path == "super" {
        let mod_path = ws.module_path_for_file(file_path).unwrap_or_else(|| reader_crate.to_string());
        let mut cur = mod_path.as_str();
        let mut rest = path;
        while let Some(r) = rest.strip_prefix("super::") {
            if let Some((p, _)) = cur.rsplit_once("::") {
                cur = p;
            }
            rest = r;
        }
        if rest == "super" {
            if let Some((p, _)) = cur.rsplit_once("::") {
                cur = p;
            }
            rest = "";
        }
        if rest.is_empty() {
            cur.to_string()
        } else {
            format!("{cur}::{rest}")
        }
    } else {
        path.to_string()
    }
}

/// The 1-based line ranges of the `use` items in `text` (a group may span lines up to its `;`).
pub(crate) fn use_item_lines(text: &str) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut open: Option<u32> = None;
    for (i, line) in text.lines().enumerate() {
        let n = i as u32 + 1;
        let code = line.split("//").next().unwrap_or(line).trim();
        if open.is_none() {
            let rest = code.strip_prefix("pub ").or_else(|| code.strip_prefix("pub(crate) ")).unwrap_or(code);
            if rest.starts_with("use ") {
                open = Some(n);
            }
        }
        if let Some(start) = open {
            if code.ends_with(';') {
                out.push((start, n));
                open = None;
            }
        }
    }
    out
}

/// Adds the traits a glob import of module `prefix` brings into scope: those defined in it, those it re-exports by
/// name (`pub use bevy_ecs::change_detection::DetectChanges`) and, through its own glob re-exports
/// (`pub use crate::ecs::prelude::*`), those of other modules, a few levels deep.
fn expand_glob_traits(ws: &WorkspaceIndex, prefix: &str, importer_crate: &str, set: &mut HashSet<String>, depth: u32) {
    if depth > 4 {
        return;
    }
    // Each module is expanded once per workspace generation.
    type GlobCache = HashMap<(usize, String, String), std::sync::Arc<HashSet<String>>>;
    static GLOB_CACHE: OnceLock<RwLock<GlobCache>> = OnceLock::new();
    let key = (ws as *const WorkspaceIndex as usize, prefix.to_string(), importer_crate.to_string());
    let cache = GLOB_CACHE.get_or_init(|| RwLock::new(HashMap::new()));
    if let Some(hit) = cache.read().ok().and_then(|c| c.get(&key).cloned()) {
        set.extend(hit.iter().cloned());
        return;
    }
    let mut own = HashSet::new();
    expand_glob_traits_uncached(ws, prefix, importer_crate, &mut own, depth);
    let own = std::sync::Arc::new(own);
    if let Ok(mut c) = cache.write() {
        if c.len() > 4096 {
            c.clear();
        }
        c.insert(key, own.clone());
    }
    set.extend(own.iter().cloned());
}

/// Whether the `use` item at 1-based `line` of `file` is `pub` (a glob import brings only public items).
fn use_item_is_pub(ws: &WorkspaceIndex, file: &str, line: u32) -> bool {
    let path = std::path::Path::new(file);
    let path = if path.is_absolute() { path.to_path_buf() } else { ws.workspace_root().map(|r| r.join(path)).unwrap_or_else(|| path.to_path_buf()) };
    let Ok(text) = std::fs::read_to_string(&path) else { return true };
    let mut i = line.saturating_sub(1) as usize;
    let lines: Vec<&str> = text.lines().collect();
    // The item may start on an earlier line of a group (`pub use crate::{\n    a::Trait,\n};`).
    while i > 0 && !lines.get(i).is_some_and(|l| l.contains("use ")) {
        i -= 1;
    }
    lines.get(i).is_some_and(|l| l.trim_start().starts_with("pub"))
}

fn expand_glob_traits_uncached(ws: &WorkspaceIndex, prefix: &str, importer_crate: &str, set: &mut HashSet<String>, depth: u32) {
    let prefix_colon = format!("{prefix}::");
    let target_crate = prefix.split("::").next().unwrap_or(prefix);
    let mut nested: Vec<String> = Vec::new();
    for candidate_reader in ws.readers() {
        let crate_name = candidate_reader.crate_name();
        if crate_name != target_crate && crate_name != importer_crate {
            continue;
        }
        for def in candidate_reader.definitions() {
            let kind = DefKind::from(def.kind);
            if !matches!(kind, DefKind::Trait | DefKind::Use) {
                continue;
            }
            let raw = candidate_reader.get_str(def.path_offset());
            let full = if raw.starts_with(&format!("{crate_name}::")) { raw.to_string() } else { format!("{crate_name}::{raw}") };
            let in_module = |p: &str| p.strip_prefix(&prefix_colon).is_some_and(|rest| !rest.contains("::") || rest.ends_with("::*") && !rest[..rest.len() - 3].contains("::"));
            if !(in_module(raw) || in_module(&full)) {
                continue;
            }
            let name = candidate_reader.get_str(def.name_offset());
            if kind == DefKind::Trait {
                set.insert(name.to_string());
                set.insert(raw.to_string());
                continue;
            }
            let target = candidate_reader.get_str(def.sig_offset());
            if target.is_empty() || !use_item_is_pub(ws, candidate_reader.get_file(def.span().file_id()), def.span().start_line()) {
                continue;
            }
            if name == "*" {
                // A crate-relative module (`schedule::condition`) is in the crate that re-exports it.
                let module = target.trim_end_matches("::*");
                let first = module.split("::").next().unwrap_or(module);
                nested.push(if ws.has_crate(first) { module.to_string() } else { format!("{crate_name}::{module}") });
            } else if let Some(t) = ws.find_definition_by_path(target).filter(|d| d.kind == DefKind::Trait) {
                set.insert(name.to_string());
                set.insert(t.path.to_string());
                set.insert(target.to_string());
            }
        }
    }
    for n in nested {
        if n != prefix {
            expand_glob_traits(ws, &n, importer_crate, set, depth + 1);
        }
    }
}

/// Collects the traits the given file brings into scope with `use` items, expanding glob imports.
pub fn collect_file_referenced_traits(ws: &WorkspaceIndex, file_path: &str) -> HashSet<String> {
    let mut set = HashSet::new();
    let use_lines = std::fs::read_to_string(file_path).map(|t| use_item_lines(&t)).unwrap_or_default();
    for idx in ws.candidate_reader_indices_for_file(file_path) {
        let reader = &ws.readers()[idx];
        let Some(target_file) = ws.target_file_for_reader(idx, file_path) else {
            continue;
        };
        if let Some(fid) = reader.find_file_id(target_file.as_ref()) {
            // Glob imports, also inside a group (`use bevy::{prelude::*, ..}`), are the file's `*` use items.
            for def in reader.definitions().iter().filter(|d| d.span().file_id() == fid && DefKind::from(d.kind) == DefKind::Use) {
                if reader.get_str(def.name_offset()) == "*" {
                    let target = reader.get_str(def.sig_offset()).trim_end_matches("::*").to_string();
                    if !target.is_empty() {
                        expand_glob_traits(ws, &target, reader.crate_name(), &mut set, 0);
                    }
                }
            }
            let refs = reader.references();
            let start = refs.partition_point(|r| r.span().file_id() < fid);
            let count = refs[start..].partition_point(|r| r.span().file_id() == fid);
            let file_refs = &refs[start..start + count];

            for r in file_refs {
                if r.is_macro_expansion() {
                    continue;
                }
                let r_kind = DefKind::from(r.kind);
                let path = reader.get_str(r.target_path_offset());
                if path.is_empty() {
                    continue;
                }

                if r_kind == DefKind::Use && path.ends_with("::*") {
                    let raw_prefix = path.trim_end_matches("::*");
                    let prefix = resolve_module_prefix(ws, file_path, reader.crate_name(), raw_prefix);
                    expand_glob_traits(ws, &prefix, reader.crate_name(), &mut set, 0);
                } else if r_kind == DefKind::Use && !path.ends_with("::*") {
                    let resolved = resolve_module_prefix(ws, file_path, reader.crate_name(), path);
                    let target = ws.find_definition_by_path(&resolved).or_else(|| ws.find_definition_by_path(path));
                    if let Some(target) = target {
                        if target.kind == DefKind::Trait {
                            if let Some(short) = path.rsplit("::").next() {
                                set.insert(short.to_string());
                            }
                            set.insert(path.to_string());
                            let t_path = target.path;
                            if let Some(short) = t_path.rsplit("::").next() {
                                set.insert(short.to_string());
                            }
                            set.insert(t_path.to_string());
                        }
                    }
                } else if r_kind == DefKind::Trait && use_lines.iter().any(|&(s, e)| (s..=e).contains(&r.span().start_line())) {
                    // The trait named in a `use` item. A trait named in an impl header, a bound or a path call
                    // (`impl fmt::Debug for X`, `T: Hash`) does not bring its methods into scope.
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

#[cfg(test)]
mod use_item_tests {
    use super::use_item_lines;

    #[test]
    fn test_use_item_lines_cover_groups() {
        let text = "use std::fmt;\nuse hir::{\n    Adt,\n    HasSource,\n};\nfn f() {}\npub use x::Y;\n";
        assert_eq!(use_item_lines(text), vec![(1, 1), (2, 5), (7, 7)]);
    }
}
