/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `Wrap return type in Option` and `in Result` (rust-analyzer's `wrap_return_type_in_option` and
//! `wrap_return_type_in_result`): `-> T` becomes `-> ::core::option::Option<T>` or `-> ::core::result::Result<T, ()>`,
//! and every value the function returns is wrapped in `Some` or `Ok` (prod-code, m=20226). Offered where the result compiles (docs/integration.md
//! §6): a function nothing calls in the workspace, whose signature no trait fixes; rust-analyzer offers it with callers
//! left broken, and writes the error type as a placeholder `_`, which does not compile.

use super::action_parse::{byte_to_utf16_pos, line_starts_of, utf16_pos_to_byte};
use ridx_format::WorkspaceIndex;
use ridx_syntax::{NodeId, SyntaxKind as K, SyntaxTree, TokenKind as T};
use serde_json::{json, Value};
use std::ops::Range;

/// Which wrapper.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Wrap {
    Option,
    Result,
}

impl Wrap {
    // Written as paths from the `core` crate, which no import, glob or `no_implicit_prelude` changes (review of
    // 36349abf): `Option` and `Some` in scope may be another item's.
    fn ty(self, inner: &str) -> String {
        match self {
            Wrap::Option => format!("::core::option::Option<{inner}>"),
            Wrap::Result => format!("::core::result::Result<{inner}, ()>"),
        }
    }

    fn value(self, inner: &str) -> String {
        match self {
            Wrap::Option => format!("::core::option::Option::Some({inner})"),
            Wrap::Result => format!("::core::result::Result::Ok({inner})"),
        }
    }
}

/// The standard library's macros, which never return from the function they are called in.
const LOCAL_MACROS: &[&str] = &[
    "println",
    "print",
    "eprintln",
    "eprint",
    "format",
    "format_args",
    "write",
    "writeln",
    "vec",
    "assert",
    "assert_eq",
    "assert_ne",
    "debug_assert",
    "debug_assert_eq",
    "debug_assert_ne",
    "panic",
    "unreachable",
    "todo",
    "unimplemented",
    "matches",
    "concat",
    "stringify",
    "line",
    "column",
    "file",
    "env",
    "option_env",
    "include_str",
    "cfg",
];

/// Whether the package of `file` is of the 2018 edition or later, where `::core` names the `core` crate: its
/// `Cargo.toml` says so, or takes its workspace's edition (Cargo has no workspace edition older than 2018 in use).
pub(crate) fn edition_2018(file: &std::path::Path) -> bool {
    let Some(manifest) = file.ancestors().skip(1).map(|d| d.join("Cargo.toml")).find(|m| m.is_file()) else { return false };
    let text = std::fs::read_to_string(manifest).unwrap_or_default();
    text.lines()
        .map(|l| l.split('#').next().unwrap_or("").replace([' ', '\t'], ""))
        .any(|l| l.starts_with("edition.workspace=true") || (l.starts_with("edition=\"20") && !l.starts_with("edition=\"2015")))
}

/// The function whose header (up to its body) covers byte `offset`, with its name: a free function or a method of an
/// inherent impl, with a body and a return type.
fn fn_at(tree: &SyntaxTree<'_>, offset: usize) -> Option<(NodeId, NodeId)> {
    let f = std::iter::once(tree.covering(offset)).chain(tree.ancestors(tree.covering(offset))).find(|&n| tree.kind(n) == K::Fn)?;
    let body = ridx_syntax::sema_fn::fn_body(tree, f)?;
    if offset >= tree.range(body).start {
        return None;
    }
    // A trait's signature, or a trait impl's that must match it, is not this function's to change.
    if let Some(owner) = tree.ancestors(f).find(|&a| matches!(tree.kind(a), K::Trait | K::Impl | K::Fn | K::BlockExpr)) {
        if tree.kind(owner) == K::Trait || (tree.kind(owner) == K::Impl && tree.own_tokens(owner).any(|(_, t)| t.kind == T::For)) {
            return None;
        }
    }
    Some((f, tree.child(f, K::Name)?))
}

/// The edits wrapping the return type of the function at byte `offset` of `doc` and the values it returns, as byte
/// ranges with their new text; `called` says whether anything but its own name names the function elsewhere.
pub(crate) fn wrap_edits(doc: &str, offset: usize, wrap: Wrap, edition_2018: bool, called: &dyn Fn(usize, &str) -> bool) -> Option<Vec<(Range<usize>, String)>> {
    let tree = ridx_syntax::parse(doc);
    let (f, name) = fn_at(&tree, offset)?;
    // `::core` is the crate from 2018 on, unless the file makes a `core` of its own.
    if !edition_2018 || tree.descendants(tree.root()).any(|d| tree.kind(d) == K::Name && tree.text(d) == "core") {
        return None;
    }
    let ty = tree.child(f, K::RetType).and_then(|r| tree.children(r).next())?;
    let ty_text = tree.text(ty);
    let head = ty_text.split('<').next().unwrap_or("").trim();
    // `-> ()` returns by falling through the end too, which no edit wraps (review of 63f24322).
    if tree.kind(ty) == K::NeverType || ty_text.trim() == "()" || head == "Option" || head == "Result" || head.ends_with("::Option") || head.ends_with("::Result") {
        return None;
    }
    let body = ridx_syntax::sema_fn::fn_body(&tree, f)?;
    // Every exit is wrapped: none through `?` (whose type the new one changes), none in a macro's arguments or its
    // expansion (only the standard library's, which return from nothing, are read: review of 36349abf).
    let macro_name = |d: NodeId| tree.tokens_of(d).iter().take_while(|t| t.kind != T::Bang).filter(|t| matches!(t.kind, T::Ident | T::RawIdent)).last().map(|t| t.text(doc));
    if !ridx_syntax::sema_fn::own_try_ops(&tree, f).is_empty()
        || tree.descendants(body).any(|d| {
            tree.kind(d) == K::MacroCall
                && (tree.tokens_of(d).iter().any(|t| t.kind == T::Return || t.kind == T::Question)
                    || !macro_name(d).is_some_and(|m| LOCAL_MACROS.contains(&m) && tree.text(d).starts_with(m)))
        })
    {
        return None;
    }
    // The end of the body returns a value: its tail, or a last `return` (a body falling through its end returns `()`,
    // under whatever alias: review of 36349abf).
    let last = tree.children(body).filter(|&c| tree.kind(c) != K::Attr).last();
    let returns_last =
        last.is_some_and(|l| tree.kind(l) == K::ReturnExpr || (tree.kind(l) == K::ExprStmt && tree.children(l).next().is_some_and(|e| tree.kind(e) == K::ReturnExpr)));
    if ridx_syntax::sema_fn::tail_expr(&tree, f).is_none() && !returns_last {
        return None;
    }
    // Every caller would break: a function named anywhere but at its own name, `r#name` included; a name beyond ASCII
    // is not compared without its normal form.
    let fn_name = ridx_syntax::sema_fn::norm_ident(tree.text(name));
    if !fn_name.is_ascii()
        || tree
            .tokens
            .iter()
            .any(|t| matches!(t.kind, T::Ident | T::RawIdent) && ridx_syntax::sema_fn::norm_ident(t.text(doc)) == fn_name && (t.start as usize) != tree.range(name).start)
        || called(tree.range(name).start, fn_name)
    {
        return None;
    }
    let mut edits = vec![(tree.range(ty), wrap.ty(ty_text))];
    for r in ridx_syntax::sema_fn::own_returns(&tree, f) {
        let value = tree.children(r).find(|&c| tree.kind(c).is_expr() || tree.kind(c) == K::MacroCall)?;
        edits.push((tree.range(value), wrap.value(tree.text(value))));
    }
    if let Some(tail) = ridx_syntax::sema_fn::tail_expr(&tree, f).filter(|&t| tree.kind(t) != K::ReturnExpr) {
        edits.push((tree.range(tail), wrap.value(tree.text(tail))));
    }
    edits.sort_by_key(|(r, _)| r.start);
    // A return inside the tail (`if c { return x } y`) is in the tail's text already: wrap the tail's parts instead.
    if edits.windows(2).any(|w| w[0].0.end > w[1].0.start) {
        return None;
    }
    Some(edits)
}

/// The `Wrap return type in Option` and `in Result` actions at the cursor of an empty `range`, with their edits.
pub fn wrap_return_actions(doc: &str, uri: &str, range: &Value, doc_version: Option<i32>, ws: &WorkspaceIndex, ov: &crate::overlay::OverlayManager) -> Vec<Value> {
    let mut out = Vec::new();
    if range["start"] != range["end"] {
        return out;
    }
    let starts = line_starts_of(doc);
    let Some(offset) = range["start"]["line"].as_u64().zip(range["start"]["character"].as_u64()).and_then(|(l, c)| utf16_pos_to_byte(doc, &starts, l as u32, c as u32)) else {
        return out;
    };
    let file = crate::lsp::protocol::uri_to_path(uri);
    let canonical = ridx_format::canonicalize_best_effort(std::path::Path::new(&file));
    // Named in another file of the workspace, by the index (this one's text was read already), or in another open
    // buffer, whose unsaved calls the index may not have yet (review of 63f24322); a definition the index does not have
    // is not known to be unused.
    let called = |name_start: usize, name: &str| {
        let line = doc[..name_start].matches('\n').count();
        let char_col = doc[starts[line]..name_start].chars().count();
        let in_buffer = ov.open_file_paths().into_iter().filter(|k| ridx_format::canonicalize_best_effort(std::path::Path::new(k)) != canonical).any(|k| {
            ov.get(&k)
                .is_some_and(|o| ridx_syntax::lex(&o.text).iter().any(|t| matches!(t.kind, T::Ident | T::RawIdent) && ridx_syntax::sema_fn::norm_ident(t.text(&o.text)) == name))
        });
        let Some(def) = ws.find_definition_at(&file, line as u32 + 1, char_col as u32 + 1) else { return true };
        in_buffer || ws.find_references(def.path).iter().any(|r| ridx_format::canonicalize_best_effort(&r.resolved_path()) != canonical)
    };
    for (wrap, title) in [(Wrap::Option, "Wrap return type in Option"), (Wrap::Result, "Wrap return type in Result")] {
        let Some(edits) = wrap_edits(doc, offset, wrap, edition_2018(&canonical), &called) else { continue };
        let edits: Vec<Value> = edits
            .into_iter()
            .map(|(r, text)| {
                let ((sl, sc), (el, ec)) = (byte_to_utf16_pos(doc, &starts, r.start), byte_to_utf16_pos(doc, &starts, r.end));
                json!({ "range": { "start": { "line": sl, "character": sc }, "end": { "line": el, "character": ec } }, "newText": text })
            })
            .collect();
        let edit = match doc_version {
            Some(v) => json!({ "documentChanges": [{ "textDocument": { "uri": uri, "version": v }, "edits": edits }] }),
            None => json!({ "changes": { uri: edits } }),
        };
        out.push(json!({ "title": title, "kind": "refactor.rewrite", "edit": edit }));
    }
    out
}

#[cfg(test)]
#[path = "tests/wrap_return_tests.rs"]
mod tests;
