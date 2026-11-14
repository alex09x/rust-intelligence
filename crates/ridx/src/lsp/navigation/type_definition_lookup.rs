/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Canonical type path lookup, candidate expansion, `TyAlias` unwrapping, and homonymous crate
//! ranking for `textDocument/typeDefinition`.

use super::type_definition_index::get_ws_type_index;
use super::type_definition_parse::{extract_rmeta_default_generics, lookup_unstable_sysroot_type};
use crate::lsp::protocol::{path_to_uri, PositionConverter};
use memmap2::Mmap;
use ridx_format::workspace::std_map::std_path_candidates;
use ridx_format::{join_resolving_overlap, DefEntry, DefKind, IndexReader, WorkspaceIndex};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

type BestScore = (bool, bool, bool, bool, bool, bool, bool);
pub(super) type TypeHit<'a> = (usize, &'a DefEntry, PathBuf, &'a str, (u32, u32, u32, u32));
type ScoredHit<'a> = (BestScore, TypeHit<'a>);

static FILE_LOC_META: OnceLock<Mutex<HashMap<PathBuf, String>>> = OnceLock::new();

pub(super) fn clear_ws_type_index() {
    super::type_definition_index::clear_type_indexes();
    if let Some(mut guard) = FILE_LOC_META.get().and_then(|l| l.lock().ok()) {
        guard.clear();
    }
}

const PRELUDE_TYPES: [(&str, &str); 7] = [
    ("Option", "core::option::Option"),
    ("Result", "core::result::Result"),
    ("Box", "alloc::boxed::Box"),
    ("String", "alloc::string::String"),
    ("Vec", "alloc::vec::Vec"),
    ("Arc", "alloc::sync::Arc"),
    ("Rc", "alloc::rc::Rc"),
];

fn expand_candidates(path: &str) -> Vec<String> {
    let mut out = std_path_candidates(path);
    if let Some(rest) = path.strip_prefix("core::") {
        out.extend([format!("std::{rest}"), format!("alloc::{rest}")]);
    } else if let Some(rest) = path.strip_prefix("alloc::") {
        out.extend([format!("std::{rest}"), format!("core::{rest}")]);
    } else if let Some((_, canonical)) = PRELUDE_TYPES.into_iter().find(|(s, _)| *s == path) {
        out.push(canonical.to_string());
    }
    out
}

pub(super) fn lookup_type_hit<'a>(ws: &'a WorkspaceIndex, file_readers: &[usize], query_file: &str, target_path: &str, is_canonical_owner: bool) -> Option<TypeHit<'a>> {
    let query_path = Path::new(query_file);
    let crate_units: Vec<(&str, &str)> = file_readers.iter().filter_map(|&idx| ws.readers().get(idx)).map(|reader| (reader.crate_name(), reader.unit_key())).collect();

    let in_same_crate = |idx: usize| -> bool {
        ws.readers().get(idx).is_some_and(|reader| {
            let (crate_name, unit_key) = (reader.crate_name(), reader.unit_key());
            crate_units.iter().any(|(c, u)| (!crate_name.is_empty() && *c == crate_name) || (!unit_key.is_empty() && *u == unit_key))
        })
    };

    let candidates = expand_candidates(target_path);
    let mut best: Option<ScoredHit<'a>> = None;
    let type_idx = get_ws_type_index(ws);
    let readers = ws.readers();

    for priority_only in [true, false] {
        for cand in &candidates {
            let first_seg = cand.split("::").next().unwrap_or("");
            let short = cand.rsplit("::").next().unwrap_or(cand);
            let Some(hits) = type_idx.by_short_name.get(short) else {
                continue;
            };
            for &(r_u32, d_u32) in hits {
                let reader_idx = r_u32 as usize;
                let reader = &readers[reader_idx];
                let crate_name = reader.crate_name();
                let same_crate = in_same_crate(reader_idx);
                let crate_match = !first_seg.is_empty() && crate_name == first_seg;
                if (same_crate || crate_match) != priority_only {
                    continue;
                }
                let def = &reader.definitions()[d_u32 as usize];
                let def_path = reader.get_str(def.path_offset());
                if !candidate_queries(cand, crate_name).iter().any(|q| q == def_path) {
                    continue;
                }
                let raw_file = reader.get_file(def.span().file_id());
                let in_dep_graph = ws.is_reader_in_dep_graph(file_readers, reader_idx);
                update_best_hit(ws, reader_idx, def, raw_file, query_path, (true, same_crate, in_dep_graph, crate_match), &mut best);
            }
            if best.as_ref().is_some_and(|(best_score, _)| best_score.0) && is_canonical_owner {
                break;
            }
        }
        if best.as_ref().is_some_and(|(best_score, _)| best_score.0) {
            break;
        }
    }

    if best.as_ref().is_none_or(|(best_score, _)| !best_score.0) {
        lookup_short_name_fallback(ws, file_readers, query_path, &candidates, &in_same_crate, &mut best);
    }

    best.map(|(_, hit)| hit)
}

fn lookup_short_name_fallback<'a>(
    ws: &'a WorkspaceIndex,
    file_readers: &[usize],
    query_path: &Path,
    candidates: &[String],
    in_same_crate: &impl Fn(usize) -> bool,
    best: &mut Option<ScoredHit<'a>>,
) {
    let first_cand = candidates.first().map_or("", String::as_str);
    let all_segs: Vec<&str> = first_cand.split("::").filter(|s| !s.is_empty()).collect();
    let Some(&short_name) = all_segs.last() else {
        return;
    };
    let type_idx = get_ws_type_index(ws);
    let Some(hits) = type_idx.by_short_name.get(short_name) else {
        return;
    };
    let first_seg = if all_segs.len() > 1 { all_segs[0] } else { "" };
    // A bare name the query file declares as a type parameter (`impl<I, O> … for E<I, O>`) is that parameter, which
    // has no type definition; it is not looked up by name across the workspace.
    if all_segs.len() == 1
        && file_readers.iter().filter_map(|&i| ws.readers().get(i).map(|r| (i, r))).any(|(i, r)| {
            r.definitions()
                .iter()
                .any(|d| d.kind == DefKind::TyParam as u8 && r.get_str(d.name_offset()) == short_name && resolve_reader_file(ws, i, r.get_file(d.span().file_id())) == query_path)
        })
    {
        return;
    }
    let is_sysroot_query = matches!(first_seg, "std" | "core" | "alloc");
    let mod_segs: &[&str] = if is_sysroot_query {
        &all_segs[1..all_segs.len() - 1]
    } else if all_segs.len() > 1 {
        &all_segs[..all_segs.len() - 1]
    } else {
        &[]
    };
    let readers = ws.readers();
    for &(r_u32, d_u32) in hits {
        let reader_idx = r_u32 as usize;
        let reader = &readers[reader_idx];
        let crate_name = reader.crate_name();
        let is_sysroot_reader = matches!(crate_name, "core" | "alloc" | "std");
        if is_sysroot_query && !is_sysroot_reader {
            continue;
        }
        let same_crate = in_same_crate(reader_idx);
        let crate_match = (is_sysroot_query && is_sysroot_reader) || (!first_seg.is_empty() && (crate_name == first_seg || crate_name.starts_with(&format!("{first_seg}_"))));
        let in_dep_graph = is_sysroot_reader || ws.is_reader_in_dep_graph(file_readers, reader_idx);
        if !same_crate && !crate_match && !in_dep_graph {
            continue;
        }
        let def = &reader.definitions()[d_u32 as usize];
        // A type parameter has no type definition, and a definition is found under its own name only.
        if def.kind == DefKind::TyParam as u8 || reader.get_str(def.name_offset()) != short_name {
            continue;
        }
        let span = def.span();
        let raw_file = reader.get_file(span.file_id());
        let def_path = reader.get_str(def.path_offset());
        let mod_match = if is_sysroot_query {
            mod_segs.is_empty() || mod_segs.iter().all(|&m| def_path.contains(&format!("::{m}::")))
        } else {
            mod_segs.iter().any(|&s| s != crate_name && (def_path.contains(&format!("::{s}::")) || def_path.starts_with(&format!("{s}::"))))
        };
        if is_sysroot_query && !mod_match {
            continue;
        }
        update_best_hit(ws, reader_idx, def, raw_file, query_path, (mod_match, same_crate, in_dep_graph, crate_match), best);
    }
}

fn update_best_hit<'a>(
    ws: &'a WorkspaceIndex,
    reader_idx: usize,
    def: &'a DefEntry,
    raw_file: &'a str,
    query_path: &Path,
    (mod_match, same_crate, in_dep_graph, crate_match): (bool, bool, bool, bool),
    best: &mut Option<ScoredHit<'a>>,
) {
    let span = def.span();
    let resolved_path = resolve_reader_file(ws, reader_idx, raw_file);
    let score = (!def.is_reexport(), mod_match, resolved_path == query_path, same_crate, in_dep_graph, crate_match, !ws.is_reader_dependency(reader_idx));
    if best.as_ref().is_none_or(|(best_score, _)| score > *best_score) {
        let coords = (span.start_line(), span.start_col(), span.end_line(), span.end_col());
        *best = Some((score, (reader_idx, def, resolved_path, raw_file, coords)));
    }
}

pub(super) fn lookup_type_by_path(
    ws: &WorkspaceIndex,
    file_readers: &[usize],
    query_file: &str,
    target_path: &str,
    is_canonical_owner: bool,
    conv: &mut PositionConverter,
) -> Option<Value> {
    if let Some((_, _, resolved_path, raw_file, coords)) = lookup_type_hit(ws, file_readers, query_file, target_path, is_canonical_owner) {
        return Some(format_location(&resolved_path, raw_file, coords, conv));
    }
    lookup_unstable_sysroot_type(ws, target_path, conv)
}

/// Collects target paths referenced on the RHS or generic default parameters of a `TyAlias` or `Struct`/`Enum`.
pub(super) fn collect_alias_or_default_targets(
    reader: &IndexReader<Mmap>,
    def: &DefEntry,
    resolved_file: &Path,
    only_default_generics: bool,
    conv: &mut PositionConverter,
) -> Vec<String> {
    let span = def.span();
    let file_id = span.file_id();
    let s_line = span.start_line();
    let path_str = resolved_file.to_str().unwrap_or("");

    if only_default_generics {
        if reader.references().is_empty() {
            return extract_rmeta_default_generics(resolved_file, s_line, conv);
        }
        if collect_type_refs_in_range(reader, file_id, (s_line, span.end_col()), (s_line, u32::MAX), false).is_empty() {
            return Vec::new();
        }
        let first_line = conv.get_line(path_str, s_line).unwrap_or("");
        let Some((min_col, max_col)) = default_generic_col_bounds(first_line, span.end_col()) else {
            return Vec::new();
        };
        return collect_type_refs_in_range(reader, file_id, (s_line, min_col), (s_line, max_col), false);
    }

    let first_line = conv.get_line(path_str, s_line).unwrap_or("").to_string();

    let mut end_line = span.end_line().max(s_line);
    if !first_line.contains(';') {
        for ln in (s_line + 1)..=(s_line + 5) {
            let Some(l_txt) = conv.get_line(path_str, ln) else { break };
            end_line = ln;
            if l_txt.contains(';') {
                break;
            }
        }
    }
    collect_type_refs_in_range(reader, file_id, (s_line, span.end_col()), (end_line, u32::MAX), true)
}

fn default_generic_col_bounds(line_str: &str, name_end_col_1based: u32) -> Option<(u32, u32)> {
    let chars: Vec<char> = line_str.chars().collect();
    let mut i = (name_end_col_1based as usize).saturating_sub(1);
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    if i >= chars.len() || chars[i] != '<' {
        return None;
    }
    let start_col = (i as u32) + 1;
    let mut depth = 0i32;
    let mut has_eq = false;
    for (idx, &ch) in chars.iter().enumerate().skip(i) {
        match ch {
            '<' => depth += 1,
            '>' => {
                depth -= 1;
                if depth == 0 {
                    return has_eq.then_some((start_col, (idx as u32) + 1));
                }
            }
            '=' if depth == 1 => has_eq = true,
            _ => {}
        }
    }
    None
}

pub(super) fn collect_type_refs_in_range(
    reader: &IndexReader<Mmap>,
    file_id: u32,
    (start_line, start_col): (u32, u32),
    (end_line, end_col): (u32, u32),
    include_traits: bool,
) -> Vec<String> {
    let refs = reader.references();
    let lo = refs.partition_point(|r| {
        let s = r.span();
        (s.file_id(), s.start_line(), s.start_col()) < (file_id, start_line, start_col)
    });
    let mut out = Vec::new();
    for r in &refs[lo..] {
        let s = r.span();
        if s.file_id() != file_id || (s.start_line(), s.start_col()) > (end_line, end_col) {
            break;
        }
        let kind = DefKind::from(r.kind);
        let raw_tp = reader.get_str(r.target_path_offset());
        let tp_opt = match kind {
            DefKind::Struct | DefKind::StructCtor | DefKind::Enum | DefKind::Union | DefKind::TyAlias => Some(raw_tp.to_string()),
            DefKind::Trait if include_traits => Some(raw_tp.to_string()),
            DefKind::AssocTy if include_traits => raw_tp.rsplit_once("::").map(|(p, _)| p.to_string()),
            _ => None,
        };
        if let Some(tp) = tp_opt {
            if !out.contains(&tp) {
                out.push(tp);
            }
        }
    }
    out
}

fn candidate_queries(cand: &str, crate_name: &str) -> Vec<String> {
    match (cand.contains("::"), crate_name.is_empty()) {
        (true, false) => {
            let alt = cand.strip_prefix(&format!("{crate_name}::")).map_or_else(|| format!("{crate_name}::{cand}"), str::to_string);
            vec![cand.to_string(), alt]
        }
        (false, false) => vec![format!("{crate_name}::{cand}"), cand.to_string()],
        _ => vec![cand.to_string()],
    }
}

pub(super) fn resolve_reader_file(ws: &WorkspaceIndex, reader_idx: usize, raw_file: &str) -> PathBuf {
    let rel_path = Path::new(raw_file);
    if rel_path.is_absolute() {
        return rel_path.to_path_buf();
    }
    if let Some(root) = ws.source_root_at(reader_idx) {
        return join_resolving_overlap(root, rel_path);
    }
    if !ws.is_reader_dependency(reader_idx) {
        if let Some(root) = ws.workspace_root() {
            return join_resolving_overlap(root, rel_path);
        }
    }
    rel_path.to_path_buf()
}

pub(super) fn format_location(resolved_path: &Path, raw_file: &str, (start_line, start_col, end_line, end_col): (u32, u32, u32, u32), conv: &mut PositionConverter) -> Value {
    let path_str = resolved_path.to_str().unwrap_or(raw_file);
    let s_u16 = conv.char_col_to_utf16_col(path_str, start_line, start_col);
    let e_u16 = conv.char_col_to_utf16_col(path_str, end_line, end_col);
    let uri = {
        let mut g = FILE_LOC_META.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap_or_else(|e| e.into_inner());
        g.entry(resolved_path.to_path_buf()).or_insert_with(|| path_to_uri(path_str)).clone()
    };
    json!({ "uri": uri, "range": { "start": { "line": start_line.saturating_sub(1), "character": s_u16 }, "end": { "line": end_line.saturating_sub(1), "character": e_u16 } } })
}
