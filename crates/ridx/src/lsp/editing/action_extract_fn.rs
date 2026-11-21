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
use ridx_syntax::{NodeId, SyntaxKind as K, SyntaxTree, TokenKind as T};
use serde_json::{json, Value};
use std::ops::Range;

/// What the extraction needs to know from outside the syntax tree: the type of the `let` whose name is at a byte
/// offset (written on it, or as `Add explicit type` would write it).
pub(crate) type LetType<'a> = &'a dyn Fn(usize) -> Option<String>;

/// Whether a value of type `ty` is copied when passed: a primitive or a shared reference.
fn copy_type(ty: &str) -> bool {
    let ty = ty.trim();
    (ty.starts_with('&') && !ty.starts_with("&mut"))
        || matches!(ty, "u8" | "u16" | "u32" | "u64" | "u128" | "usize" | "i8" | "i16" | "i32" | "i64" | "i128" | "isize" | "f32" | "f64" | "bool" | "char")
}

/// The local `name`, read at byte `at` inside `f`, with its type: a parameter, or a `let` before the selection whose
/// uses include `at`. `Err` when the name is bound otherwise (a pattern, a closure's parameter): not known.
fn local_of(tree: &SyntaxTree<'_>, f: NodeId, name: &str, at: usize, let_type: LetType) -> Result<Option<String>, ()> {
    for p in ridx_syntax::sema_fn::params(tree, f) {
        if p.bindings.as_slice() == [name] && p.pat.is_some_and(|pat| tree.kind(pat) == K::IdentPat) {
            let uses = ridx_syntax::sema_fn::param_uses(tree, f, name);
            if uses.uses.iter().any(|r| r.start == at) {
                return Ok(p.ty_text.map(str::to_string));
            }
        }
    }
    let mut bound_otherwise = false;
    for pat in tree.descendants(f).filter(|&n| tree.kind(n) == K::IdentPat && tree.range(n).end <= at) {
        let Some(n) = tree.child(pat, K::Name).filter(|&n| ridx_syntax::sema_fn::norm_ident(tree.text(n)) == name) else { continue };
        match tree.parent(pat).filter(|&s| tree.kind(s) == K::LetStmt) {
            Some(stmt) if ridx_syntax::sema_fn::let_uses(tree, stmt, name).uses.iter().any(|r| r.start == at) => {
                let written = tree.children(stmt).find(|&c| tree.kind(c).is_type()).map(|t| tree.text(t).to_string());
                return Ok(written.or_else(|| let_type(tree.range(n).start)));
            }
            Some(_) => {}
            None => bound_otherwise = true,
        }
    }
    if bound_otherwise {
        Err(())
    } else {
        Ok(None)
    }
}

/// The edits extracting the expression `sel` (byte range) of `doc` into a function: its call in place, and the
/// function after the item holding it.
pub(crate) fn extract_edits(doc: &str, sel: Range<usize>, let_type: LetType) -> Option<Vec<(Range<usize>, String)>> {
    let tree = ridx_syntax::parse(doc);
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
        || tree.own_tokens(f).any(|(_, t)| t.kind == T::Const)
    {
        return None;
    }
    let inner: Vec<NodeId> = std::iter::once(expr).chain(tree.descendants(expr)).collect();
    if inner.iter().any(|&n| matches!(tree.kind(n), K::ReturnExpr | K::TryExpr | K::BreakExpr | K::ContinueExpr | K::AwaitExpr | K::MacroCall | K::IdentPat | K::ClosureExpr))
        || tree.tokens_of(expr).iter().any(|t| matches!(t.kind, T::SelfKw | T::SelfType))
    {
        return None;
    }
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
    if ret.contains("impl ") || ret.contains('_') && ret.split(|c: char| !(c.is_alphanumeric() || c == '_')).any(|w| w == "_") {
        return None;
    }
    // The locals it reads, in order of first use.
    let mut params: Vec<(String, String)> = Vec::new();
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
        match local_of(&tree, f, name, tok.start as usize, let_type).ok()? {
            Some(ty) if copy_type(&ty) => {
                if !params.iter().any(|(p, _)| p == name) {
                    params.push((name.to_string(), ty));
                }
            }
            Some(_) => return None,
            None if tree.descendants(f).any(|d| tree.kind(d) == K::IdentPat && tree.child(d, K::Name).is_some_and(|nn| tree.text(nn) == name)) => return None,
            None => {}
        }
    }
    let name = (0..)
        .map(|i| if i == 0 { "fun_name".to_string() } else { format!("fun_name{i}") })
        .find(|n| !tree.tokens.iter().any(|t| ridx_syntax::sema_fn::norm_ident(t.text(doc)) == n.as_str()))?;
    let args: Vec<&str> = params.iter().map(|(p, _)| p.as_str()).collect();
    let sig: Vec<String> = params.iter().map(|(p, t)| format!("{p}: {t}")).collect();
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
