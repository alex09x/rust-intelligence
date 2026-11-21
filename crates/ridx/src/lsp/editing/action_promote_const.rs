/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `Promote local to constant` (rust-analyzer's `promote_local_to_const`): `let area = 4 * 8;` becomes
//! `const AREA: i32 = 4 * 8;` and its uses `AREA` (prod-code, m=20226). Offered where the result compiles
//! (docs/integration.md §6): an initializer of literals and operators alone, whose type ridx can write; rust-analyzer
//! also promotes one that reads parameters (`size * scale`), which no constant can.

use super::action_parse::{byte_to_utf16_pos, line_starts_of, utf16_pos_to_byte};
use crate::overlay::OverlayManager;
use ridx_format::WorkspaceIndex;
use ridx_syntax::{SyntaxKind as K, SyntaxTree, TokenKind as T};
use serde_json::{json, Value};
use std::ops::Range;

/// Whether the expression `init` is evaluated at compile time as it is: literals, operators, parentheses, tuples and
/// arrays, and no name.
fn const_evaluable(tree: &SyntaxTree<'_>, init: ridx_syntax::NodeId) -> bool {
    let kinds_ok = std::iter::once(init).chain(tree.descendants(init)).all(|n| {
        matches!(
            tree.kind(n),
            K::Literal | K::BinExpr | K::PrefixExpr | K::ParenExpr | K::TupleExpr | K::ArrayExpr | K::CastExpr | K::PathType | K::Path | K::PathSegment | K::Name
        )
    });
    // A cast names its type (`1 as u8`), a primitive; no other name.
    kinds_ok && !tree.tokens_of(init).iter().any(|t| matches!(t.kind, T::Ident | T::RawIdent | T::SelfKw) && !is_primitive(t.text(tree.src)))
}

fn is_primitive(s: &str) -> bool {
    matches!(s, "u8" | "u16" | "u32" | "u64" | "u128" | "usize" | "i8" | "i16" | "i32" | "i64" | "i128" | "isize" | "f32" | "f64" | "bool" | "char")
}

/// `name` in upper snake case, as a constant is named.
fn const_name(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() && i > 0 && !out.ends_with('_') {
            out.push('_');
        }
        out.extend(c.to_uppercase());
    }
    out
}

/// The edits promoting the `let` whose name covers byte `offset` of `doc` to a constant of type `ty` (the type
/// written on the binding, else `inferred`'s), as byte ranges with their new text.
pub(crate) fn promote_edits(doc: &str, offset: usize, inferred: &dyn Fn() -> Option<String>) -> Option<Vec<(Range<usize>, String)>> {
    let tree = ridx_syntax::parse(doc);
    let at = |o: usize| Some(tree.covering(o)).filter(|&n| tree.kind(n) == K::Name);
    let name = at(offset).or_else(|| offset.checked_sub(1).and_then(at))?;
    let pat = tree.parent(name).filter(|&p| tree.kind(p) == K::IdentPat)?;
    let stmt = tree.parent(pat).filter(|&s| tree.kind(s) == K::LetStmt)?;
    if tree.own_tokens(pat).any(|(_, t)| matches!(t.kind, T::Mut | T::Ref)) || tree.children(pat).count() != 1 {
        return None;
    }
    let (mut init, mut written) = (None, None);
    for c in tree.children(stmt) {
        match tree.kind(c) {
            _ if c == pat => {}
            k if (k.is_expr() || k == K::MacroCall) && init.is_none() => init = Some(c),
            k if k.is_type() && written.is_none() && init.is_none() => written = Some(tree.text(c).to_string()),
            _ => return None,
        }
    }
    let init = init?;
    if !const_evaluable(&tree, init) {
        return None;
    }
    let ty = written.or_else(inferred)?;
    let var = ridx_syntax::sema_fn::norm_ident(tree.text(name));
    let upper = const_name(var);
    // The new name is free, and no string formats the binding by name (`{area}`), which the rename would miss.
    if tree.tokens.iter().any(|t| matches!(t.kind, T::Ident | T::RawIdent) && t.text(doc) == upper)
        || tree.tokens.iter().filter(|t| t.kind.is_literal() && (t.start as usize) > tree.range(stmt).end).any(|t| {
            let s = t.text(doc);
            s.contains(&format!("{{{var}}}")) || s.contains(&format!("{{{var}:"))
        })
    {
        return None;
    }
    let uses = ridx_syntax::sema_fn::let_uses(&tree, stmt, var);
    let mut edits = vec![(tree.range(stmt), format!("const {upper}: {ty} = {};", tree.text(init)))];
    edits.extend(uses.uses.iter().chain(&uses.macro_uses).map(|r| (r.clone(), upper.clone())));
    edits.sort_by_key(|(r, _)| r.start);
    Some(edits)
}

/// The `Promote local to constant` action at the cursor of an empty `range`, with its edit.
pub fn promote_const_action(doc: &str, uri: &str, range: &Value, doc_version: Option<i32>, ws: &WorkspaceIndex, ov: &OverlayManager) -> Option<Value> {
    if range["start"] != range["end"] {
        return None;
    }
    let starts = line_starts_of(doc);
    let offset = utf16_pos_to_byte(doc, &starts, range["start"]["line"].as_u64()? as u32, range["start"]["character"].as_u64()? as u32)?;
    // The type `Add explicit type` writes there.
    let inferred = || {
        let explicit = super::action_explicit_type::explicit_type_action(doc, uri, range, doc_version, ws, ov)?;
        let edit = explicit["edit"]["documentChanges"][0]["edits"][0].clone();
        let edit = if edit.is_null() { explicit["edit"]["changes"][uri][0].clone() } else { edit };
        edit["newText"].as_str()?.strip_prefix(": ").map(str::to_string)
    };
    let edits: Vec<Value> = promote_edits(doc, offset, &inferred)?
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
    Some(json!({ "title": "Promote local to constant", "kind": "refactor.rewrite", "edit": edit }))
}

#[cfg(test)]
#[path = "tests/promote_const_tests.rs"]
mod tests;
