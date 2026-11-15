/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Dot-completion items beyond the receiver's own methods: blanket-trait methods, postfix snippets, and the
//! methods reachable through `iter()`.

use super::{item, signature_takes_receiver, trait_short_name, OwnerCompletionContext};
use crate::lsp::intelligence::scope_cache::{get_or_collect_file_referenced_traits, get_or_parse_file_scopes, get_or_resolve_deref_chain};
use crate::lsp::protocol::lsp_completion_kind;
use ridx_format::{DefKind, WorkspaceIndex};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;

/// Traits every type implements through blanket impls, with whether the prelude brings them into scope. `ToOwned`
/// and `ToString` hold only for `Clone` and `Display` types; the receiver expansion adds them for those.
const UNIVERSAL_BLANKET_TRAITS: &[(&str, bool)] =
    &[("core::convert::Into", true), ("core::convert::TryInto", true), ("core::borrow::Borrow", false), ("core::borrow::BorrowMut", false)];

pub(super) const ITERATOR_TRAIT: &str = "core::iter::traits::iterator::Iterator";

/// The traits of the iterator `iter()` returns on a collection, option or path, whose methods complete as
/// `iter().name`.
const THROUGH_ITER_TRAITS: &[&str] = &[
    ITERATOR_TRAIT,
    "core::iter::traits::double_ended::DoubleEndedIterator",
    "core::iter::traits::exact_size::ExactSizeIterator",
    "core::iter::traits::collect::IntoIterator",
    "core::clone::Clone",
    "alloc::borrow::ToOwned",
    "core::convert::Into",
    "core::convert::TryInto",
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

/// Postfix snippets that name a type, with the path rust-analyzer always notes for them (also when imported).
const POSTFIX_IMPORTS: &[(&str, &str)] = &[("arc", "use std::sync::Arc"), ("rc", "use std::rc::Rc")];

pub(super) fn collect_extension_and_blanket_items(
    ws: &WorkspaceIndex,
    ctx: &OwnerCompletionContext<'_>,
    visited_owners: &[String],
    impl_traits: &[String],
    typed_snippets: &[(&str, &str)],
    file_traits: &mut Option<Arc<HashSet<String>>>,
    in_scope_traits: &mut Vec<Value>,
) -> Vec<Value> {
    let mut blanket_and_postfix = Vec::new();
    if !ws.has_sysroot() {
        return blanket_and_postfix;
    }

    let f_traits = file_traits.get_or_insert_with(|| get_or_collect_file_referenced_traits(ws, ctx.file, ctx.overlay_text)).clone();
    let in_scope = |tr: &str| f_traits.contains(tr) || f_traits.contains(trait_short_name(tr));

    // Blanket traits every type implements (those the prelude or the file brings into scope), then the traits the
    // receiver's bounds name. Methods of other traits the file merely mentions are not offered: the receiver need
    // not implement them, and rust-analyzer does not offer them.
    let universal = UNIVERSAL_BLANKET_TRAITS.iter().filter(|(tr, prelude)| *prelude || in_scope(tr)).map(|(tr, _)| (*tr, true));
    for (tr, is_universal) in universal.chain(impl_traits.iter().map(|t| (t.as_str(), false))) {
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
            in_scope_traits.push(item::with_decl(item::item(def.name, item::def_note(&def, Some(tr)).as_deref(), lsp_completion_kind(def.kind), &detail), &def));
        }
    }

    // Imported extension traits whose supertrait the receiver implements (`Itertools: Iterator`,
    // `AstNodeEdit: AstNode`): they come with a blanket impl for every such type.
    for tr in extension_traits(ws, &f_traits, visited_owners) {
        for def in ws.find_definitions_by_owner_for_file(ctx.file, &tr) {
            if !matches!(def.kind, DefKind::AssocFn | DefKind::Fn) || !def.signature.is_some_and(signature_takes_receiver) || super::alias::is_hidden_library_item(&def) {
                continue;
            }
            if !ctx.prefix.is_empty() && !def.name.starts_with(ctx.prefix) {
                continue;
            }
            let detail = def.signature.map(|sig| format!("{sig} (in {})", def.path)).unwrap_or_else(|| format!("{} (in {})", def.name, def.path));
            in_scope_traits.push(item::with_decl(item::item(def.name, item::def_note(&def, Some(&tr)).as_deref(), lsp_completion_kind(def.kind), &detail), &def));
        }
    }

    let file_text = ctx.overlay_text.map(str::to_string).or_else(|| get_or_parse_file_scopes(ctx.file, None).map(|s| s.text.to_string()));
    let is_statement = file_text.as_deref().is_none_or(|t| {
        let lines: Vec<&str> = t.lines().collect();
        let at = ctx.line_1based.saturating_sub(1) as usize;
        let before = &ctx.line_slice[..ctx.line_slice.len() - ctx.prefix.len()];
        let prev = lines[..at.min(lines.len())].iter().rev().find(|l| !l.trim().is_empty()).copied();
        lines.get(at).is_none_or(|line| super::postfix::is_statement_expression(line, before, prev))
    });
    for &(label, detail) in POSTFIX_SNIPPETS.iter().chain(typed_snippets) {
        if !ctx.prefix.is_empty() && !label.starts_with(ctx.prefix) {
            continue;
        }
        if matches!(label, "let" | "letm") && !is_statement {
            continue;
        }
        let import = POSTFIX_IMPORTS.iter().find(|(l, _)| *l == label).map(|(_, import)| format!("({import})"));
        blanket_and_postfix.push(item::item(label, import.as_deref(), 15, detail));
    }

    blanket_and_postfix
}

/// The postfix snippets whose presence depends on neither the receiver's type nor its position (all of
/// [`POSTFIX_SNIPPETS`] except the labels in `positional`), filtered by `prefix`.
pub(crate) fn fixed_postfix_items(prefix: &str, positional: &[&str]) -> Vec<Value> {
    POSTFIX_SNIPPETS
        .iter()
        .filter(|(label, _)| !positional.contains(label) && (prefix.is_empty() || label.starts_with(prefix)))
        .map(|&(label, detail)| {
            let import = POSTFIX_IMPORTS.iter().find(|(l, _)| *l == label).map(|(_, import)| format!("({import})"));
            item::item(label, import.as_deref(), 15, detail)
        })
        .collect()
}

/// The imported traits (full paths) that are not among the receiver's `owners` but have a supertrait that is.
fn extension_traits(ws: &WorkspaceIndex, file_traits: &HashSet<String>, owners: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tr in file_traits.iter().filter(|t| t.contains("::")) {
        let Some(def) = ws.find_definition_by_path(tr).filter(|d| d.kind == DefKind::Trait) else { continue };
        let path = def.path.to_string();
        if owners.contains(&path) || out.contains(&path) {
            continue;
        }
        // A supertrait every other trait builds on (`Vec4Swizzles: Sized + Copy + Clone`) says nothing about a
        // blanket impl; an extension trait extends a trait with behavior (`Iterator`, `AstNode`).
        let supers = get_or_resolve_deref_chain(ws, &path, 1);
        let is_extension = |s: &str| {
            !matches!(
                trait_short_name(s),
                "Sized" | "Copy" | "Clone" | "Send" | "Sync" | "Unpin" | "Debug" | "Display" | "Default" | "Eq" | "PartialEq" | "Ord" | "PartialOrd" | "Hash" | "Any"
            )
        };
        if supers.iter().any(|s| is_extension(&s.target) && owners.iter().any(|o| o == &s.target)) {
            out.push(path);
        }
    }
    out.sort();
    out
}

/// The methods of the iterator a receiver's `iter()` returns, offered as `iter().name` the way rust-analyzer offers
/// them on collections, options and paths that are not iterators themselves.
pub(crate) fn collect_through_iter_items(ws: &WorkspaceIndex, ctx: &OwnerCompletionContext<'_>) -> Vec<Value> {
    let mut out = Vec::new();
    let f_traits = get_or_collect_file_referenced_traits(ws, ctx.file, ctx.overlay_text);
    let iter_extensions = extension_traits(ws, &f_traits, &[ITERATOR_TRAIT.to_string()]);
    for tr in THROUGH_ITER_TRAITS.iter().copied().chain(iter_extensions.iter().map(String::as_str)) {
        for def in ws.find_definitions_by_owner_for_file(ctx.file, tr) {
            if !matches!(def.kind, DefKind::AssocFn | DefKind::Fn) || !def.signature.is_some_and(signature_takes_receiver) {
                continue;
            }
            if !ctx.prefix.is_empty() && !def.name.starts_with(ctx.prefix) {
                continue;
            }
            let note = item::def_note(&def, Some(tr)).unwrap_or_default();
            out.push(item::through_iter_item(def.name, &note, lsp_completion_kind(def.kind), def.signature.unwrap_or_default()));
        }
    }
    out
}
