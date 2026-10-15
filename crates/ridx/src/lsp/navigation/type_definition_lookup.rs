/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Canonical type path lookup, candidate expansion, and homonymous crate ranking
//! for `textDocument/typeDefinition`.

use super::type_definition::is_type_def_kind;
use crate::lsp::protocol::{path_to_uri, PositionConverter};
use ridx_format::workspace::std_map::std_path_candidates;
use ridx_format::{join_resolving_overlap, DefEntry, DefKind, IndexReader, WorkspaceIndex};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

type BestScore = (bool, bool, bool, bool, bool, bool);
type BestHit<'a> = (BestScore, PathBuf, &'a str, (u32, u32, u32, u32));

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

fn find_matching_type_defs<'a, B: AsRef<[u8]>>(
    reader: &'a IndexReader<B>,
    query_path: &str,
) -> Vec<&'a DefEntry> {
    let mut results = Vec::new();
    if let Some(def) = reader.find_definition_by_path(query_path) {
        if is_type_def_kind(DefKind::from(def.kind)) {
            results.push(def);
        }
    }
    if let Some((parent, _)) = query_path.rsplit_once("::") {
        for def in reader.find_definitions_by_owner(parent) {
            if reader.get_str(def.path_offset()) == query_path
                && is_type_def_kind(DefKind::from(def.kind))
                && !results.iter().any(|existing| std::ptr::eq(*existing, def))
            {
                results.push(def);
            }
        }
    }
    results
}

pub(super) fn lookup_type_by_path(
    ws: &WorkspaceIndex,
    file_readers: &[usize],
    query_file: &str,
    target_path: &str,
    is_canonical_owner: bool,
    conv: &mut PositionConverter,
) -> Option<Value> {
    let query_path = Path::new(query_file);
    let crate_units: Vec<(&str, &str)> = file_readers
        .iter()
        .filter_map(|&idx| ws.readers().get(idx))
        .map(|reader| (reader.crate_name(), reader.unit_key()))
        .collect();

    let in_same_crate = |idx: usize| -> bool {
        ws.readers().get(idx).is_some_and(|reader| {
            let (crate_name, unit_key) = (reader.crate_name(), reader.unit_key());
            crate_units.iter().any(|(c, u)| {
                (!crate_name.is_empty() && *c == crate_name)
                    || (!unit_key.is_empty() && *u == unit_key)
            })
        })
    };

    let candidates = expand_candidates(target_path);
    let mut best: Option<BestHit<'_>> = None;

    for priority_only in [true, false] {
        for cand in &candidates {
            let first_seg = cand.split("::").next().unwrap_or("");
            for (reader_idx, reader) in ws.readers().iter().enumerate() {
                let crate_name = reader.crate_name();
                let same_crate = in_same_crate(reader_idx);
                let crate_match = !first_seg.is_empty() && crate_name == first_seg;
                if (same_crate || crate_match) != priority_only {
                    continue;
                }
                let mut in_dep_opt: Option<bool> = None;
                for query_cand in &candidate_queries(cand, crate_name) {
                    for def in find_matching_type_defs(reader, query_cand) {
                        let span = def.span();
                        let raw_file = reader.get_file(span.file_id());
                        if raw_file == "external" || raw_file.is_empty() {
                            continue;
                        }
                        let in_dep_graph = *in_dep_opt.get_or_insert_with(|| {
                            ws.is_reader_in_dep_graph(file_readers, reader_idx)
                        });
                        let resolved_path = resolve_reader_file(ws, reader_idx, raw_file);
                        let is_local = !ws.is_reader_dependency(reader_idx);
                        let same_file = resolved_path == query_path;
                        let score = (
                            !def.is_reexport(),
                            same_file,
                            same_crate,
                            in_dep_graph,
                            crate_match,
                            is_local,
                        );
                        if best
                            .as_ref()
                            .is_none_or(|(best_score, ..)| score > *best_score)
                        {
                            let coords = (
                                span.start_line(),
                                span.start_col(),
                                span.end_line(),
                                span.end_col(),
                            );
                            best = Some((score, resolved_path, raw_file, coords));
                        }
                    }
                }
            }
            if best.as_ref().is_some_and(|(best_score, ..)| best_score.0) && is_canonical_owner {
                break;
            }
        }
        if best.as_ref().is_some_and(|(best_score, ..)| best_score.0) {
            break;
        }
    }

    let (_, resolved_path, raw_file, coords) = best?;
    Some(format_location(&resolved_path, raw_file, coords, conv))
}

fn candidate_queries(cand: &str, crate_name: &str) -> Vec<String> {
    match (cand.contains("::"), crate_name.is_empty()) {
        (true, false) => {
            let alt = cand
                .strip_prefix(&format!("{crate_name}::"))
                .map_or_else(|| format!("{crate_name}::{cand}"), str::to_string);
            vec![cand.to_string(), alt]
        }
        (false, false) => vec![format!("{crate_name}::{cand}"), cand.to_string()],
        _ => vec![cand.to_string()],
    }
}

pub(super) fn resolve_reader_file(
    ws: &WorkspaceIndex,
    reader_idx: usize,
    raw_file: &str,
) -> PathBuf {
    let rel_path = Path::new(raw_file);
    if rel_path.is_absolute() {
        return rel_path.to_path_buf();
    }
    let primary = ws
        .source_root_at(reader_idx)
        .or_else(|| ws.workspace_root())
        .map_or_else(
            || rel_path.to_path_buf(),
            |root| join_resolving_overlap(root, rel_path),
        );
    match (!primary.exists(), ws.workspace_root()) {
        (true, Some(root)) if join_resolving_overlap(root, rel_path).exists() => {
            join_resolving_overlap(root, rel_path)
        }
        _ => primary,
    }
}

pub(super) fn format_location(
    resolved_path: &Path,
    raw_file: &str,
    (start_line, start_col, end_line, end_col): (u32, u32, u32, u32),
    conv: &mut PositionConverter,
) -> Value {
    let path_str = resolved_path.to_str().unwrap_or(raw_file);
    let start_col_u16 = conv.char_col_to_utf16_col(path_str, start_line, start_col);
    let end_col_u16 = conv.char_col_to_utf16_col(path_str, end_line, end_col);
    json!({
        "uri": path_to_uri(path_str),
        "range": {
            "start": { "line": start_line.saturating_sub(1), "character": start_col_u16 },
            "end": { "line": end_line.saturating_sub(1), "character": end_col_u16 }
        }
    })
}
