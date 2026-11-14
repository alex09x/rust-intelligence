/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The completion list after `receiver.` from the receiver's inferred type.
//!
//! The owner index names a receiver by its head type, so it offers every method of every impl of that head. The
//! live inference knows the receiver's full type, the traits in scope and the expression's position, so it decides
//! the fields, methods and postfix snippets; the owner index answers only when the receiver's type is unknown.

use std::collections::HashSet;

use serde_json::Value;

use super::owner_completion::{collect_through_iter_items, fixed_postfix_items, item, OwnerCompletionContext};
use crate::live::{Cand, DotCompletion, PostfixFacts, TryEnum};
use crate::lsp::navigation as live_nav;
use crate::lsp::protocol::lsp_completion_kind;
use crate::overlay::OverlayManager;
use ridx_format::WorkspaceIndex;

/// LSP completion kinds Snippet and Keyword.
const SNIPPET_KIND: u64 = 15;
const KEYWORD_KIND: u32 = 14;

/// Postfix snippets whose presence depends on the receiver's type or position.
const POSITIONAL_POSTFIX: [&str; 11] = ["let", "letm", "for", "match", "ifl", "lete", "while", "if", "not", "break", "drop"];

/// The positional postfix snippets rust-analyzer offers for `facts`, with their details.
fn positional_postfix(facts: &PostfixFacts) -> Vec<(&'static str, &'static str)> {
    let (some, ifl, lete, while_let) = match facts.try_enum {
        Some(TryEnum::Result) => ("let Ok", "if let Ok {}", "let Ok else {}", "while let Ok {}"),
        _ => ("let Some", "if let Some {}", "let Some else {}", "while let Some {}"),
    };
    let mut out = Vec::with_capacity(8);
    if facts.in_condition {
        out.push(("let", if facts.try_enum.is_some() { some } else { "let" }));
        if facts.try_enum.is_some() {
            out.push(("letm", "let mut"));
        }
    } else {
        if facts.in_block || facts.in_arm_or_closure {
            out.extend([("let", "let"), ("letm", "let mut")]);
        }
        out.push(("match", "match expr {}"));
        if facts.try_enum.is_some() {
            out.extend([("ifl", ifl), ("lete", lete), ("while", while_let)]);
        } else if facts.is_bool {
            out.extend([("if", "if expr {}"), ("while", "while expr {}")]);
        } else if facts.into_iter {
            out.push(("for", "for ele in expr {}"));
        }
    }
    if facts.is_bool {
        out.push(("not", "!expr"));
    }
    if facts.in_breakable {
        out.push(("break", "break expr"));
    }
    if facts.impls_drop {
        out.push(("drop", "drop(expr)"));
    }
    out
}

/// The whole completion list after `receiver.` from the receiver's inferred type: its fields and methods, the
/// postfix snippets for its type and position, and the `iter().name` items of a collection. `None` when the
/// receiver's type cannot be inferred, so the owner index answers instead.
pub(super) fn live_dot_items(ws: &WorkspaceIndex, ctx: &OwnerCompletionContext<'_>, cursor_byte: u32, overlay_mgr: &OverlayManager) -> Option<Vec<Value>> {
    let DotCompletion { cands, postfix } = {
        let db = live_nav::live_db_for(ws);
        let mut db = db.lock().unwrap_or_else(|e| e.into_inner());
        db.dot_completion(ws, ctx.file, cursor_byte, Some(overlay_mgr))
    };
    // `Drop::drop` cannot be called explicitly; the `drop` postfix snippet stands for it.
    let is_drop_method = |c: &Cand| c.label == "drop" && c.from_trait.as_deref().is_some_and(|t| t.ends_with("Drop"));
    if cands.iter().all(is_drop_method) {
        return None;
    }
    let prefix = ctx.prefix;
    let mut items: Vec<Value> = Vec::with_capacity(cands.len() + 32);
    let mut seen: HashSet<&str> = HashSet::with_capacity(cands.len());
    // In `receiver.name(...)` only methods complete; a field is not callable there.
    let only_methods = postfix.as_ref().is_some_and(|f| f.is_method_call);
    for c in cands.iter().filter(|c| !is_drop_method(c) && !(only_methods && c.kind == ridx_format::DefKind::Field)) {
        if c.label.starts_with(prefix) && seen.insert(c.label.as_str()) {
            let note = c.from_trait.as_deref().map(item::trait_note);
            items.push(item::item(&c.label, note.as_deref(), lsp_completion_kind(c.kind), c.detail.as_deref().unwrap_or_default()));
        }
    }
    let is_iterator = cands.iter().any(|c| c.label == "next" && c.from_trait.as_deref().is_some_and(|t| t.ends_with("Iterator")));
    if !is_iterator && cands.iter().any(|c| c.label == "iter") {
        items.extend(collect_through_iter_items(ws, ctx));
    }
    if postfix.as_ref().is_some_and(|f| f.is_future) && "await".starts_with(prefix) {
        items.push(item::item("await", None, KEYWORD_KIND, "expr.await"));
    }
    items.extend(fixed_postfix_items(prefix, &POSITIONAL_POSTFIX));
    let positional = postfix.as_ref().map_or_else(|| vec![("match", "match expr {}")], positional_postfix);
    for (label, detail) in positional {
        if label.starts_with(prefix) {
            items.push(item::item(label, None, SNIPPET_KIND as u32, detail));
        }
    }
    item::number_sort_text(&mut items);
    Some(items)
}
