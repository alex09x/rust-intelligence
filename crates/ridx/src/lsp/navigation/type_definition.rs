/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP `textDocument/typeDefinition` handler using indexed hash and binary-search table lookups
//! (`defs_by_hash`, `defs_by_owner`, span-sorted `types`/`references`, and `UnitMatchIndex::defs_by_pos`)
//! with single-line lexical checks (`PositionConverter::get_line`), avoiding full-text and full-table scans.

use super::type_definition_lookup::{collect_type_refs_in_range, format_location, resolve_reader_file};
use super::type_definition_qual::{
    extract_qualifier_ident, is_enclosing_macro_or_raw_string, is_single_token_ref, is_use_statement_line, resolve_qualified_path_fallback, resolve_self_into_locs,
    should_prefer_def_at,
};
use super::type_definition_walk::{
    collect_following_generic_args, collect_type_components, finalize_locations, push_unique_locs, resolve_path_with_alias_expansion, resolve_walked_type_entry,
};
use crate::lsp::protocol::{uri_to_path, PositionConverter};
use crate::overlay::OverlayManager;
use memmap2::Mmap;
use ridx_format::{DefKind, IndexReader, RefEntry, TypeEntry, WorkspaceIndex};
use serde_json::Value;

const PRIMITIVES: &[&str] = &[
    "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize", "f16", "f32", "f64", "f128", "bool", "char", "str", "slice", "array", "pointer",
    "tuple", "unit", "never", "!", "()",
];

/// Per-reader pre-resolved context for the queried file and cursor position,
/// ensuring `RefEntry` and `TypeEntry` binary searches run once per reader per request.
pub(super) struct FileReaderCtx<'a> {
    pub idx: usize,
    pub reader: &'a IndexReader<Mmap>,
    pub file_id: u32,
    pub inner_ref: Option<&'a RefEntry>,
    pub inner_type: Option<&'a TypeEntry>,
}

/// Resolves `textDocument/typeDefinition` at the requested cursor position via indexed lookups.
pub fn handle_type_definition(msg: &Value, workspace: Option<&WorkspaceIndex>, overlay_mgr: &OverlayManager, conv: &mut PositionConverter) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);
    let line_1based = msg["params"]["position"]["line"].as_u64().unwrap_or(0) as u32 + 1;
    let utf16_col = msg["params"]["position"]["character"].as_u64().unwrap_or(0) as u32;

    let overlay_opt = overlay_mgr.get(&file);
    let col_1based = match overlay_opt {
        Some(overlay) => overlay.utf16_col_to_char_col(line_1based, utf16_col),
        None => conv.utf16_col_to_char_col(&file, line_1based, utf16_col),
    };
    // The inferred type answers on edited buffers too; the indexed tables below describe the saved file only.
    if let Some(loc) = workspace.and_then(|ws| super::type_definition_live::resolve_inferred_type(ws, &file, (line_1based, col_1based), overlay_mgr, conv)) {
        return loc;
    }
    let Some(ws) = workspace.filter(|_| !overlay_opt.is_some_and(|o| o.is_dirty())) else {
        return Value::Null;
    };

    let (file_readers, ctx) = build_file_reader_contexts(ws, &file, line_1based, col_1based);
    let pos = (line_1based, col_1based);
    let line_str = conv.get_line(&file, line_1based).unwrap_or("").to_string();
    if is_enclosing_macro_or_raw_string(&ctx, &line_str, col_1based) {
        return Value::Null;
    }

    let type_entry = extract_type_entry(&ctx);
    let mut tried_def_at = false;

    if let Some((ty_name, owner_opt)) = type_entry {
        if should_prefer_def_at(&ctx, pos, &line_str) {
            tried_def_at = true;
            if let Some(loc) = resolve_from_definition_at(ws, &file_readers, &ctx, (&file, pos, &line_str), type_entry, conv) {
                return loc;
            }
        }
        if let Some(loc) = resolve_walked_type_entry(ws, &file_readers, &ctx, (&file, pos), ty_name, owner_opt, conv) {
            return loc;
        }
    }

    if tried_def_at {
        return Value::Null;
    }
    resolve_from_definition_at(ws, &file_readers, &ctx, (&file, pos, &line_str), type_entry, conv).unwrap_or(Value::Null)
}

fn build_file_reader_contexts<'a>(ws: &'a WorkspaceIndex, file: &str, line: u32, col: u32) -> (Vec<usize>, Vec<FileReaderCtx<'a>>) {
    let mut raw_indices = ws.reader_indices_for_file(file);
    raw_indices.sort_by_key(|&idx| usize::from(!ws.target_kind_at(idx).is_some_and(|k| k.is_lib())));
    let mut file_readers = Vec::new();
    let mut ctx = Vec::new();

    for idx in raw_indices {
        let Some(tf) = ws.target_file_for_reader(idx, file) else {
            continue;
        };
        let reader = &ws.readers()[idx];
        let Some(file_id) = reader.find_file_id(tf.as_ref()) else {
            continue;
        };
        file_readers.push(idx);
        ctx.push(FileReaderCtx {
            idx,
            reader,
            file_id,
            inner_ref: inner_ref_at(ws, idx, file_id, line, col),
            inner_type: reader.find_innermost_type_entry(tf.as_ref(), line, col),
        });
    }

    if ctx.is_empty() {
        for idx in ws.candidate_reader_indices_for_file(file) {
            let Some(tf) = ws.target_file_for_reader(idx, file) else {
                continue;
            };
            let reader = &ws.readers()[idx];
            let Some(file_id) = reader.find_file_id(tf.as_ref()) else {
                continue;
            };
            file_readers.push(idx);
            ctx.push(FileReaderCtx {
                idx,
                reader,
                file_id,
                inner_ref: inner_ref_at(ws, idx, file_id, line, col),
                inner_type: reader.find_innermost_type_entry(tf.as_ref(), line, col),
            });
        }
    }

    (file_readers, ctx)
}

/// The innermost reference at the cursor, unless a definition at the cursor lies inside its span:
/// then the definition is the answer and the reference is not consulted.
fn inner_ref_at(ws: &WorkspaceIndex, idx: usize, file_id: u32, line: u32, col: u32) -> Option<&RefEntry> {
    ws.readers()[idx].find_innermost_ref_at_file_id(file_id, line, col).filter(|r| ws.definition_inside_reference_at(idx, file_id, line, col, r).is_none())
}

fn extract_type_entry<'a>(ctx: &[FileReaderCtx<'a>]) -> Option<(&'a str, Option<&'a str>)> {
    ctx.iter()
        .filter_map(|c| {
            let entry = c.inner_type?;
            let ty_name = c.reader.get_str(entry.type_offset());
            let owner_offset = entry.owner_path_offset();
            let owner = (owner_offset != 0).then(|| c.reader.get_str(owner_offset)).filter(|s| !s.is_empty());
            let key = (ty_name == "()", entry.span().approximate_size());
            Some((key, (ty_name, owner)))
        })
        .min_by_key(|(key, _)| *key)
        .map(|(_, val)| val)
}

fn resolve_from_definition_at(
    ws: &WorkspaceIndex,
    file_readers: &[usize],
    ctx: &[FileReaderCtx<'_>],
    (file, (line, col), line_str): (&str, (u32, u32), &str),
    type_entry: Option<(&str, Option<&str>)>,
    conv: &mut PositionConverter,
) -> Option<Value> {
    if is_use_statement_line(line_str) {
        return None;
    }
    let mut needs_cross = false;
    for c in ctx {
        let Some(resolved) = ws.find_def_at_in_reader(c.idx, c.file_id, line, col, c.inner_ref) else {
            continue;
        };
        if c.inner_ref.is_some_and(|r| !is_single_token_ref(c.reader, r, line_str)) {
            continue;
        }
        if matches!(resolved.kind, DefKind::Variant | DefKind::VariantCtor) {
            let (enum_path, var_name) = resolved.path.rsplit_once("::")?;
            if type_entry.is_some_and(|(ty, _)| (ty.contains(enum_path) || ty.contains(&format!("::{var_name}"))) && ty.contains('<')) {
                return None;
            }
            let mut locs = Vec::new();
            resolve_path_with_alias_expansion(ws, file_readers, file, enum_path, true, 0, &mut locs, conv);
            collect_following_generic_args(ws, file_readers, ctx, (file, (line, col), line_str), &mut locs, conv);
            return finalize_locations(locs);
        }
        if resolved.kind == DefKind::AssocTy {
            let mut locs = Vec::new();
            if c.inner_ref.is_none() && line_str.contains('=') {
                for ref_path in collect_type_refs_in_range(c.reader, c.file_id, (line, resolved.end_col), (resolved.end_line, 1000), true) {
                    resolve_path_with_alias_expansion(ws, file_readers, file, &ref_path, ref_path.contains("::"), 0, &mut locs, conv);
                }
                if let Some(val) = finalize_locations(locs) {
                    return Some(val);
                }
                return None;
            }
            let (trait_path, _) = resolved.path.rsplit_once("::")?;
            let mut comps = Vec::new();
            collect_type_components(trait_path, &mut comps);
            for comp in comps {
                resolve_path_with_alias_expansion(ws, file_readers, file, &comp, true, 0, &mut locs, conv);
            }
            if let Some(qual_id) = extract_qualifier_ident(line_str, col) {
                if qual_id == "Self" {
                    resolve_self_into_locs(ws, file_readers, ctx, file, line, true, &mut locs, conv);
                } else {
                    resolve_path_with_alias_expansion(ws, file_readers, file, qual_id, false, 0, &mut locs, conv);
                }
            }
            return finalize_locations(locs);
        }
        if resolved.kind == DefKind::Field && c.inner_ref.is_none() {
            let mut locs = Vec::new();
            for ref_path in collect_type_refs_in_range(c.reader, c.file_id, (line, resolved.end_col), (resolved.end_line, 1000), true) {
                resolve_path_with_alias_expansion(ws, file_readers, file, &ref_path, ref_path.contains("::"), 0, &mut locs, conv);
            }
            return finalize_locations(locs);
        }
        if resolved.kind == DefKind::AssocConst && type_entry.is_none_or(|(ty, _)| ty == "()") {
            let (owner_path, _) = resolved.path.rsplit_once("::")?;
            let mut locs = Vec::new();
            resolve_path_with_alias_expansion(ws, file_readers, file, owner_path, true, 0, &mut locs, conv);
            return finalize_locations(locs);
        }
        if !is_type_def_kind(resolved.kind) && resolved.kind != DefKind::Use {
            if resolved.kind == DefKind::Mod {
                break;
            }
            return None;
        }
        if resolved.kind == DefKind::TyParam {
            return None;
        }
        if is_type_def_kind(resolved.kind) && !is_primitive_or_builtin(resolved.name) {
            if type_entry.is_some_and(|(ty, _)| strip_generics(peel_type_modifiers(ty)) == resolved.path && ty.contains('<')) {
                return None;
            }
            let mut locs = Vec::new();
            resolve_path_with_alias_expansion(ws, file_readers, file, resolved.path, true, 0, &mut locs, conv);
            if locs.is_empty() && resolved.file != "external" && !resolved.is_reexport {
                let coords = (resolved.start_line, resolved.start_col, resolved.end_line, resolved.end_col);
                let resolved_path = resolve_reader_file(ws, c.idx, resolved.file);
                push_unique_locs(&mut locs, format_location(&resolved_path, resolved.file, coords, conv));
            }
            collect_following_generic_args(ws, file_readers, ctx, (file, (line, col), line_str), &mut locs, conv);
            if let Some(val) = finalize_locations(locs) {
                return Some(val);
            }
        }
        needs_cross = true;
        break;
    }
    if needs_cross || ctx.is_empty() {
        if let Some(def) = ws.find_definition_at(file, line, col) {
            if matches!(def.kind, DefKind::Variant | DefKind::VariantCtor) {
                let (enum_path, _) = def.path.rsplit_once("::")?;
                let mut locs = Vec::new();
                resolve_path_with_alias_expansion(ws, file_readers, file, enum_path, true, 0, &mut locs, conv);
                collect_following_generic_args(ws, file_readers, ctx, (file, (line, col), line_str), &mut locs, conv);
                return finalize_locations(locs);
            }
            if is_type_def_kind(def.kind) && !is_primitive_or_builtin(def.name) {
                let mut locs = Vec::new();
                resolve_path_with_alias_expansion(ws, file_readers, file, def.path, true, 0, &mut locs, conv);
                if locs.is_empty() && def.file != "external" && !def.is_reexport {
                    let coords = (def.start_line, def.start_col, def.end_line, def.end_col);
                    push_unique_locs(&mut locs, format_location(&def.resolved_path(), def.file, coords, conv));
                }
                collect_following_generic_args(ws, file_readers, ctx, (file, (line, col), line_str), &mut locs, conv);
                return finalize_locations(locs);
            }
        }
    }
    resolve_qualified_path_fallback(ws, file_readers, ctx, (file, (line, col), line_str), type_entry, conv)
}

pub(super) fn is_type_def_kind(kind: DefKind) -> bool {
    use DefKind::*;
    [Struct, StructCtor, Enum, Union, Trait, TyAlias, TyParam].contains(&kind)
}

pub(super) fn is_primitive_or_builtin(ty: &str) -> bool {
    let trimmed = ty.trim();
    trimmed.is_empty() || trimmed.starts_with(['[', '(', '{']) || ["fn(", "unsafe fn(", "extern ", "for<"].iter().any(|p| trimmed.starts_with(p)) || PRIMITIVES.contains(&trimmed)
}

pub(super) fn peel_type_modifiers(mut ty: &str) -> &str {
    loop {
        ty = ty.trim();
        if let Some(rest) = ty.strip_prefix('&') {
            let mut inner = rest.trim_start();
            if inner.starts_with('\'') {
                let Some(idx) = inner.find(|c: char| c.is_whitespace() || c == '&') else {
                    break;
                };
                inner = inner[idx..].trim_start();
            }
            if let Some(after_mut) = inner.strip_prefix("mut") {
                if after_mut.starts_with(char::is_whitespace) || after_mut.starts_with('&') {
                    inner = after_mut.trim_start();
                }
            }
            ty = inner;
            continue;
        }
        if let Some(rest) = ["*const ", "*mut ", "dyn ", "impl "].iter().find_map(|p| ty.strip_prefix(p)) {
            ty = rest;
            continue;
        }
        break;
    }
    ty.split_once(" + ").map_or(ty, |(head, _)| head.trim())
}

pub(super) fn strip_generics(ty: &str) -> &str {
    let trimmed = ty.trim().strip_suffix("::").unwrap_or(ty.trim());
    let head = trimmed.find('<').map_or(trimmed, |idx| trimmed[..idx].trim());
    head.strip_suffix("::").unwrap_or(head)
}
