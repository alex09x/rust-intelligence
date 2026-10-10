/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Fast owner-index and autoderef-based completion handler.

use super::super::protocol::lsp_completion_kind;
use ridx_format::{DefKind, WorkspaceIndex};
use serde_json::{json, Value};
use std::collections::HashSet;

/// Returns true if the path belongs to standard/sysroot crates not yet indexed in `.ridx`.
pub(crate) fn is_sysroot_path(path: &str) -> bool {
    path.starts_with("core::")
        || path.starts_with("alloc::")
        || path.starts_with("std::")
        || path == "str"
        || path == "slice"
        || path == "array"
        || path == "pointer"
        || matches!(
            path,
            "bool"
                | "char"
                | "i8"
                | "i16"
                | "i32"
                | "i64"
                | "i128"
                | "isize"
                | "u8"
                | "u16"
                | "u32"
                | "u64"
                | "u128"
                | "usize"
                | "f32"
                | "f64"
        )
}

/// Checks if a function signature takes a receiver argument (`self`, `&self`, `&'a self`, `&'a mut self`, `&mut self`, `mut self`).
pub(crate) fn signature_takes_receiver(sig: &str) -> bool {
    let Some(i) = sig.find('(') else {
        return false;
    };
    let mut p = sig[i + 1..].trim_start();
    if let Some(rest) = p.strip_prefix('&') {
        p = rest.trim_start();
        // Skip optional lifetime like 'a, '_, 'static
        if p.starts_with('\'') {
            if let Some(sp) = p.find(|c: char| c.is_whitespace() || c == ',' || c == ')') {
                p = p[sp..].trim_start();
            }
        }
    }
    if let Some(rest) = p.strip_prefix("mut") {
        if rest.starts_with(' ') || rest.starts_with('\t') {
            p = rest.trim_start();
        }
    }
    p.starts_with("self")
        && (p.len() == 4
            || p[4..].starts_with(',')
            || p[4..].starts_with(')')
            || p[4..].starts_with(':')
            || p[4..].starts_with(' '))
}

pub(crate) use ridx_format::workspace::is_prelude_trait;

/// Collects all trait names/paths referenced in the given file.
fn collect_file_referenced_traits(ws: &WorkspaceIndex, file_path: &str) -> HashSet<String> {
    let mut set = HashSet::new();
    for idx in ws.candidate_reader_indices_for_file(file_path) {
        let reader = &ws.readers()[idx];
        let target_file = ws.target_file_for_reader(idx, file_path);
        let target_str = target_file.as_deref().unwrap_or(file_path);
        if let Some(fid) = reader.find_file_id(target_str) {
            for r in reader.references() {
                if r.span().file_id() == fid {
                    let path = reader.get_str(r.target_path_offset());
                    if !path.is_empty() {
                        if let Some(short) = path.rsplit("::").next() {
                            set.insert(short.to_string());
                        }
                        set.insert(path.to_string());
                    }
                }
            }
            break;
        }
    }
    set
}
pub mod qualifier;
pub mod ufcs;
pub use qualifier::*;
pub use ufcs::*;

/// Context parameters for owner-based scoped completion.
pub struct OwnerCompletionContext<'a> {
    pub file: &'a str,
    pub line_1based: u32,
    pub line_slice: &'a str,
    pub prefix: &'a str,
    pub is_dot: bool,
    pub is_colon: bool,
    pub trimmed_before: &'a str,
    pub overlay_text: Option<&'a str>,
    pub is_dirty: bool,
}

/// Attempts high-speed owner-based completion for dot- (`receiver.`) or colon- (`path::`) trigger.
///
/// Returns `Some(items)` if the owner index resolved candidate items directly (sub-100 µs),
/// or `None` if fallback to workspace symbol scanning is required (e.g. sysroot types, unknown types).
pub fn try_owner_completion(
    ws: &WorkspaceIndex,
    ctx: &OwnerCompletionContext<'_>,
) -> Option<Vec<Value>> {
    if ctx.is_dot {
        let before_dot = ctx.trimmed_before[..ctx.trimmed_before.len() - 1].trim_end();
        let rname: String = before_dot
            .chars()
            .rev()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let rstart = before_dot.len().saturating_sub(rname.len());
        let rcol = ctx.line_slice[..rstart].chars().count() as u32 + 1;
        let rend_col = ctx.line_slice[..before_dot.len()].chars().count() as u32;

        let type_entry = ws
            .type_entry_at(ctx.file, ctx.line_1based, rend_col)
            .or_else(|| ws.type_entry_at(ctx.file, ctx.line_1based, rcol));

        let Some((_ty_name, Some(owner))) = type_entry else {
            return None;
        };

        let has_sysroot = ws.has_sysroot();
        if !has_sysroot && is_sysroot_path(owner) {
            return None;
        }

        let mut owners = vec![owner.to_string()];
        for deref_target in ws.resolve_deref_chain(owner, 8) {
            owners.push(deref_target.to_string());
        }

        let mut items = Vec::new();
        let mut seen = HashSet::new();
        let mut file_traits: Option<HashSet<String>> = None;

        for o in &owners {
            if o != owner && !is_prelude_trait(o) {
                let is_trait = ws
                    .find_definition_by_path(o)
                    .map(|d| d.kind == DefKind::Trait)
                    .unwrap_or(false);
                if is_trait {
                    let file_set = file_traits
                        .get_or_insert_with(|| collect_file_referenced_traits(ws, ctx.file));
                    let short_name = o.rsplit("::").next().unwrap_or(o);
                    if !file_set.contains(short_name) && !file_set.contains(o) {
                        continue;
                    }
                }
            }

            for def in ws.find_definitions_by_owner(o) {
                if def.kind == DefKind::Field {
                    // Struct fields are always accessible via dot
                } else if matches!(def.kind, DefKind::AssocFn | DefKind::Fn) {
                    let has_receiver = def
                        .signature
                        .as_ref()
                        .is_some_and(|sig| signature_takes_receiver(sig));
                    if !has_receiver {
                        continue;
                    }
                } else {
                    continue;
                }
                if !ctx.prefix.is_empty() && !def.name.starts_with(ctx.prefix) {
                    continue;
                }
                if seen.insert(def.name.to_string()) {
                    let detail = def
                        .signature
                        .map(|sig| format!("{sig} (in {})", def.path))
                        .unwrap_or_else(|| format!("{} (in {})", def.name, def.path));
                    items.push(json!({
                        "label": def.name,
                        "kind": lsp_completion_kind(def.kind),
                        "detail": detail
                    }));
                    if items.len() >= 100 {
                        break;
                    }
                }
            }
            if items.len() >= 100 {
                break;
            }
        }

        if items.is_empty() {
            None
        } else {
            Some(items)
        }
    } else if ctx.is_colon {
        let qual = extract_colon_qualifier(ctx.trimmed_before);

        if qual.is_empty() {
            return None;
        }

        let target_owners: Vec<String> = if qual == "Self" {
            super::self_completion::resolve_self_owners(
                ws,
                ctx.file,
                ctx.line_1based,
                ctx.line_slice,
                ctx.overlay_text,
            )?
        } else if let Some(owners) = super::generic_completion::resolve_generic_param_owners(
            ws,
            ctx.file,
            ctx.line_1based,
            ctx.line_slice,
            ctx.overlay_text,
            ctx.is_dirty,
            &qual,
        ) {
            owners
        } else {
            let def_opt = ws.resolve_qualifier(ctx.file, &qual);
            let target_owner = def_opt
                .as_ref()
                .and_then(|d| {
                    if d.is_reexport || d.kind == DefKind::Use {
                        if let Some(sig) = d.signature {
                            return Some(sig.to_string());
                        }
                        return None;
                    }
                    Some(d.path.to_string())
                })
                .unwrap_or_else(|| {
                    if qual == "crate" {
                        ws.crate_name_for_file(ctx.file)
                            .map(|c| c.to_string())
                            .unwrap_or_else(|| qual.clone())
                    } else if let Some(rest) = qual.strip_prefix("crate::") {
                        ws.crate_name_for_file(ctx.file)
                            .map(|c| format!("{c}::{rest}"))
                            .unwrap_or_else(|| qual.clone())
                    } else {
                        qual.clone()
                    }
                });
            vec![target_owner]
        };

        let has_sysroot = ws.has_sysroot();
        let syntax_pos = super::syntax_pos::classify_colon_position(ctx.trimmed_before);
        let mut items = Vec::new();
        let mut seen = HashSet::new();

        for target_owner in &target_owners {
            if !has_sysroot && is_sysroot_path(target_owner) {
                continue;
            }
            for def in ws.find_definitions_by_owner(target_owner) {
                if def.kind == DefKind::TyParam {
                    continue;
                }
                if !super::syntax_pos::is_def_allowed_in_position(def.kind, syntax_pos) {
                    continue;
                }
                if !ctx.prefix.is_empty() && !def.name.starts_with(ctx.prefix) {
                    continue;
                }
                if seen.insert(def.name.to_string()) {
                    let detail = def
                        .signature
                        .map(|sig| format!("{sig} (in {})", def.path))
                        .unwrap_or_else(|| format!("{} (in {})", def.name, def.path));
                    items.push(json!({
                        "label": def.name,
                        "kind": lsp_completion_kind(def.kind),
                        "detail": detail
                    }));
                    if items.len() >= 100 {
                        break;
                    }
                }
            }
            if items.len() >= 100 {
                break;
            }
        }

        if items.is_empty() {
            None
        } else {
            Some(items)
        }
    } else {
        None
    }
}
