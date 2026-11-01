/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Receiver type resolution, multiline chain walking, and owner expansion for dot-completion.

use super::super::scope_cache::{get_or_collect_file_referenced_traits, get_or_parse_file_scopes, get_or_resolve_deref_chain};
use super::{
    base_trait_path, base_type_path, extract_nth_non_lifetime_type_arg, extract_parent_type, extract_smart_pointer_inner_type, is_array_type, is_generic_deref_wrapper,
    is_in_prelude, is_pointer_type, is_slice_type, is_smart_pointer, peel_type_modifiers, trait_short_name, OwnerCompletionContext,
};
use ridx_format::{DefKind, WorkspaceIndex};
use std::collections::HashSet;
use std::sync::Arc;

pub(crate) fn push_unique(vec: &mut Vec<String>, val: String) {
    if !val.is_empty() && !vec.iter().any(|x| x == &val) {
        vec.push(val);
    }
}

fn strip_line_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_str = false;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && in_str {
            i += 2;
            continue;
        }
        if bytes[i] == b'"' {
            in_str = !in_str;
        } else if !in_str && i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            return &line[..i];
        }
        i += 1;
    }
    line
}

/// Resolves `(ty_name, owner)` for the receiver of a dot-completion request.
pub fn resolve_dot_receiver(ws: &WorkspaceIndex, ctx: &OwnerCompletionContext<'_>) -> Option<(String, String)> {
    let before_dot = ctx.trimmed_before[..ctx.trimmed_before.len().saturating_sub(1)].trim_end();
    if !before_dot.is_empty() {
        return resolve_receiver_on_line(ws, ctx, ctx.line_1based, ctx.line_slice, before_dot);
    }

    let scopes = get_or_parse_file_scopes(ctx.file, ctx.overlay_text)?;
    let max_prev = (ctx.line_1based as usize).saturating_sub(1).min(scopes.line_starts.len());
    let min_prev = max_prev.saturating_sub(30);
    let mut tried_non_empty = 0usize;

    for idx in (min_prev..max_prev).rev() {
        let line_start = scopes.line_starts[idx];
        let next_start = scopes.line_starts.get(idx + 1).copied().unwrap_or(scopes.text.len());
        let raw_line = scopes.text[line_start..next_start].trim_end_matches(&['\r', '\n'][..]);
        let uncommented = strip_line_comment(raw_line).trim_end();
        if uncommented.trim().is_empty() {
            continue;
        }
        if uncommented.ends_with(';') || (uncommented.ends_with('{') && tried_non_empty == 0) {
            break;
        }
        let prev_line_1based = (idx + 1) as u32;
        if let Some(resolved) = resolve_receiver_on_line(ws, ctx, prev_line_1based, raw_line, uncommented) {
            return Some(resolved);
        }
        tried_non_empty += 1;
        if tried_non_empty >= 8 {
            break;
        }
    }
    None
}

fn resolve_receiver_on_line(ws: &WorkspaceIndex, ctx: &OwnerCompletionContext<'_>, line_1based: u32, line_slice: &str, before_dot: &str) -> Option<(String, String)> {
    let file = ctx.file;
    let rname: String = before_dot.chars().rev().take_while(|c| c.is_alphanumeric() || *c == '_').collect::<Vec<_>>().into_iter().rev().collect();
    let rstart = before_dot.len().saturating_sub(rname.len());
    let rcol = line_slice.get(..rstart).map(|s| s.chars().count() as u32 + 1).unwrap_or(1);
    let rend_col = line_slice.get(..before_dot.len()).map(|s| s.chars().count() as u32).unwrap_or(1);

    if before_dot.ends_with('?') && rend_col > 1 {
        let without_q = before_dot[..before_dot.len() - 1].trim_end();
        if let Some((inner_ty, _)) = ws
            .type_entry_at(file, line_1based, rend_col - 1)
            .map(|(t, o)| (t.to_string(), o.unwrap_or_default().to_string()))
            .or_else(|| resolve_receiver_on_line(ws, ctx, line_1based, line_slice, without_q))
        {
            if let Some(inner) = extract_nth_non_lifetime_type_arg(&inner_ty, 0) {
                if let Some(res) = resolve_owner_from_type_str(ws, file, inner) {
                    return Some(res);
                }
            }
        }
        if let Some((ty, Some(owner))) = ws.type_entry_at(file, line_1based, rend_col) {
            if owner.ends_with("::ControlFlow") || owner == "ControlFlow" {
                if let Some(inner) = extract_nth_non_lifetime_type_arg(ty, 1) {
                    if let Some(res) = resolve_owner_from_type_str(ws, file, inner) {
                        return Some(res);
                    }
                }
            }
        }
    }

    let type_entry = ws.type_entry_at(file, line_1based, rend_col).or_else(|| ws.type_entry_at(file, line_1based, rcol));
    if let Some((ty, Some(owner))) = type_entry {
        if owner.starts_with('<') || ty.starts_with('<') {
            if let Some(res) = resolve_owner_from_type_str(ws, file, ty) {
                return Some(res);
            }
        }
        if owner != "()" {
            return Some((ty.to_string(), owner.to_string()));
        }
    }
    if let Some((ty, None)) = type_entry {
        if ty != "()" {
            if let Some(res) = resolve_owner_from_type_str(ws, file, ty) {
                return Some(res);
            }
            let peeled = peel_type_modifiers(ty);
            if !peeled.is_empty() && peeled.chars().all(|c| c.is_alphanumeric() || c == '_') {
                if let Some(mut g_owners) =
                    super::super::generic_completion::resolve_generic_param_owners(ws, file, line_1based, line_slice, ctx.overlay_text, ctx.is_dirty, peeled)
                {
                    if let Some(first) = g_owners.drain(..).next() {
                        return Some((ty.to_string(), first));
                    }
                }
            }
        }
    }

    if before_dot.ends_with(')') {
        if let Some(res) = resolve_call_receiver(ws, file, line_1based, line_slice, before_dot) {
            return Some(res);
        }
    }

    if let Some(def) = ws.find_definition_at(file, line_1based, rend_col).or_else(|| ws.find_definition_at(file, line_1based, rcol)) {
        if matches!(def.kind, DefKind::Field | DefKind::Const | DefKind::Static | DefKind::Local) {
            if let Some(sig) = def.signature {
                let ty_part = sig.split(':').nth(1).unwrap_or(sig).split('=').next().unwrap_or(sig).trim();
                if let Some(res) = resolve_owner_from_type_str(ws, file, ty_part) {
                    return Some(res);
                }
            }
        }
    }
    None
}

fn strip_trailing_turbofish(s: &str) -> &str {
    let trimmed = s.trim_end();
    if !trimmed.ends_with('>') {
        return trimmed;
    }
    let bytes = trimmed.as_bytes();
    let mut depth = 0usize;
    for (i, &b) in bytes.iter().enumerate().rev() {
        if b == b'>' {
            depth += 1;
        } else if b == b'<' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                let before = trimmed[..i].trim_end();
                return before.strip_suffix("::").unwrap_or(before).trim_end();
            }
        }
    }
    trimmed
}

fn resolve_call_receiver(ws: &WorkspaceIndex, file: &str, line_1based: u32, line_slice: &str, before_dot: &str) -> Option<(String, String)> {
    let bytes = before_dot.as_bytes();
    let mut depth = 0usize;
    let mut open_paren = None;
    for (i, &b) in bytes.iter().enumerate().rev() {
        if b == b')' {
            depth += 1;
        } else if b == b'(' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                open_paren = Some(i);
                break;
            }
        }
    }
    let before_paren = strip_trailing_turbofish(&before_dot[..open_paren?]);
    let cname: String = before_paren.chars().rev().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == ':').collect::<Vec<_>>().into_iter().rev().collect();
    if cname.is_empty() {
        return None;
    }
    let cend_col = line_slice.get(..before_paren.len()).map(|s| s.chars().count() as u32)?;
    if let Some((ty, Some(owner))) = ws.type_entry_at(file, line_1based, cend_col) {
        if !ty.starts_with("fn(") && !ty.starts_with("fn ") && owner != "()" {
            return Some((ty.to_string(), owner.to_string()));
        }
    }
    let def = ws
        .find_definition_at(file, line_1based, cend_col)
        .or_else(|| ws.resolve_qualifier(file, &cname))
        .or_else(|| ws.find_definition_by_path(&cname))
        .or_else(|| cname.contains("::").then(|| ws.find_definition_by_path(&format!("std::{cname}"))).flatten())?;
    let sig = def.signature?;
    let arrow = sig.find("->")?;
    let ret = sig[arrow + 2..].split("where").next().unwrap_or(&sig[arrow + 2..]).trim();
    if peel_type_modifiers(ret) == "Self" {
        if let Some((parent, _)) = def.path.rsplit_once("::") {
            return resolve_owner_from_type_str(ws, file, extract_parent_type(parent));
        }
    }
    resolve_owner_from_type_str(ws, file, ret)
}

fn strip_facade_crate<'a>(ws: &WorkspaceIndex, path: &'a str) -> Option<&'a str> {
    let (_first, rest) = path.split_once("::")?;
    ws.has_crate(rest.split("::").next().unwrap_or(rest)).then_some(rest)
}

/// Resolves a raw type string into `(ty_name, owner)` using workspace index definitions and bounds.
pub fn resolve_owner_from_type_str(ws: &WorkspaceIndex, file: &str, ty_str: &str) -> Option<(String, String)> {
    let s = peel_type_modifiers(ty_str);
    if s.is_empty() || matches!(s, "()" | "_" | "!") {
        return None;
    }
    if is_slice_type(s) {
        return Some((ty_str.to_string(), "slice".to_string()));
    } else if is_array_type(s) {
        return Some((ty_str.to_string(), "array".to_string()));
    } else if is_pointer_type(s) {
        return Some((ty_str.to_string(), "pointer".to_string()));
    } else if s.starts_with('(') {
        return Some((s.to_string(), "tuple".to_string()));
    }
    if s.starts_with('<') {
        if let Some(end_angle) = s.rfind(">::") {
            let inside = &s[1..end_angle];
            let assoc_name = s[end_angle + 3..].split('<').next().unwrap_or(&s[end_angle + 3..]).trim();
            if let Some(as_idx) = inside.find(" as ") {
                let trait_base = base_type_path(inside[as_idx + 4..].trim());
                let canon_trait = ws
                    .find_definition_by_path(trait_base)
                    .or_else(|| ws.resolve_qualifier(file, trait_base))
                    .or_else(|| strip_facade_crate(ws, trait_base).and_then(|sub| ws.find_definition_by_path(sub)))
                    .map(|d| d.path.to_string());
                for q_owner in [Some(trait_base), canon_trait.as_deref()].into_iter().flatten() {
                    for def in ws.find_definitions_by_owner_for_file(file, q_owner) {
                        if def.kind == DefKind::AssocTy && def.name == assoc_name {
                            if let Some(sig) = def.signature.and_then(|sig| sig.split_once(':').map(|(_, b)| b)) {
                                let bound = sig.split('+').next().unwrap_or("").trim().trim_end_matches(';');
                                if let Some(res) = resolve_owner_from_type_str(ws, def.file, bound).or_else(|| resolve_owner_from_type_str(ws, file, bound)) {
                                    return Some(res);
                                }
                            }
                        }
                    }
                    if let Some((trait_mod, _)) = q_owner.rsplit_once("::") {
                        for cand in [format!("{trait_mod}::{assoc_name}"), format!("{trait_mod}::{assoc_name}Access")] {
                            if let Some(d) = ws.find_definition_by_path(&cand) {
                                return Some((s.to_string(), d.path.to_string()));
                            }
                        }
                    }
                }
            }
            let owner = ws.resolve_qualifier(file, assoc_name).or_else(|| ws.find_definition_by_path(assoc_name)).map(|d| d.path).unwrap_or(assoc_name);
            return Some((s.to_string(), owner.to_string()));
        }
    }
    let base = base_type_path(s.split('+').next().unwrap_or(s).trim());
    if base.is_empty() || base.starts_with('(') {
        return None;
    }
    if matches!(base, "str" | "bool" | "char" | "usize" | "u8" | "u16" | "u32" | "u64" | "u128" | "isize" | "i8" | "i16" | "i32" | "i64" | "i128" | "f32" | "f64") {
        return Some((s.to_string(), base.to_string()));
    }
    if let Some(sub) = strip_facade_crate(ws, base) {
        let owner = ws.find_definition_by_path(sub).map(|d| d.path).unwrap_or(sub);
        return Some((s.to_string(), owner.to_string()));
    }
    let owner = ws.find_definition_by_path(base).or_else(|| ws.resolve_qualifier(file, base)).map(|d| d.path).unwrap_or(base);
    Some((s.to_string(), owner.to_string()))
}

/// Expands the primary `(ty_name, owner)` into `(primary_owners, extra_impl_traits)`.
pub fn expand_dot_owners(
    ws: &WorkspaceIndex,
    ctx: &OwnerCompletionContext<'_>,
    ty_name: &str,
    owner: &str,
    file_traits: &mut Option<Arc<HashSet<String>>>,
) -> (Vec<String>, Vec<String>) {
    let mut owners = Vec::new();
    let mut impl_traits = Vec::new();
    push_unique(&mut owners, owner.to_string());
    if let Some(sub) = strip_facade_crate(ws, owner) {
        push_unique(&mut owners, sub.to_string());
        if let Some(def) = ws.find_definition_by_path(sub) {
            push_unique(&mut owners, def.path.to_string());
        }
    }
    let base_owner = base_type_path(owner);
    if base_owner != owner {
        push_unique(&mut owners, base_owner.to_string());
    }
    if owner == "array" || is_array_type(ty_name) {
        push_unique(&mut owners, "slice".to_string());
    }

    let peeled_ty = peel_type_modifiers(ty_name);
    if peeled_ty.contains('+') && !peeled_ty.contains('<') {
        for bound in peeled_ty.split('+') {
            if let Some((_, b_owner)) = resolve_owner_from_type_str(ws, ctx.file, bound) {
                push_unique(&mut owners, b_owner);
            }
        }
    }

    if is_smart_pointer(owner) {
        if let Some(inner) = extract_smart_pointer_inner_type(ty_name) {
            push_unique(&mut owners, inner.to_string());
            let inner_base = base_type_path(inner);
            push_unique(&mut owners, inner_base.to_string());
            if let Some(target) = ws.find_definition_by_path(inner_base).or_else(|| ws.resolve_qualifier(ctx.file, inner_base)) {
                push_unique(&mut owners, target.path.to_string());
            }
        }
    }

    let mut cur_ty = ty_name.to_string();
    let mut cur_owner = owner.to_string();
    for _ in 0..2 {
        let is_wrapper = is_generic_deref_wrapper(&cur_owner) || {
            cur_ty.contains('<')
                && get_or_resolve_deref_chain(ws, &cur_owner, 1).iter().all(|i| i.is_trait)
                && ws.find_definitions_by_owner_for_file(ctx.file, &cur_owner).iter().any(|d| d.name == "deref")
        };
        if !is_wrapper {
            break;
        }
        let Some(inner_arg) = extract_nth_non_lifetime_type_arg(&cur_ty, 0).map(str::to_string) else {
            break;
        };
        let Some((next_ty, next_owner)) = resolve_owner_from_type_str(ws, ctx.file, &inner_arg) else {
            break;
        };
        push_unique(&mut owners, next_owner.clone());
        let next_base = base_type_path(&next_owner);
        if next_base != next_owner {
            push_unique(&mut owners, next_base.to_string());
        }
        cur_ty = next_ty;
        cur_owner = next_owner;
    }

    let seed_owners = owners.clone();
    let mut has_display = false;
    let mut has_clone = false;
    for seed in &seed_owners {
        for item in get_or_resolve_deref_chain(ws, seed, 3).iter() {
            let target_str = item.target.as_str();
            if target_str.ends_with("::Display") || target_str == "Display" {
                has_display = true;
            }
            if target_str.ends_with("::Clone") || target_str == "Clone" {
                has_clone = true;
            }
            if !item.is_trait {
                push_unique(&mut owners, item.target.clone());
                if item.target == "array" {
                    push_unique(&mut owners, "slice".to_string());
                }
            } else {
                let f_traits = file_traits.get_or_insert_with(|| get_or_collect_file_referenced_traits(ws, ctx.file, ctx.overlay_text));
                if is_in_prelude(target_str) || f_traits.contains(target_str) || f_traits.contains(trait_short_name(target_str)) || f_traits.contains(base_trait_path(target_str)) {
                    push_unique(&mut owners, item.target.clone());
                } else if ws.has_sysroot() {
                    push_unique(&mut impl_traits, item.target.clone());
                }
            }
        }
    }
    if has_display {
        push_unique(&mut owners, "alloc::string::ToString".to_string());
    }
    if has_clone {
        push_unique(&mut owners, "alloc::borrow::ToOwned".to_string());
    }
    (owners, impl_traits)
}
