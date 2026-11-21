/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `Extract into function` (rust-analyzer's `extract_function`) for a selected expression: it becomes the body of a new
//! `fn fun_name(..) -> T` after the item holding it, called in its place with the locals it reads (prod-code, m=20226).
//! Offered where the result compiles (docs/integration.md §6): every local it reads is a parameter or a `let` of the
//! function whose type ridx knows and which is `Copy` (passed by value, read after as before), the value's type is
//! known (the `let` it initializes, or the function's return type for its tail), and nothing in it leaves the
//! function (`return`, `?`, `break`, `.await`) or names `self` or a macro's arguments. rust-analyzer also extracts
//! statements, takes references and writes `-> _` where it cannot name a type; ridx does not offer those yet.

use super::action_parse::{byte_to_utf16_pos, line_starts_of, utf16_pos_to_byte};
use super::assist_locals::{copy_type, local_of, primitive_shadowed, LetType};
use ridx_syntax::{NodeId, SyntaxKind as K, TokenKind as T};
use serde_json::{json, Value};
use std::ops::Range;

/// The edits extracting the expression `sel` (byte range) of `doc` into a function: its call in place, and the
/// function after the item holding it.
pub(crate) fn extract_edits(doc: &str, sel: Range<usize>, let_type: LetType) -> Option<Vec<(Range<usize>, String)>> {
    if sel.start > sel.end || sel.end > doc.len() {
        return None;
    }
    let tree = ridx_syntax::parse(doc);
    // An identifier beyond ASCII anywhere in the file may be, in its normal form, the very name an ASCII spelling
    // reads (`K` is `K`, `cafe\u{301}` is `café`): no name of the file is compared then (review of 9d489afe).
    if tree.tokens.iter().any(|t| matches!(t.kind, T::Ident | T::RawIdent | T::Lifetime) && !t.text(doc).is_ascii()) {
        return None;
    }
    let sel = sel.start + (doc[sel.clone()].len() - doc[sel.clone()].trim_start().len())..sel.end - (doc[sel.clone()].len() - doc[sel.clone()].trim_end().len());
    let start = tree.covering(sel.start);
    let expr = std::iter::once(start).chain(tree.ancestors(start)).find(|&n| tree.range(n) == sel)?;
    if !(tree.kind(expr).is_expr()) || matches!(tree.kind(expr), K::BlockExpr | K::PathExpr | K::Literal) {
        return None;
    }
    let f = tree.ancestors(expr).find(|&a| tree.kind(a) == K::Fn)?;
    let body = ridx_syntax::sema_fn::fn_body(&tree, f)?;
    // Inside the body, of a function with no generics (the new one would need them), not `const` (it would call a
    // function that is not).
    if !tree.ancestors(expr).any(|a| a == body)
        || tree.ancestors(f).chain(std::iter::once(f)).any(|a| tree.child(a, K::GenericParamList).is_some())
        || tree.own_tokens(f).any(|(_, t)| matches!(t.kind, T::Const | T::Unsafe))
    {
        return None;
    }
    // The context a plain function in the module lacks (review of 531974ed): an `unsafe` or `const` block around it, a
    // `cfg` on the function or what holds it (the helper would outlive what it calls), `track_caller` (the caller
    // reported would be the helper), `target_feature`.
    // From the expression up (a `cfg` on a statement too), by any spelling (`#[r#cfg]`); an array's length and a const
    // argument are evaluated at compile time like a `const` block (review of 5852217f).
    let in_length = |a: NodeId| tree.kind(a) == K::ArrayExpr && tree.own_tokens(a).any(|(_, t)| t.kind == T::Semi && (t.start as usize) < tree.range(expr).start);
    if tree.ancestors(expr).take_while(|&a| a != f).any(|a| {
        (tree.kind(a) == K::BlockExpr && tree.own_tokens(a).any(|(_, t)| matches!(t.kind, T::Unsafe | T::Const)))
            || matches!(tree.kind(a), K::ArrayType | K::ConstArg | K::ConstParam | K::Const | K::Static)
            || in_length(a)
    }) || tree.ancestors(expr).any(|a| {
        tree.children(a)
            .filter(|&c| tree.kind(c) == K::Attr)
            .any(|attr| tree.tokens_of(attr).iter().any(|t| matches!(ridx_syntax::sema_fn::norm_ident(t.text(doc)), "cfg" | "cfg_attr" | "track_caller" | "target_feature")))
    }) {
        return None;
    }
    let inner: Vec<NodeId> = std::iter::once(expr).chain(tree.descendants(expr)).collect();
    // An assignment (`n += 1`) writes the local, which a copy passed would not (review of 531974ed).
    let assigns = |n: NodeId| {
        tree.kind(n) == K::BinExpr && {
            let op: String = tree.own_tokens(n).filter(|(_, t)| !t.is_trivia()).map(|(_, t)| t.text(doc)).collect();
            op.ends_with('=') && !matches!(op.as_str(), "==" | "!=" | "<=" | ">=")
        }
    };
    if inner.iter().any(|&n| {
        matches!(tree.kind(n), K::ReturnExpr | K::TryExpr | K::BreakExpr | K::ContinueExpr | K::AwaitExpr | K::MacroCall | K::IdentPat | K::ClosureExpr | K::RefExpr) || assigns(n)
    }) || tree.tokens_of(expr).iter().any(|t| matches!(t.kind, T::SelfKw | T::SelfType))
    {
        return None;
    }
    // A name the function's body declares (a `const`, a `fn`, a `use`) is not seen from the module (review of 531974ed:
    // a local `N` would become the module's).
    let local_items: Vec<&str> = tree
        .descendants(body)
        .filter(|&d| tree.kind(d) == K::Name && tree.parent(d).is_some_and(|p| tree.kind(p).is_item()))
        .map(|d| ridx_syntax::sema_fn::norm_ident(tree.text(d)))
        .collect();
    // A name beyond ASCII is not compared without its normal form (`café` and `cafe\u{301}` are one name): not
    // extracted (review of 5852217f).
    if tree.descendants(body).any(|d| tree.kind(d) == K::Use)
        || super::assist_locals::macro_may_declare(&tree, tree.root())
        || tree.tokens_of(expr).iter().any(|t| {
            let name = ridx_syntax::sema_fn::norm_ident(t.text(doc));
            matches!(t.kind, T::Ident | T::RawIdent) && (local_items.contains(&name) || !name.is_ascii())
        })
    {
        return None;
    }
    // Nor does a type of the new signature name what the body declares (`type Word = u8` in it).
    let names_local = |ty: &str| ty.split(|c: char| !(c.is_alphanumeric() || c == '_')).any(|w| local_items.contains(&w) || !w.is_ascii());
    let primitive_shadowed = primitive_shadowed(&tree);
    // The value's type: the `let` it initializes, or the function's for its tail.
    let parent = tree.parent(expr)?;
    let ret = if tree.kind(parent) == K::LetStmt {
        let name = tree.children(parent).find(|&c| tree.kind(c) == K::IdentPat).and_then(|p| tree.child(p, K::Name))?;
        tree.children(parent).find(|&c| tree.kind(c).is_type()).map(|t| tree.text(t).to_string()).or_else(|| let_type(tree.range(name).start))?
    } else if ridx_syntax::sema_fn::tail_expr(&tree, f) == Some(expr) {
        tree.text(tree.children(tree.child(f, K::RetType)?).next()?).to_string()
    } else {
        return None;
    };
    // A type the new signature can hold as written: no `impl`, no placeholder, no `Self` (a free function has none),
    // and no reference out (whose lifetime the signature would have to tell: review of 531974ed).
    let nameable = |ty: &str| !ty.contains("impl ") && !names_local(ty) && !ty.split(|c: char| !(c.is_alphanumeric() || c == '_')).any(|w| w == "_" || w == "Self");
    if !nameable(&ret) || ret.contains(['&', '\'']) {
        return None;
    }
    // The locals it reads, in order of first use.
    // A borrow (`&x`) would borrow the copy passed, not the local: no `RefExpr` above. Each local is written in its own
    // spelling (`r#type`) and told apart by its name.
    let mut params: Vec<(String, String, String)> = Vec::new();
    for &n in &inner {
        if tree.kind(n) != K::PathExpr || tree.children(n).count() != 1 {
            continue;
        }
        let toks: Vec<_> = tree.tokens_of(n).iter().filter(|t| !t.is_trivia()).collect();
        let [tok] = toks.as_slice() else { continue };
        if !matches!(tok.kind, T::Ident | T::RawIdent) {
            continue;
        }
        let name = ridx_syntax::sema_fn::norm_ident(tok.text(doc));
        // A receiver, a field's base or an indexed value is borrowed where it is, by the compiler (`n.address()` takes
        // `&n`): the copy passed would be borrowed instead (review of 6c5f3f95).
        // Through parentheses too: `(n).address()` borrows `n` all the same (review of 56f8c6cd).
        let mut outer = n;
        while let Some(p) = tree.parent(outer).filter(|&p| tree.kind(p) == K::ParenExpr) {
            outer = p;
        }
        if tree.parent(outer).is_some_and(|p| matches!(tree.kind(p), K::MethodCallExpr | K::FieldExpr | K::IndexExpr) && tree.children(p).next() == Some(outer)) {
            return None;
        }
        // A comparison borrows both operands (`PartialEq::eq(&n, &p)`), which a user's `PartialEq<&P> for u8` may
        // observe: no copy of them is passed (review of 1db80426).
        let comparison = |p: NodeId| {
            tree.kind(p) == K::BinExpr && {
                let op: String = tree.own_tokens(p).filter(|(_, t)| !t.is_trivia()).map(|(_, t)| t.text(doc)).collect();
                matches!(op.as_str(), "==" | "!=" | "<" | ">" | "<=" | ">=")
            }
        };
        if tree.parent(outer).is_some_and(comparison) {
            return None;
        }
        match local_of(&tree, f, name, tok.start as usize, let_type).ok()? {
            Some(ty) if copy_type(&ty, primitive_shadowed) && nameable(&ty) => {
                if !params.iter().any(|(p, _, _)| p == name) {
                    params.push((name.to_string(), tok.text(doc).to_string(), ty));
                }
            }
            Some(_) => return None,
            None if tree
                .descendants(f)
                .any(|d| tree.kind(d) == K::IdentPat && tree.child(d, K::Name).is_some_and(|nn| ridx_syntax::sema_fn::norm_ident(tree.text(nn)) == name)) =>
            {
                return None
            }
            None => {}
        }
    }
    let name = (0..)
        .map(|i| if i == 0 { "fun_name".to_string() } else { format!("fun_name{i}") })
        .find(|n| !tree.tokens.iter().any(|t| ridx_syntax::sema_fn::norm_ident(t.text(doc)) == n.as_str()))?;
    let args: Vec<&str> = params.iter().map(|(_, raw, _)| raw.as_str()).collect();
    let sig: Vec<String> = params.iter().map(|(_, raw, t)| format!("{raw}: {t}")).collect();
    // After the item of the module that holds the function (the function, or its impl), in that module, so that what
    // the module imports names its types; a function in another function's body is not extracted from.
    let mut top = f;
    for a in tree.ancestors(f) {
        match tree.kind(a) {
            K::Impl => top = a,
            K::SourceFile | K::Module => break,
            K::BlockExpr | K::Fn | K::Trait => return None,
            _ => {}
        }
    }
    let end = tree.range(top).end;
    let line_start = doc[..tree.range(top).start].rfind('\n').map_or(0, |i| i + 1);
    let indent: String = doc[line_start..].chars().take_while(|c| *c == ' ' || *c == '\t').collect();
    let new_fn = format!("\n\n{indent}fn {name}({}) -> {ret} {{\n{indent}    {}\n{indent}}}", sig.join(", "), tree.text(expr));
    Some(vec![(sel, format!("{name}({})", args.join(", "))), (end..end, new_fn)])
}

/// The `Extract into function` action for a non-empty `range`, with its edit.
pub fn extract_fn_action(doc: &str, uri: &str, range: &Value, doc_version: Option<i32>, let_type: LetType) -> Option<Value> {
    if range["start"] == range["end"] {
        return None;
    }
    let starts = line_starts_of(doc);
    let pos = |p: &Value| utf16_pos_to_byte(doc, &starts, p["line"].as_u64()? as u32, p["character"].as_u64()? as u32);
    let edits: Vec<Value> = extract_edits(doc, pos(&range["start"])?..pos(&range["end"])?, let_type)?
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
    Some(json!({ "title": "Extract into function", "kind": "refactor.extract", "edit": edit }))
}

#[cfg(test)]
#[path = "tests/extract_fn_tests.rs"]
mod tests;
