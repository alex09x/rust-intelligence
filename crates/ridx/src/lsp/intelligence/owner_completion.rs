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
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;

mod alias;
mod extras;
pub mod item;
mod postfix;
pub mod qualifier;
pub mod receiver;
mod shape;
pub mod smart_pointer;
pub mod trait_filter;
pub mod ufcs;
use extras::{collect_extension_and_blanket_items, collect_through_iter_items};
pub use qualifier::*;
pub use receiver::*;
pub use smart_pointer::*;
pub use trait_filter::*;
pub use ufcs::*;

const MAX_COMPLETION_ITEMS: usize = 500;

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
            if seen.insert(item::item_key(&item)) {
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
        item::number_sort_text(&mut items);
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
        let file_crate = ws.crate_name_for_file(ctx.file);
        let (mut has_iter_method, mut is_iterator) = (false, impl_traits.iter().any(|t| item::is_iterator_trait(t)));
        for o in &owners {
            let owner_is_trait = ws.find_definition_by_path(o).is_some_and(|d| d.kind == DefKind::Trait);
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

                // Not offered by rust-analyzer: `Drop::drop` (calling it is an error), methods of impls for another
                // shape of the receiver's type (`Option<&T>::copied` on `Option<String>`), hidden library items.
                let is_drop = item::trait_of_def(def.path).is_some_and(|t| trait_short_name(t) == "Drop");
                // An impl item of a hidden trait method (`assert_fields_are_eq` of a derived `Eq`) is hidden too.
                let hidden_in_trait = extract_trait_from_def_path(def.path)
                    .and_then(|t| ws.find_definition_by_path(&format!("{}::{}", base_trait_path(t), def.name)))
                    .is_some_and(|td| alias::is_hidden_library_item(&td));
                // A method or field another crate declares without `pub` is not visible here.
                let foreign = def.reader_idx.and_then(|i| ws.readers().get(i)).is_some_and(|r| Some(r.crate_name()) != file_crate);
                let invisible = foreign && matches!(category, DefScopeCategory::Field | DefScopeCategory::Inherent) && !owner_is_trait && alias::is_crate_private(&def);
                if is_drop || hidden_in_trait || invisible || (o == owner && !shape::impl_shape_matches(def.path, ty_name)) || alias::is_hidden_library_item(&def) {
                    continue;
                }
                let detail = def.signature.map(|sig| format!("{sig} (in {})", def.path)).unwrap_or_else(|| format!("{} (in {})", def.name, def.path));
                let provider = match category {
                    DefScopeCategory::PreludeTrait | DefScopeCategory::InScopeTrait => item::trait_of_def(def.path),
                    // A method of the receiver's own trait (`self` in a trait's default method, `T: Trait`).
                    _ if owner_is_trait => Some(o.as_str()),
                    _ => None,
                };
                let note = item::def_note(&def, provider);
                if category == DefScopeCategory::Inherent && def.name == "iter" && def.signature.is_some_and(item::takes_only_receiver) {
                    has_iter_method = true;
                }
                if item::trait_of_def(def.path).is_some_and(item::is_iterator_trait) {
                    is_iterator = true;
                }
                let item = item::item(def.name, note.as_deref(), lsp_completion_kind(def.kind), &detail);

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

        // Postfix snippets that depend on the receiver's type: `for` over anything iterable, the `if let` / `while
        // let` family over `Option` and `Result`.
        let mut typed_snippets: Vec<(&str, &str)> = Vec::new();
        if is_iterator || owners.iter().any(|o| o.ends_with("::IntoIterator")) {
            typed_snippets.push(("for", "for ele in expr {}"));
        }
        if matches!(base_type_path(owner), "core::option::Option" | "core::result::Result") {
            typed_snippets.extend([("ifl", "if let Some {}"), ("lete", "let Some else {}"), ("while", "while let Some {}")]);
        }
        let blanket_and_postfix = if defs_count > 0 || (ws.has_sysroot() && (owner == "tuple" || ws.find_definition_by_path(owner).is_some())) {
            collect_extension_and_blanket_items(ws, ctx, &owners, &impl_traits, &typed_snippets, &mut file_traits, &mut in_scope_traits)
        } else {
            Vec::new()
        };
        let through_iter = if has_iter_method && !is_iterator { collect_through_iter_items(ws, ctx) } else { Vec::new() };

        let t_col0 = profile_active.then(std::time::Instant::now);
        let res = collect_prioritized_items([fields, inherent, prelude_traits, in_scope_traits, blanket_and_postfix, through_iter]);
        let t_collect = t_col0.map(|t| t.elapsed()).unwrap_or_default();

        if let Some(t_all) = t_start_all {
            eprintln!(
                "[RIDX_PROFILE] owners={owners:?} deref_us={} find_defs_us={} (defs={}) cat_us={} collect_us={} total_us={}",
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
                let note =
                    item::def_note(&def, matches!(category, DefScopeCategory::PreludeTrait | DefScopeCategory::InScopeTrait).then(|| item::trait_of_def(def.path)).flatten());
                let item = item::item(def.name, note.as_deref(), lsp_completion_kind(def.kind), &detail);

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
