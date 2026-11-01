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
use super::scope_cache::{get_or_collect_file_referenced_traits, is_profile_completion_enabled};
use ridx_format::{DefKind, WorkspaceIndex};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::Arc;

pub mod qualifier;
pub mod receiver;
pub mod smart_pointer;
pub mod trait_filter;
pub mod ufcs;
pub use qualifier::*;
pub use receiver::*;
pub use smart_pointer::*;
pub use trait_filter::*;
pub use ufcs::*;

const MAX_COMPLETION_ITEMS: usize = 500;

const UNIVERSAL_BLANKET_TRAITS: &[&str] = &[
    "core::convert::Into",
    "core::convert::TryInto",
    "alloc::borrow::ToOwned",
    "alloc::string::ToString",
    "core::clone::Clone",
    "core::borrow::Borrow",
    "core::borrow::BorrowMut",
];

const POSTFIX_SNIPPETS: &[(&str, &str)] = &[
    ("arc", "Arc::new(expr)"),
    ("box", "Box::new(expr)"),
    ("call", "function(expr)"),
    ("const", "const { expr }"),
    ("dbg", "dbg!(expr)"),
    ("dbgr", "dbg!(&expr)"),
    ("deref", "*expr"),
    ("err", "Err(expr)"),
    ("let", "let"),
    ("letm", "let mut"),
    ("match", "match expr {}"),
    ("ok", "Ok(expr)"),
    ("pinbox", "Box::pin(expr)"),
    ("rc", "Rc::new(expr)"),
    ("ref", "&expr"),
    ("refm", "&mut expr"),
    ("return", "return expr"),
    ("some", "Some(expr)"),
    ("unsafe", "unsafe { expr }"),
];

/// Returns true if the path belongs to standard/sysroot crates not yet indexed in `.ridx`.
pub(crate) fn is_sysroot_path(path: &str) -> bool {
    path.starts_with("core::")
        || path.starts_with("alloc::")
        || path.starts_with("std::")
        || path == "str"
        || path == "slice"
        || path == "array"
        || path == "pointer"
        || matches!(path, "bool" | "char" | "i8" | "i16" | "i32" | "i64" | "i128" | "isize" | "u8" | "u16" | "u32" | "u64" | "u128" | "usize" | "f32" | "f64")
}

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

fn collect_prioritized_items<const N: usize>(buckets: [Vec<Value>; N]) -> Option<Vec<Value>> {
    let mut items = Vec::with_capacity(128);
    let mut seen = HashSet::new();
    for bucket in buckets {
        for item in bucket {
            let name = item["label"].as_str().unwrap_or_default();
            if seen.insert(name.to_string()) {
                items.push(item);
                if items.len() >= MAX_COMPLETION_ITEMS {
                    break;
                }
            }
        }
        if items.len() >= MAX_COMPLETION_ITEMS {
            break;
        }
    }
    if items.is_empty() {
        None
    } else {
        Some(items)
    }
}

/// Attempts high-speed owner-based completion for dot- (`receiver.`) or colon- (`path::`) trigger.
///
/// Returns `Some(items)` if the owner index resolved candidate items directly (sub-100 µs),
/// or `None` if fallback to workspace symbol scanning is required (e.g. sysroot types, unknown types).
pub fn try_owner_completion(ws: &WorkspaceIndex, ctx: &OwnerCompletionContext<'_>) -> Option<Vec<Value>> {
    if ctx.is_dot {
        let (ty_name, owner) = resolve_dot_receiver(ws, ctx)?;
        let owner = owner.as_str();
        let ty_name = ty_name.as_str();

        let mut fields = Vec::new();
        let mut inherent = Vec::new();
        let mut prelude_traits = Vec::new();
        let mut in_scope_traits = Vec::new();
        let mut file_traits: Option<Arc<HashSet<String>>> = None;

        let profile_active = is_profile_completion_enabled();
        let t_start_all = profile_active.then(std::time::Instant::now);
        let mut t_find = std::time::Duration::ZERO;
        let mut t_cat = std::time::Duration::ZERO;
        let mut defs_count = 0usize;

        let t_d0 = profile_active.then(std::time::Instant::now);
        let (owners, impl_traits) = expand_dot_owners(ws, ctx, ty_name, owner, &mut file_traits);
        let t_deref = t_d0.map(|t| t.elapsed()).unwrap_or_default();

        let mut seen_labels = HashSet::new();
        for o in &owners {
            let t_f0 = profile_active.then(std::time::Instant::now);
            let defs = ws.find_definitions_by_owner_for_file(ctx.file, o);
            if let Some(t) = t_f0 {
                t_find += t.elapsed();
            }
            defs_count += defs.len();

            for def in defs {
                let t_c0 = profile_active.then(std::time::Instant::now);
                if def.kind == DefKind::Field {
                    // Struct fields are always accessible via dot
                } else if matches!(def.kind, DefKind::AssocFn | DefKind::Fn) {
                    let has_receiver = def.signature.as_ref().is_some_and(|sig| signature_takes_receiver(sig));
                    if !has_receiver {
                        if let Some(t) = t_c0 {
                            t_cat += t.elapsed();
                        }
                        continue;
                    }
                } else {
                    if let Some(t) = t_c0 {
                        t_cat += t.elapsed();
                    }
                    continue;
                }
                if !ctx.prefix.is_empty() && !def.name.starts_with(ctx.prefix) {
                    if let Some(t) = t_c0 {
                        t_cat += t.elapsed();
                    }
                    continue;
                }

                let f_traits = file_traits.get_or_insert_with(|| get_or_collect_file_referenced_traits(ws, ctx.file, ctx.overlay_text));
                let category = categorize_definition(def.kind, def.path, o, owner, f_traits.as_ref());

                if category == DefScopeCategory::OutOfScopeTrait {
                    if let Some(t) = t_c0 {
                        t_cat += t.elapsed();
                    }
                    continue;
                }

                let detail = def.signature.map(|sig| format!("{sig} (in {})", def.path)).unwrap_or_else(|| format!("{} (in {})", def.name, def.path));
                let item = json!({
                    "label": def.name,
                    "kind": lsp_completion_kind(def.kind),
                    "detail": detail
                });

                match category {
                    DefScopeCategory::Field => {
                        seen_labels.insert(def.name);
                        fields.push(item);
                    }
                    DefScopeCategory::Inherent => {
                        if seen_labels.insert(def.name) {
                            inherent.push(item);
                        }
                    }
                    DefScopeCategory::PreludeTrait => prelude_traits.push(item),
                    DefScopeCategory::InScopeTrait => in_scope_traits.push(item),
                    DefScopeCategory::OutOfScopeTrait => {}
                }
                if let Some(t) = t_c0 {
                    t_cat += t.elapsed();
                }
                if fields.len() + inherent.len() >= MAX_COMPLETION_ITEMS {
                    break;
                }
            }
            if fields.len() + inherent.len() >= MAX_COMPLETION_ITEMS {
                break;
            }
        }

        let blanket_and_postfix = if defs_count > 0 || (ws.has_sysroot() && (owner == "tuple" || ws.find_definition_by_path(owner).is_some())) {
            collect_extension_and_blanket_items(ws, ctx, &owners, &impl_traits, &mut file_traits, &mut in_scope_traits)
        } else {
            Vec::new()
        };

        let t_col0 = profile_active.then(std::time::Instant::now);
        let res = collect_prioritized_items([fields, inherent, prelude_traits, in_scope_traits, blanket_and_postfix]);
        let t_collect = t_col0.map(|t| t.elapsed()).unwrap_or_default();

        if let Some(t_all) = t_start_all {
            eprintln!(
                "[RIDX_PROFILE] deref_us={} find_defs_us={} (defs={}) cat_us={} collect_us={} total_us={}",
                t_deref.as_micros(),
                t_find.as_micros(),
                defs_count,
                t_cat.as_micros(),
                t_collect.as_micros(),
                t_all.elapsed().as_micros()
            );
        }

        res
    } else if ctx.is_colon {
        let qual = extract_colon_qualifier(ctx.trimmed_before);

        if qual.is_empty() {
            return None;
        }

        let target_owners: Vec<String> = if qual == "Self" {
            super::self_completion::resolve_self_owners(ws, ctx.file, ctx.line_1based, ctx.line_slice, ctx.overlay_text)?
        } else if let Some(owners) = super::generic_completion::resolve_generic_param_owners(ws, ctx.file, ctx.line_1based, ctx.line_slice, ctx.overlay_text, ctx.is_dirty, &qual) {
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
                        ws.crate_name_for_file(ctx.file).map(|c| c.to_string()).unwrap_or_else(|| qual.clone())
                    } else if let Some(rest) = qual.strip_prefix("crate::") {
                        ws.crate_name_for_file(ctx.file).map(|c| format!("{c}::{rest}")).unwrap_or_else(|| qual.clone())
                    } else {
                        qual.clone()
                    }
                });
            vec![target_owner]
        };

        let has_sysroot = ws.has_sysroot();
        let syntax_pos = super::syntax_pos::classify_colon_position(ctx.trimmed_before);
        let mut fields = Vec::new();
        let mut inherent = Vec::new();
        let mut prelude_traits = Vec::new();
        let mut in_scope_traits = Vec::new();
        let mut file_traits: Option<Arc<HashSet<String>>> = None;

        for target_owner in &target_owners {
            if !has_sysroot && is_sysroot_path(target_owner) {
                continue;
            }
            for def in ws.find_definitions_by_owner_for_file(ctx.file, target_owner) {
                if def.kind == DefKind::TyParam {
                    continue;
                }
                if !super::syntax_pos::is_def_allowed_in_position(def.kind, syntax_pos) {
                    continue;
                }
                if !ctx.prefix.is_empty() && !def.name.starts_with(ctx.prefix) {
                    continue;
                }

                let f_traits = file_traits.get_or_insert_with(|| get_or_collect_file_referenced_traits(ws, ctx.file, ctx.overlay_text));
                let category = categorize_definition(def.kind, def.path, target_owner, target_owner, f_traits.as_ref());

                if category == DefScopeCategory::OutOfScopeTrait {
                    continue;
                }

                let detail = def.signature.map(|sig| format!("{sig} (in {})", def.path)).unwrap_or_else(|| format!("{} (in {})", def.name, def.path));
                let item = json!({
                    "label": def.name,
                    "kind": lsp_completion_kind(def.kind),
                    "detail": detail
                });

                match category {
                    DefScopeCategory::Field => fields.push(item),
                    DefScopeCategory::Inherent => inherent.push(item),
                    DefScopeCategory::PreludeTrait => prelude_traits.push(item),
                    DefScopeCategory::InScopeTrait => in_scope_traits.push(item),
                    DefScopeCategory::OutOfScopeTrait => {}
                }
            }
        }

        collect_prioritized_items([fields, inherent, prelude_traits, in_scope_traits])
    } else {
        None
    }
}

fn collect_extension_and_blanket_items(
    ws: &WorkspaceIndex,
    ctx: &OwnerCompletionContext<'_>,
    visited_owners: &[String],
    impl_traits: &[String],
    file_traits: &mut Option<Arc<HashSet<String>>>,
    in_scope_traits: &mut Vec<Value>,
) -> Vec<Value> {
    let mut blanket_and_postfix = Vec::new();
    if !ws.has_sysroot() {
        return blanket_and_postfix;
    }

    for (tr, is_universal) in UNIVERSAL_BLANKET_TRAITS.iter().copied().map(|t| (t, true)).chain(impl_traits.iter().map(|t| (t.as_str(), false))) {
        if is_universal && visited_owners.iter().any(|o| o == tr) {
            continue;
        }
        for def in ws.find_definitions_by_owner_for_file(ctx.file, tr) {
            if !matches!(def.kind, DefKind::AssocFn | DefKind::Fn) || !def.signature.is_some_and(signature_takes_receiver) {
                continue;
            }
            if !ctx.prefix.is_empty() && !def.name.starts_with(ctx.prefix) {
                continue;
            }
            let detail = def.signature.map(|sig| format!("{sig} (in {})", def.path)).unwrap_or_else(|| format!("{} (in {})", def.name, def.path));
            in_scope_traits.push(json!({
                "label": def.name,
                "kind": lsp_completion_kind(def.kind),
                "detail": detail
            }));
        }
    }

    for &(label, detail) in POSTFIX_SNIPPETS {
        if !ctx.prefix.is_empty() && !label.starts_with(ctx.prefix) {
            continue;
        }
        blanket_and_postfix.push(json!({
            "label": label,
            "kind": 15,
            "detail": detail
        }));
    }

    let f_traits = file_traits.get_or_insert_with(|| get_or_collect_file_referenced_traits(ws, ctx.file, ctx.overlay_text));
    let mut sorted_traits: Vec<&String> = f_traits.iter().filter(|tr| tr.contains("::") && !visited_owners.iter().any(|o| o == *tr)).collect();
    sorted_traits.sort();
    for tr in sorted_traits {
        for def in ws.find_definitions_by_owner_for_file(ctx.file, tr) {
            if !matches!(def.kind, DefKind::AssocFn | DefKind::Fn) || !def.signature.is_some_and(signature_takes_receiver) {
                continue;
            }
            if !ctx.prefix.is_empty() && !def.name.starts_with(ctx.prefix) {
                continue;
            }
            let detail = def.signature.map(|sig| format!("{sig} (in {})", def.path)).unwrap_or_else(|| format!("{} (in {})", def.name, def.path));
            blanket_and_postfix.push(json!({
                "label": def.name,
                "kind": lsp_completion_kind(def.kind),
                "detail": detail
            }));
        }
    }

    blanket_and_postfix
}
