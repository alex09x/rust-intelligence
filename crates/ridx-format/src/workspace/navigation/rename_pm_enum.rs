/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Semantic module and re-export path-prefix resolution for enum references inside `proc-macro` crates.

use super::qualified_path::extract_root_crate;
use super::trait_impl_refs::strip_generics;
use super::*;
use crate::workspace::cursor_path::is_ident_byte;
use crate::workspace::rename_cache::RenameWorkspaceCache;
use crate::workspace::rename_parse::parse_file_imports;
use std::collections::HashSet;

fn read_mod_source(ws: &WorkspaceIndex, crate_name: &str, mod_path: &str) -> Option<String> {
    for &ri in ws.readers_for_crate_fast(crate_name).iter() {
        if ws.is_reader_external(ri) {
            continue;
        }
        let r = &ws.readers[ri];
        let mut src_roots = Vec::new();
        for fid in 0..r.header().files_count {
            let f = r.get_file(fid);
            let s = if let Some((pre, _)) = f.rsplit_once("/src/") {
                format!("{pre}/src")
            } else if f.starts_with("src/") {
                "src".to_string()
            } else {
                continue;
            };
            if !src_roots.contains(&s) {
                src_roots.push(s);
            }
        }
        let rel_candidates = if mod_path == crate_name {
            vec!["lib.rs".to_string()]
        } else {
            let local_m = mod_path.strip_prefix(&format!("{crate_name}::")).unwrap_or(mod_path).replace("::", "/");
            vec![format!("{local_m}/mod.rs"), format!("{local_m}.rs")]
        };
        for sr in &src_roots {
            for rel in &rel_candidates {
                let p = format!("{sr}/{rel}");
                if let Some(src) = ws.read_source_file(&p).or_else(|| ws.resolve_path_for_crate(crate_name, &p).to_str().and_then(|s| ws.read_source_file(s))) {
                    return Some(src);
                }
            }
        }
    }
    None
}

/// Collects canonical module paths (`crate::mod_a::mod_b`) that directly define or publicly re-export `def`.
pub(super) fn collect_enum_valid_mod_paths(ws: &WorkspaceIndex, cache: &RenameWorkspaceCache, def: &ResolvedDefinition<'_>) -> HashSet<String> {
    let mut mod_paths = HashSet::new();
    if def.kind != DefKind::Enum || def.name.is_empty() {
        return mod_paths;
    }
    let def_crate = def.reader_idx.and_then(|ri| ws.readers.get(ri)).map(|r| r.crate_name()).unwrap_or_else(|| extract_root_crate(def.path));
    if def_crate.is_empty() {
        return mod_paths;
    }
    let clean = strip_generics(def.path);
    let full_def = if clean.starts_with(&format!("{def_crate}::")) { clean.to_string() } else { format!("{def_crate}::{clean}") };
    let Some((orig_mod, _)) = full_def.rsplit_once("::") else { return mod_paths };
    mod_paths.insert(orig_mod.to_string());

    let needle = def.name.to_ascii_lowercase();
    for (ri, r) in ws.readers.iter().enumerate() {
        if ws.is_reader_external(ri) || !cache.cargo_targets.crate_can_reexport(r.crate_name(), def_crate) {
            continue;
        }
        let rc = r.crate_name();
        for d in r.search_symbols_ascii(needle.as_bytes()) {
            if r.get_str(d.name_offset()) != def.name {
                continue;
            }
            let dk = DefKind::from(d.kind);
            if !matches!(dk, DefKind::Enum | DefKind::Use) {
                continue;
            }
            if dk == DefKind::Enum && !d.is_reexport() {
                let f = r.get_file(d.span().file_id());
                if d.span().start_line() != def.start_line || !(f == def.file || f.ends_with(def.file) || def.file.ends_with(f)) {
                    continue;
                }
            }
            let dp = r.get_str(d.path_offset());
            let full_p = if dp.starts_with(&format!("{rc}::")) { dp.to_string() } else { format!("{rc}::{dp}") };
            if let Some((m, _)) = full_p.rsplit_once("::") {
                mod_paths.insert(m.to_string());
            }
        }
    }

    // Walk ancestor modules inside `def_crate` for `pub use ...::EnumName` or `pub use child::*`
    let mut cur = orig_mod.to_string();
    while let Some((anc, child_seg)) = cur.rsplit_once("::") {
        if let Some(src) = read_mod_source(ws, def_crate, anc) {
            let (exp, globs) = parse_file_imports(&src, def_crate, true);
            let child_full = format!("{anc}::{child_seg}");
            let reexported_explicit = exp.get(def.name).is_some_and(|ps| {
                ps.iter().any(|p| {
                    p.ends_with(&format!("::{child_seg}::{}", def.name))
                        || p == &format!("{child_seg}::{}", def.name)
                        || p == &format!("self::{child_seg}::{}", def.name)
                        || mod_paths.iter().any(|m| p == &format!("{m}::{}", def.name))
                })
            });
            let reexported_glob = globs.iter().any(|g| g == &child_full || g == child_seg || g == &format!("self::{child_seg}") || mod_paths.contains(g));
            if reexported_explicit || reexported_glob {
                mod_paths.insert(anc.to_string());
            }
        }
        cur = anc.to_string();
    }

    // Propagate across workspace facade crates that `pub use def_crate::*` or `pub use def_crate::submod`
    let base_mods: Vec<String> = mod_paths.iter().cloned().collect();
    let mut seen_crates = HashSet::new();
    for (ri, r) in ws.readers.iter().enumerate() {
        let rc = r.crate_name();
        if ws.is_reader_external(ri) || rc == def_crate || !cache.cargo_targets.crate_can_reexport(rc, def_crate) || !seen_crates.insert(rc) {
            continue;
        }
        let Some(lib_src) = read_mod_source(ws, rc, rc) else { continue };
        let (exp, globs) = parse_file_imports(&lib_src, rc, true);
        let glob_all = globs.iter().any(|g| g == def_crate);
        for bm in &base_mods {
            if bm == def_crate {
                if glob_all || exp.get(def.name).is_some_and(|ps| ps.iter().any(|p| p == &format!("{def_crate}::{}", def.name))) {
                    mod_paths.insert(rc.to_string());
                }
            } else if let Some(rel) = bm.strip_prefix(&format!("{def_crate}::")) {
                let top_seg = rel.split("::").next().unwrap_or("");
                let top_reexported = glob_all || exp.get(top_seg).is_some_and(|ps| ps.iter().any(|p| p == &format!("{def_crate}::{top_seg}")));
                if top_reexported {
                    mod_paths.insert(format!("{rc}::{rel}"));
                }
            }
        }
    }
    mod_paths
}

fn extract_path_prefix_before_colons(before: &str) -> Option<(bool, &str)> {
    let stem = before.strip_suffix("::")?;
    let bytes = stem.as_bytes();
    let mut pos = bytes.len();
    while pos > 0 {
        if is_ident_byte(bytes[pos - 1]) {
            pos -= 1;
        } else if pos >= 2 && bytes[pos - 1] == b':' && bytes[pos - 2] == b':' {
            pos -= 2;
        } else {
            break;
        }
    }
    let raw = stem[pos..].trim_start_matches("::");
    if raw.is_empty() {
        return None;
    }
    if pos > 0 && bytes[pos - 1] == b'#' {
        let (_, rest) = raw.split_once("::")?;
        return (!rest.is_empty()).then_some((true, rest));
    }
    Some((false, raw))
}

/// Returns `true` when `before` ends with `<mod_prefix>::` and `<mod_prefix>` resolves to a module
/// defining or re-exporting `def`.
pub(super) fn pm_enum_prefix_resolves(before: &str, pm_crate: &str, pm_content: &str, def_crate: &str, cache: &RenameWorkspaceCache, valid_mod_paths: &HashSet<String>) -> bool {
    if valid_mod_paths.is_empty() {
        return false;
    }
    let Some((has_quote_crate_var, prefix)) = extract_path_prefix_before_colons(before) else {
        return false;
    };
    if has_quote_crate_var {
        return valid_mod_paths.iter().any(|m| m.split_once("::").is_some_and(|(cr, rest)| rest == prefix && cache.cargo_targets.crate_can_reexport(cr, def_crate)));
    }
    if valid_mod_paths.contains(prefix) {
        return true;
    }
    let (pm_imports, pm_globs) = parse_file_imports(pm_content, pm_crate, false);
    let (first_seg, tail) = match prefix.split_once("::") {
        Some((f, t)) => (f, Some(t)),
        None => (prefix, None),
    };
    if let Some(imps) = pm_imports.get(first_seg) {
        if imps.iter().any(|imp| {
            let resolved = match tail {
                Some(t) => format!("{imp}::{t}"),
                None => imp.clone(),
            };
            valid_mod_paths.contains(&resolved)
        }) {
            return true;
        }
    }
    pm_globs.iter().any(|g| valid_mod_paths.contains(&format!("{g}::{prefix}")))
}
