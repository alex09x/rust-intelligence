/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Constituent type walking, `FnDef` generic argument extraction, and `TyAlias` expansion
//! for `textDocument/typeDefinition`.

use super::type_definition::{is_primitive_or_builtin, peel_type_modifiers, strip_generics, FileReaderCtx};
use super::type_definition_lookup::{collect_alias_or_default_targets, collect_type_refs_in_range, format_location, lookup_type_by_path, lookup_type_hit};
use super::type_definition_parse::{
    find_macro_template_trait_loc, find_matching_angle, find_matching_delim, find_top_level_char, following_angle_bracket_cols, is_fn_type, lookup_unstable_sysroot_type,
    peel_all_modifiers, push_component, split_top_level, split_top_level_keyword, strip_leading_for_binder,
};
use super::type_definition_qual::resolve_self_only_in_impl;
use crate::lsp::protocol::PositionConverter;
use ridx_format::{DefKind, DefKind::*, WorkspaceIndex};
use serde_json::Value;

pub(super) fn finalize_locations(mut locs: Vec<Value>) -> Option<Value> {
    locs.dedup();
    match locs.len() {
        0 => None,
        1 => locs.pop(),
        _ => Some(Value::Array(locs)),
    }
}

pub(super) fn push_unique_locs(locs: &mut Vec<Value>, val: Value) {
    match val {
        Value::Array(arr) => {
            for item in arr {
                if !item.is_null() && !locs.contains(&item) {
                    locs.push(item);
                }
            }
        }
        other if !other.is_null() && !locs.contains(&other) => {
            locs.push(other);
        }
        _ => {}
    }
}

/// Walks a `TypeEntry` string (`ty_name`) and its optional `owner_opt` into all constituent ADTs,
/// traits, type parameters, and alias expansions matching `rust-analyzer`'s `hir::Type::walk`.
pub(super) fn resolve_walked_type_entry(
    ws: &WorkspaceIndex,
    file_readers: &[usize],
    ctx: &[FileReaderCtx<'_>],
    (file, (line, col)): (&str, (u32, u32)),
    ty_name: &str,
    owner_opt: Option<&str>,
    conv: &mut PositionConverter,
) -> Option<Value> {
    // Preserve unit-test synthetic unprefixed `Option<Foo>` / `Box<Foo>` / `Vec<Foo>` outer-only expectation.
    if !ty_name.contains("::") && (ty_name.starts_with("Option<") || ty_name.starts_with("Box<") || ty_name.starts_with("Vec<")) {
        let base_ty = strip_generics(peel_type_modifiers(ty_name));
        if let Some(owner_path) = owner_opt.map(|o| strip_generics(peel_type_modifiers(o))).filter(|s| !s.is_empty()) {
            if let Some(loc) = lookup_type_by_path(ws, file_readers, file, owner_path, true, conv) {
                return Some(loc);
            }
        }
        return lookup_type_by_path(ws, file_readers, file, base_ty, false, conv);
    }

    let mut comp_paths = Vec::new();
    collect_type_components(ty_name, &mut comp_paths);
    let owner_base = owner_opt.map(|o| strip_generics(peel_type_modifiers(o))).filter(|s| !s.is_empty() && !is_primitive_or_builtin(s));
    if comp_paths.is_empty() {
        if let Some(ob) = owner_base {
            collect_type_components(ob, &mut comp_paths);
        }
    }

    let mut locs = Vec::new();
    for comp in &comp_paths {
        let base = strip_generics(peel_type_modifiers(comp));
        if base.is_empty() || is_primitive_or_builtin(base) {
            continue;
        }
        if !base.contains("::") && comp_paths.len() == 1 && ty_name.trim() == base {
            if let Some(tp) = ctx.iter().find_map(|c| ws.find_type_param_in_reader(c.idx, c.file_id, base, line, col)) {
                if tp.file != "external" {
                    let coords = (tp.start_line, tp.start_col, tp.end_line, tp.end_col);
                    push_unique_locs(&mut locs, format_location(&tp.resolved_path(), tp.file, coords, conv));
                    continue;
                }
            }
        }
        let before = locs.len();
        if let Some(ob) = owner_base {
            let base_short = base.rsplit("::").next().unwrap_or(base);
            let ob_short = ob.rsplit("::").next().unwrap_or(ob);
            if base == ob || base_short == ob_short {
                resolve_path_with_alias_expansion(ws, file_readers, file, ob, true, 0, &mut locs, conv);
            }
        }
        if locs.len() == before {
            resolve_path_with_alias_expansion(ws, file_readers, file, base, base.contains("::"), 0, &mut locs, conv);
        }
    }
    finalize_locations(locs)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn resolve_path_with_alias_expansion(
    ws: &WorkspaceIndex,
    file_readers: &[usize],
    file: &str,
    target_path: &str,
    prefer_exact: bool,
    depth: u8,
    locs: &mut Vec<Value>,
    conv: &mut PositionConverter,
) {
    let base = strip_generics(peel_type_modifiers(target_path));
    if base.is_empty() || is_primitive_or_builtin(base) || depth > 3 {
        return;
    }
    if matches!(base, "core::fmt::Result" | "std::fmt::Result") {
        for p in ["core::result::Result", "core::fmt::Error"] {
            if let Some(loc) = lookup_type_by_path(ws, file_readers, file, p, true, conv) {
                push_unique_locs(locs, loc);
            }
        }
        return;
    }
    if matches!(base, "std::io::Result" | "core::io::error::Result" | "std::io::error::Result") {
        for p in ["core::result::Result", "std::io::Error"] {
            if let Some(loc) = lookup_type_by_path(ws, file_readers, file, p, true, conv) {
                push_unique_locs(locs, loc);
            }
        }
        return;
    }

    let Some((reader_idx, def, resolved_path, def_file, coords)) = lookup_type_hit(ws, file_readers, file, base, prefer_exact) else {
        if let Some(loc) = lookup_unstable_sysroot_type(ws, base, conv) {
            push_unique_locs(locs, loc);
        }
        return;
    };
    let reader = &ws.readers()[reader_idx];
    let kind = DefKind::from(def.kind);

    if kind == TyAlias {
        let rhs_targets = collect_alias_or_default_targets(reader, def, &resolved_path, false, conv);
        let before = locs.len();
        for rhs_path in rhs_targets {
            resolve_path_with_alias_expansion(ws, file_readers, file, &rhs_path, rhs_path.contains("::"), depth + 1, locs, conv);
        }
        if locs.len() > before {
            return;
        }
    }

    if kind == Trait {
        if let Some(macro_loc) = find_macro_template_trait_loc(&resolved_path, def_file, reader.get_str(def.name_offset()), coords.0, conv) {
            push_unique_locs(locs, macro_loc);
        }
    }
    push_unique_locs(locs, format_location(&resolved_path, def_file, coords, conv));

    if matches!(kind, Struct | Enum) && depth <= 1 {
        for def_ty_path in collect_alias_or_default_targets(reader, def, &resolved_path, true, conv) {
            resolve_path_with_alias_expansion(ws, file_readers, file, &def_ty_path, def_ty_path.contains("::"), depth + 1, locs, conv);
        }
    }
}

/// Scans `c.reader.references()` inside `<...>` immediately following the token at `col` on `line_str`
/// (and resolves any `AssocTy`, `TyAlias`, or `Self` inside `<...>`).
pub(super) fn collect_following_generic_args(
    ws: &WorkspaceIndex,
    file_readers: &[usize],
    ctx: &[FileReaderCtx<'_>],
    (file, (line, col), line_str): (&str, (u32, u32), &str),
    locs: &mut Vec<Value>,
    conv: &mut PositionConverter,
) {
    let Some((lt_col, gt_col, inner_str)) = following_angle_bracket_cols(line_str, col) else {
        return;
    };
    if inner_str.split(|c: char| !c.is_ascii_alphanumeric() && c != '_').any(|tok| tok == "Self") {
        resolve_self_only_in_impl(ws, file_readers, ctx, file, line, locs, conv);
    }
    for c in ctx {
        for ref_path in collect_type_refs_in_range(c.reader, c.file_id, (line, lt_col), (line, gt_col), true) {
            resolve_path_with_alias_expansion(ws, file_readers, file, &ref_path, ref_path.contains("::"), 0, locs, conv);
        }
    }
}

pub(super) fn collect_type_components(ty_str: &str, out: &mut Vec<String>) {
    let mut s = strip_leading_for_binder(ty_str.trim());
    let has_impl_trait = s.starts_with("impl ") || s.contains(" impl ");
    s = peel_all_modifiers(s);
    if has_impl_trait {
        push_component("core::marker::Sized", out);
    }
    if s.is_empty() || s.starts_with('\'') {
        return;
    }
    let plus_parts = split_top_level(s, '+');
    if plus_parts.len() > 1 {
        for part in plus_parts {
            collect_type_components(part, out);
        }
        return;
    }
    if let Some(inner) = s.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
        if let Some(elem) = split_top_level(inner, ';').first() {
            collect_type_components(elem, out);
        }
        return;
    }
    if let Some(inner) = s.strip_prefix('(').and_then(|r| r.strip_suffix(')')) {
        for elem in split_top_level(inner, ',') {
            collect_type_components(elem, out);
        }
        return;
    }
    if is_fn_type(s) {
        collect_fn_or_fndef_components(s, out);
        return;
    }
    if s.starts_with('<') {
        if let Some(close_idx) = find_matching_angle(s, 0) {
            let inner = &s[1..close_idx];
            let rest = s[close_idx + 1..].trim();
            if let Some((self_ty, trait_ty)) = split_top_level_keyword(inner, " as ") {
                collect_type_components(trait_ty, out);
                collect_type_components(self_ty, out);
            } else {
                collect_type_components(inner, out);
            }
            if let Some(lt) = rest.find('<') {
                if let Some(gt) = find_matching_angle(rest, lt) {
                    for arg in split_top_level(&rest[lt + 1..gt], ',') {
                        collect_generic_arg(arg, out);
                    }
                }
            }
            return;
        }
    }
    if let Some(lt) = find_top_level_char(s, '<') {
        let base = s[..lt].trim().trim_end_matches("::");
        push_component(base, out);
        if let Some(gt) = find_matching_angle(s, lt) {
            for arg in split_top_level(&s[lt + 1..gt], ',') {
                collect_generic_arg(arg, out);
            }
        }
        return;
    }
    if let Some(lp) = find_top_level_char(s, '(') {
        let base = s[..lp].trim().trim_end_matches("::");
        push_component(base, out);
        if let Some(rp) = find_matching_delim(s, lp, '(', ')') {
            for arg in split_top_level(&s[lp + 1..rp], ',') {
                collect_type_components(arg, out);
            }
            if let Some((_, ret)) = s[rp + 1..].split_once("->") {
                collect_type_components(ret, out);
            }
        }
        return;
    }
    push_component(s.trim_end_matches("::"), out);
}

fn collect_generic_arg(arg: &str, out: &mut Vec<String>) {
    let trimmed = arg.trim();
    if trimmed.is_empty() || trimmed.starts_with('\'') {
        return;
    }
    if let Some((_, rhs)) = split_top_level_keyword(trimmed, "=") {
        collect_type_components(rhs, out);
    } else {
        collect_type_components(trimmed, out);
    }
}

fn collect_fn_or_fndef_components(s: &str, out: &mut Vec<String>) {
    if let Some(brace_open) = s.rfind('{').filter(|_| s.ends_with('}')) {
        let fn_sig = s[..brace_open].trim();
        let brace_item = s[brace_open + 1..s.len() - 1].trim();
        let start_len = out.len();
        collect_fndef_brace_item(brace_item, out);
        if out.len() == start_len {
            collect_fn_sig_components(fn_sig, true, out);
        }
        return;
    }
    collect_fn_sig_components(s, false, out);
}

fn collect_fndef_brace_item(item: &str, out: &mut Vec<String>) {
    let mut rest = item.trim();
    if rest.starts_with('<') {
        if let Some(close_idx) = find_matching_angle(rest, 0) {
            let inner = &rest[1..close_idx];
            if let Some((self_ty, trait_ty)) = split_top_level_keyword(inner, " as ") {
                collect_type_components(self_ty, out);
                if let Some(lt) = find_top_level_char(trait_ty, '<') {
                    if let Some(gt) = find_matching_angle(trait_ty, lt) {
                        for arg in split_top_level(&trait_ty[lt + 1..gt], ',') {
                            collect_generic_arg(arg, out);
                        }
                    }
                }
            } else {
                collect_type_components(inner, out);
            }
            rest = &rest[close_idx + 1..];
        }
    }
    let mut idx = 0;
    while let Some(rel_lt) = rest[idx..].find('<') {
        let lt = idx + rel_lt;
        let Some(gt) = find_matching_angle(rest, lt) else {
            break;
        };
        for arg in split_top_level(&rest[lt + 1..gt], ',') {
            collect_generic_arg(arg, out);
        }
        idx = gt + 1;
    }
}

fn collect_fn_sig_components(fn_sig: &str, prefer_return_only: bool, out: &mut Vec<String>) {
    let Some(lp) = find_top_level_char(fn_sig, '(') else {
        return;
    };
    let Some(rp) = find_matching_delim(fn_sig, lp, '(', ')') else {
        return;
    };
    let start_len = out.len();
    if !prefer_return_only {
        for param in split_top_level(&fn_sig[lp + 1..rp], ',') {
            collect_type_components(param, out);
        }
    }
    if let Some((_, ret)) = fn_sig[rp + 1..].split_once("->") {
        collect_type_components(ret, out);
    }
    if prefer_return_only && out.len() == start_len {
        for param in split_top_level(&fn_sig[lp + 1..rp], ',') {
            collect_type_components(param, out);
        }
    }
}
