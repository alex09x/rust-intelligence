/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP `textDocument/typeDefinition` handler using hash lookup in each reader (`defs_by_hash` / `defs_by_owner_hash`), no full-text scan.

use crate::lsp::protocol::{path_to_uri, uri_to_path, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::workspace::std_map::std_path_candidates;
use ridx_format::{join_resolving_overlap, DefEntry, DefKind, IndexReader, WorkspaceIndex};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

type BestHit<'a> = (
    (bool, bool, bool, bool, bool),
    PathBuf,
    &'a str,
    (u32, u32, u32, u32),
);

const PRIMITIVES: &[&str] = &[
    "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize", "f16",
    "f32", "f64", "f128", "bool", "char", "str", "slice", "array", "pointer", "tuple", "unit",
    "never", "!", "()",
];

const PRELUDE_TYPES: [(&str, &str); 7] = [
    ("Option", "core::option::Option"),
    ("Result", "core::result::Result"),
    ("Box", "alloc::boxed::Box"),
    ("String", "alloc::string::String"),
    ("Vec", "alloc::vec::Vec"),
    ("Arc", "alloc::sync::Arc"),
    ("Rc", "alloc::rc::Rc"),
];

/// Resolves `textDocument/typeDefinition` at the requested cursor position via hash lookup in each reader.
///
/// # Compound Type Resolution Rules
/// - **Modifiers (`&Foo`, `&mut Foo`, `&'a Foo`, `Foo<'a>`, `dyn Trait`)**: Stripped to `Foo` / `Trait`.
/// - **Generic wrappers (`Wrapper<i32>`, `Option<Foo>`, `Result<Foo, Bar>`, `Box<Foo>`)**: Uniformly
///   resolve to the **outer** type constructor (`Wrapper`, `Option`, `Result`, `Box`).
/// - **Primitives and built-ins (`u32`, `bool`, `str`, `!`, `()`)**: Return `Value::Null`.
/// - **Generic type parameters (`TyParam`, e.g. `T`)**: Resolved via [`WorkspaceIndex::find_type_param_in_scope`].
/// - **Canonical `owner_path` lookup**: Hash lookup in each reader (`defs_by_hash` / `defs_by_owner_hash`), no full-text scan.
/// - **Type identifier fallback**: Falls back to [`WorkspaceIndex::find_definition_at`].
pub fn handle_type_definition(
    msg: &Value,
    workspace: Option<&WorkspaceIndex>,
    overlay_mgr: &OverlayManager,
    conv: &mut PositionConverter,
) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);
    let line_1based = msg["params"]["position"]["line"].as_u64().unwrap_or(0) as u32 + 1;
    let utf16_col = msg["params"]["position"]["character"].as_u64().unwrap_or(0) as u32;

    let overlay_opt = overlay_mgr.get(&file);
    let col_1based = match overlay_opt {
        Some(overlay) => overlay.utf16_col_to_char_col(line_1based, utf16_col),
        None => conv.utf16_col_to_char_col(&file, line_1based, utf16_col),
    };
    let Some(ws) = workspace.filter(|_| !overlay_opt.is_some_and(|o| o.is_dirty())) else {
        return Value::Null;
    };

    let mut file_readers: Vec<usize> = ws
        .reader_indices_for_file(&file)
        .into_iter()
        .filter(|&i| ws.target_file_for_reader(i, &file).is_some())
        .collect();
    if file_readers.is_empty() {
        file_readers = ws.candidate_reader_indices_for_file(&file);
    }
    let pos = (line_1based, col_1based);

    if let Some((ty_name, owner_opt)) = ws.type_entry_at(&file, line_1based, col_1based) {
        if is_enclosing_type_span_multiline(ws, &file_readers, &file, pos) {
            if let Some(loc) = resolve_from_definition_at(ws, &file_readers, &file, pos, conv) {
                return loc;
            }
        }

        let base_ty = strip_generics(peel_type_modifiers(ty_name));
        let is_prim =
            is_primitive_or_builtin(base_ty) || owner_opt.is_some_and(is_primitive_or_builtin);
        if !is_prim {
            if !base_ty.contains("::") {
                if let Some(tp) =
                    ws.find_type_param_in_scope(&file, base_ty, line_1based, col_1based)
                {
                    if tp.file != "external" {
                        let c = (tp.start_line, tp.start_col, tp.end_line, tp.end_col);
                        return format_location(&tp.resolved_path(), tp.file, c, conv);
                    }
                }
            }
            if let Some(owner_base) = owner_opt
                .map(|o| strip_generics(peel_type_modifiers(o)))
                .filter(|s| !s.is_empty())
            {
                if let Some(loc) =
                    lookup_type_by_path(ws, &file_readers, &file, owner_base, true, conv)
                {
                    return loc;
                }
            }
            if !base_ty.is_empty() {
                if let Some(loc) =
                    lookup_type_by_path(ws, &file_readers, &file, base_ty, false, conv)
                {
                    return loc;
                }
            }
        }
    }

    resolve_from_definition_at(ws, &file_readers, &file, pos, conv).unwrap_or(Value::Null)
}

fn is_enclosing_type_span_multiline(
    ws: &WorkspaceIndex,
    file_readers: &[usize],
    file: &str,
    (line, col): (u32, u32),
) -> bool {
    file_readers.iter().any(|&i| {
        ws.target_file_for_reader(i, file)
            .and_then(|tf| ws.readers()[i].find_innermost_type_entry(tf.as_ref(), line, col))
            .is_some_and(|e| e.span().start_line() != e.span().end_line())
    })
}

fn resolve_from_definition_at(
    ws: &WorkspaceIndex,
    file_readers: &[usize],
    file: &str,
    (line, col): (u32, u32),
    conv: &mut PositionConverter,
) -> Option<Value> {
    let mut needs_cross = false;
    for &i in file_readers {
        let Some(tf) = ws.target_file_for_reader(i, file) else {
            continue;
        };
        let Some(r) = ws.readers()[i].find_definition_at(tf.as_ref(), line, col) else {
            continue;
        };
        if matches!(r.kind, DefKind::Variant | DefKind::VariantCtor) {
            let (enum_path, _) = r.path.rsplit_once("::")?;
            return lookup_type_by_path(ws, file_readers, file, enum_path, true, conv);
        }
        if !is_type_def_kind(r.kind) && r.kind != DefKind::Use {
            return None;
        }
        if r.file != "external"
            && !r.is_reexport
            && is_type_def_kind(r.kind)
            && !is_primitive_or_builtin(r.name)
        {
            let c = (r.start_line, r.start_col, r.end_line, r.end_col);
            let res = resolve_reader_file(ws, i, r.file);
            return Some(format_location(&res, r.file, c, conv));
        }
        needs_cross = true;
        break;
    }
    if !needs_cross && !file_readers.is_empty() {
        return None;
    }
    let def = ws.find_definition_at(file, line, col)?;
    if matches!(def.kind, DefKind::Variant | DefKind::VariantCtor) {
        let (enum_path, _) = def.path.rsplit_once("::")?;
        return lookup_type_by_path(ws, file_readers, file, enum_path, true, conv);
    }
    if is_type_def_kind(def.kind) && !is_primitive_or_builtin(def.name) {
        if def.file != "external" && !def.is_reexport {
            let c = (def.start_line, def.start_col, def.end_line, def.end_col);
            return Some(format_location(&def.resolved_path(), def.file, c, conv));
        }
        return lookup_type_by_path(ws, file_readers, file, def.path, true, conv);
    }
    None
}

fn is_type_def_kind(k: DefKind) -> bool {
    use DefKind::*;
    [Struct, StructCtor, Enum, Union, Trait, TyAlias, TyParam].contains(&k)
}

fn is_primitive_or_builtin(ty: &str) -> bool {
    let t = ty.trim();
    t.is_empty()
        || t.starts_with(['[', '('])
        || ["fn(", "unsafe fn(", "extern "]
            .iter()
            .any(|p| t.starts_with(p))
        || PRIMITIVES.contains(&t)
}

fn peel_type_modifiers(mut ty: &str) -> &str {
    loop {
        ty = ty.trim();
        if let Some(rest) = ty.strip_prefix('&') {
            let mut r = rest.trim_start();
            if r.starts_with('\'') {
                let Some(idx) = r.find(|c: char| c.is_whitespace() || c == '&') else {
                    break;
                };
                r = r[idx..].trim_start();
            }
            if let Some(after_mut) = r.strip_prefix("mut") {
                if after_mut.starts_with(char::is_whitespace) || after_mut.starts_with('&') {
                    r = after_mut.trim_start();
                }
            }
            ty = r;
            continue;
        }
        if let Some(rest) = ["*const ", "*mut ", "dyn ", "impl "]
            .iter()
            .find_map(|p| ty.strip_prefix(p))
        {
            ty = rest;
            continue;
        }
        break;
    }
    ty.split_once(" + ").map_or(ty, |(head, _)| head.trim())
}

fn strip_generics(ty: &str) -> &str {
    let s = ty.trim().strip_suffix("::").unwrap_or(ty.trim());
    let head = s.find('<').map_or(s, |idx| s[..idx].trim());
    head.strip_suffix("::").unwrap_or(head)
}

fn expand_candidates(path: &str) -> Vec<String> {
    let mut out = std_path_candidates(path);
    if let Some(rest) = path.strip_prefix("core::") {
        out.extend([format!("std::{rest}"), format!("alloc::{rest}")]);
    } else if let Some(rest) = path.strip_prefix("alloc::") {
        out.extend([format!("std::{rest}"), format!("core::{rest}")]);
    } else if let Some((_, c)) = PRELUDE_TYPES.into_iter().find(|(s, _)| *s == path) {
        out.push(c.to_string());
    }
    out
}

fn find_matching_type_defs<'a, B: AsRef<[u8]>>(
    reader: &'a IndexReader<B>,
    qp: &str,
) -> Vec<&'a DefEntry> {
    let mut results = Vec::new();
    if let Some(def) = reader.find_definition_by_path(qp) {
        if is_type_def_kind(DefKind::from(def.kind)) {
            results.push(def);
        }
    }
    if let Some((parent, _)) = qp.rsplit_once("::") {
        for d in reader.find_definitions_by_owner(parent) {
            if reader.get_str(d.path_offset()) == qp
                && is_type_def_kind(DefKind::from(d.kind))
                && !results.iter().any(|existing| std::ptr::eq(*existing, d))
            {
                results.push(d);
            }
        }
    }
    results
}

fn lookup_type_by_path(
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
        .map(|r| (r.crate_name(), r.unit_key()))
        .collect();

    let in_same_crate = |idx: usize| -> bool {
        ws.readers().get(idx).is_some_and(|r| {
            let (cr, uk) = (r.crate_name(), r.unit_key());
            crate_units
                .iter()
                .any(|(c, u)| (!cr.is_empty() && *c == cr) || (!uk.is_empty() && *u == uk))
        })
    };

    let candidates = expand_candidates(target_path);
    let mut best: Option<BestHit<'_>> = None;

    for priority_only in [true, false] {
        for cand in &candidates {
            let first_seg = cand.split("::").next().unwrap_or("");
            for (r_idx, reader) in ws.readers().iter().enumerate() {
                let cr_name = reader.crate_name();
                let same_cr = in_same_crate(r_idx);
                let cr_match = !first_seg.is_empty() && cr_name == first_seg;
                if (same_cr || cr_match) != priority_only {
                    continue;
                }
                for qp in &candidate_queries(cand, cr_name) {
                    for def in find_matching_type_defs(reader, qp) {
                        let s = def.span();
                        let raw = reader.get_file(s.file_id());
                        if raw == "external" || raw.is_empty() {
                            continue;
                        }
                        let res_path = resolve_reader_file(ws, r_idx, raw);
                        let is_dep = ws.is_reader_dependency(r_idx);
                        let score = (
                            !def.is_reexport(),
                            res_path == query_path,
                            same_cr,
                            cr_match,
                            !is_dep,
                        );
                        if best.as_ref().is_none_or(|(bs, ..)| score > *bs) {
                            let c = (s.start_line(), s.start_col(), s.end_line(), s.end_col());
                            best = Some((score, res_path, raw, c));
                        }
                    }
                }
            }
            if best.as_ref().is_some_and(|(bs, ..)| bs.0) && is_canonical_owner {
                break;
            }
        }
        if best.as_ref().is_some_and(|(bs, ..)| bs.0) {
            break;
        }
    }

    let (_, resolved_path, raw_file, coords) = best?;
    Some(format_location(&resolved_path, raw_file, coords, conv))
}

fn candidate_queries(cand: &str, cr: &str) -> Vec<String> {
    match (cand.contains("::"), cr.is_empty()) {
        (true, false) => {
            let alt = cand
                .strip_prefix(&format!("{cr}::"))
                .map_or_else(|| format!("{cr}::{cand}"), str::to_string);
            vec![cand.to_string(), alt]
        }
        (false, false) => vec![format!("{cr}::{cand}"), cand.to_string()],
        _ => vec![cand.to_string()],
    }
}

fn resolve_reader_file(ws: &WorkspaceIndex, reader_idx: usize, raw_file: &str) -> PathBuf {
    let p = Path::new(raw_file);
    if p.is_absolute() {
        return p.to_path_buf();
    }
    let primary = ws
        .source_root_at(reader_idx)
        .or_else(|| ws.workspace_root())
        .map_or_else(|| p.to_path_buf(), |r| join_resolving_overlap(r, p));
    match (!primary.exists(), ws.workspace_root()) {
        (true, Some(r)) if join_resolving_overlap(r, p).exists() => join_resolving_overlap(r, p),
        _ => primary,
    }
}

fn format_location(
    resolved_path: &Path,
    raw_file: &str,
    (sl, sc, el, ec): (u32, u32, u32, u32),
    conv: &mut PositionConverter,
) -> Value {
    let path_str = resolved_path.to_str().unwrap_or(raw_file);
    let sc_u16 = conv.char_col_to_utf16_col(path_str, sl, sc);
    let ec_u16 = conv.char_col_to_utf16_col(path_str, el, ec);
    json!({
        "uri": path_to_uri(path_str),
        "range": {
            "start": { "line": sl.saturating_sub(1), "character": sc_u16 },
            "end": { "line": el.saturating_sub(1), "character": ec_u16 }
        }
    })
}
