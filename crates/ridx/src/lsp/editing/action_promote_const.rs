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
//! `const VALUE: i32 = 1;` and its uses `VALUE` (prod-code, m=20226). Offered where the result compiles
//! (docs/integration.md §6): a lone literal, whose type is written on the binding or spelled by the literal itself
//! (`8u64`, `true`, `'c'`, `"hi"`). An operator is not promoted: a constant evaluates it at compile time, where an
//! overflow or a division by zero no longer compiles (`1u8 / 0u8`, review of 03933e37); rust-analyzer also promotes
//! one that reads parameters (`size * scale`), which no constant can.

use super::action_parse::{byte_to_utf16_pos, line_starts_of, utf16_pos_to_byte};
use ridx_syntax::{SyntaxKind as K, TokenKind as T};
use serde_json::{json, Value};
use std::ops::Range;

/// The type a literal spells by itself, or `None` where its type is inferred (`1`, `1.5`) or not a plain one (`b"a"`).
fn literal_type(t: &str) -> Option<&'static str> {
    const NUMBERS: &[&str] = &["u128", "usize", "u16", "u32", "u64", "u8", "i128", "isize", "i16", "i32", "i64", "i8", "f32", "f64"];
    match t {
        "true" | "false" => Some("bool"),
        _ if t.starts_with('\'') => Some("char"),
        _ if t.starts_with('"') || t.starts_with("r\"") || t.starts_with("r#") => Some("&str"),
        // A float suffix on hexadecimal digits is digits (`0xf32`).
        _ if t.starts_with(|c: char| c.is_ascii_digit()) => {
            NUMBERS.iter().find(|s| t.ends_with(*s)).filter(|s| !(s.starts_with('f') && (t.starts_with("0x") || t.starts_with("0o") || t.starts_with("0b")))).copied()
        }
        _ => None,
    }
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

/// The edits promoting the `let` whose name covers byte `offset` of `doc` to a constant of the type written on the
/// binding, else the one its literal spells, as byte ranges with their new text.
pub(crate) fn promote_edits(doc: &str, offset: usize, std_only: super::assist_locals::StdOnly<'_>) -> Option<Vec<(Range<usize>, String)>> {
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
    if tree.kind(init) != K::Literal {
        return None;
    }
    // Whatever type is written, a macro may declare the constant's name already (review of f60136bc).
    if super::assist_locals::macro_may_declare(&tree, tree.root(), std_only) {
        return None;
    }
    // The primitive a literal spells is that primitive only while the file names nothing so.
    let ty = match written {
        Some(w) => w,
        None => literal_type(tree.text(init)).filter(|_| !super::assist_locals::primitive_shadowed(&tree, std_only))?.to_string(),
    };
    // A constant's type is an item's: no placeholder, no generic parameter of the function or impl around it (review
    // of 63f24322: `const X: _` and `[T; 0]` in `fn f<T>()`).
    // `Self` is the enclosing impl's, which a constant cannot see either (review of 36349abf).
    if ty.split(|c: char| !(c.is_alphanumeric() || c == '_')).any(|w| w == "_" || w == "Self")
        || tree.ancestors(stmt).any(|a| tree.child(a, K::GenericParamList).is_some())
        || !tree.text(name).is_ascii()
    {
        return None;
    }
    let var = ridx_syntax::sema_fn::norm_ident(tree.text(name));
    let upper = const_name(var);
    // The new name is free, `r#N` included, and no string formats the binding by name (`{area}`), which the rename
    // would miss.
    if tree.tokens.iter().any(|t| matches!(t.kind, T::Ident | T::RawIdent) && ridx_syntax::sema_fn::norm_ident(t.text(doc)) == upper)
        || tree.tokens.iter().filter(|t| t.kind.is_literal() && (t.start as usize) > tree.range(stmt).end).any(|t| {
            let s = t.text(doc);
            s.contains(&format!("{{{var}}}")) || s.contains(&format!("{{{var}:"))
        })
    {
        return None;
    }
    // An identifier beyond ASCII may be, in its normal form, the new name (`K` is `K`): none is in the file (review of
    // b854363f).
    if tree.tokens.iter().any(|t| matches!(t.kind, T::Ident | T::RawIdent) && !t.text(doc).is_ascii()) {
        return None;
    }
    let uses = ridx_syntax::sema_fn::let_uses(&tree, stmt, var);
    // A same-spelled token in a macro's arguments is not known to be a use (`matches!(2u8, x)` binds a new `x`), and a
    // raw borrow needs the local's place, which a constant is not (`&raw const x`: review of b854363f).
    let raw_borrow = |r: &Range<usize>| {
        let mut n = tree.covering(r.start);
        while let Some(p) = tree.parent(n).filter(|&p| matches!(tree.kind(p), K::PathExpr | K::ParenExpr | K::Path | K::PathSegment)) {
            n = p;
        }
        tree.parent(n).is_some_and(|p| tree.kind(p) == K::RefExpr && tree.own_tokens(p).any(|(_, t)| t.text(doc) == "raw"))
    };
    if !uses.macro_uses.is_empty() || uses.uses.iter().any(raw_borrow) {
        return None;
    }
    let mut edits = vec![(tree.range(stmt), format!("const {upper}: {ty} = {};", tree.text(init)))];
    // A field shorthand (`S { x }`) keeps its field: `S { x: X }` (review of 63f24322).
    let shorthand = |r: &Range<usize>| super::action_inline_variable::is_shorthand(&tree, r.start);
    edits.extend(uses.uses.iter().map(|r| (r.clone(), if shorthand(r) { format!("{}: {upper}", &doc[r.clone()]) } else { upper.clone() })));
    edits.sort_by_key(|(r, _)| r.start);
    Some(edits)
}

/// The `Promote local to constant` action at the cursor of an empty `range`, with its edit.
pub fn promote_const_action(doc: &str, uri: &str, range: &Value, doc_version: Option<i32>, std_only: super::assist_locals::StdOnly<'_>) -> Option<Value> {
    if range["start"] != range["end"] {
        return None;
    }
    let starts = line_starts_of(doc);
    let offset = utf16_pos_to_byte(doc, &starts, range["start"]["line"].as_u64()? as u32, range["start"]["character"].as_u64()? as u32)?;
    let edits: Vec<Value> = promote_edits(doc, offset, std_only)?
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
