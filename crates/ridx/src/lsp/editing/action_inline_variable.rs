/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `Inline variable` (rust-analyzer's `inline_local_variable`): a `let` binding used once is replaced at its use by
//! its initializer, and the `let` goes (prod-code, m=20226). Offered only where that provably means what the code
//! meant and compiles (docs/integration.md §6, reviews of 63f24322 and 36349abf):
//!
//! - the initializer is a value with nothing to evaluate: a literal with a fixed type, a `Copy` local whose type is
//!   written on it (a parameter, an annotated `let`: an inferred one may be stale, review of 03933e37), or those in
//!   parentheses, a tuple, an array or a cast to a primitive. No operator (whose overflow or order of evaluation the
//!   move would change), call, macro, field, index or borrow: nothing to drop, move, panic or run earlier or later;
//! - no other binding or item of the function shares a name the initializer reads: the name means the same at the use;
//! - the use is a value read: not borrowed, not a method's receiver (both see the binding's own storage), through
//!   parentheses too, not in a loop, a closure or an `async` block the `let` is outside of.

use super::action_parse::{byte_to_utf16_pos, line_starts_of, utf16_pos_to_byte};
use super::assist_locals::{copy_type, local_of, primitive_shadowed, PRIMITIVES};
use ridx_syntax::{NodeId, SyntaxKind as K, SyntaxTree, TokenKind as T};
use serde_json::{json, Value};

/// Where a use takes any expression as it is.
const OPEN: &[K] = &[K::ArgList, K::LetStmt, K::BlockExpr, K::ExprStmt, K::ReturnExpr, K::ParenExpr, K::ArrayExpr, K::TupleExpr, K::RecordExprField];

/// Integer suffixes, which no digit of any radix spells.
const INT_SUFFIXES: &[&str] = &["u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize"];

/// Whether the number literal `t` has a type of its own: an integer suffix, or a float one on a decimal literal (`0xff32`
/// is hexadecimal digits, review of 36349abf).
fn typed_number(t: &str) -> bool {
    INT_SUFFIXES.iter().any(|s| t.ends_with(s)) || (!t.starts_with("0x") && !t.starts_with("0o") && !t.starts_with("0b") && (t.ends_with("f32") || t.ends_with("f64")))
}

/// Whether the use at byte `at` is a field shorthand (`S { x }`, attributes before it or not), which names the field
/// too.
pub(crate) fn is_shorthand(tree: &SyntaxTree<'_>, at: usize) -> bool {
    let c = tree.covering(at);
    std::iter::once(c).chain(tree.ancestors(c)).find(|&n| tree.kind(n) == K::RecordExprField).is_some_and(|f| !tree.own_tokens(f).any(|(_, t)| t.kind == T::Colon))
}

/// Whether the initializer `n` is a value with nothing to evaluate (no operator, call or effect), and of a primitive
/// type when `Some(true)`. `local` gives the type written on the local read at a byte.
fn pure(tree: &SyntaxTree<'_>, n: NodeId, local: &dyn Fn(&str, usize) -> Option<String>, shadowed: bool) -> Option<bool> {
    let kids: Vec<NodeId> = tree.children(n).filter(|&c| tree.kind(c).is_expr()).collect();
    match tree.kind(n) {
        K::Literal => {
            let t = tree.text(n);
            if t.starts_with(|c: char| c.is_ascii_digit()) {
                (typed_number(t) && !shadowed).then_some(true)
            } else {
                Some(matches!(t, "true" | "false") || t.starts_with('\'') && !shadowed)
            }
        }
        K::PathExpr => {
            let toks: Vec<_> = tree.tokens_of(n).iter().filter(|t| !t.is_trivia()).collect();
            let [tok] = toks.as_slice() else { return None };
            let name = ridx_syntax::sema_fn::norm_ident(tok.text(tree.src));
            let ty = local(name, tok.start as usize).filter(|ty| copy_type(ty, shadowed))?;
            Some(PRIMITIVES.contains(&ty.trim()))
        }
        K::ParenExpr => pure(tree, *kids.first()?, local, shadowed),
        // A cast between primitives neither panics nor runs code.
        K::CastExpr => {
            let prim = pure(tree, *kids.first()?, local, shadowed)?;
            let target = tree.children(n).find(|&c| tree.kind(c).is_type()).map(|t| tree.text(t).trim().to_string())?;
            (prim && !shadowed && PRIMITIVES.contains(&target.as_str())).then_some(true)
        }
        K::TupleExpr | K::ArrayExpr => {
            for k in kids {
                pure(tree, k, local, shadowed)?;
            }
            Some(false)
        }
        _ => None,
    }
}

/// The `let` statement whose binding's name covers byte `offset` of `tree`, with the name's node and the initializer:
/// a plain binding (no `mut`, no `ref`, no sub-pattern), no type written (the initializer's type would be inferred
/// again where it goes) and no `else`.
fn plain_let(tree: &SyntaxTree<'_>, offset: usize) -> Option<(NodeId, NodeId, NodeId)> {
    // The cursor just after the name (`area|`) is on it too.
    let at = |o: usize| Some(tree.covering(o)).filter(|&n| tree.kind(n) == K::Name);
    let name = at(offset).or_else(|| offset.checked_sub(1).and_then(at))?;
    let pat = tree.parent(name).filter(|&p| tree.kind(p) == K::IdentPat)?;
    let stmt = tree.parent(pat).filter(|&s| tree.kind(s) == K::LetStmt)?;
    if tree.own_tokens(pat).any(|(_, t)| matches!(t.kind, T::Mut | T::Ref)) || tree.children(pat).count() != 1 {
        return None;
    }
    let mut init = None;
    for c in tree.children(stmt) {
        match tree.kind(c) {
            _ if c == pat => {}
            k if (k.is_expr() || k == K::MacroCall) && init.is_none() => init = Some(c),
            _ => return None,
        }
    }
    Some((stmt, name, init?))
}

/// The edit of `Inline variable` at byte `offset` of `doc`: the deletion of the `let` and the initializer at the use,
/// as byte ranges with their new text, in order.
pub(crate) fn inline_edits(doc: &str, offset: usize) -> Option<Vec<(std::ops::Range<usize>, String)>> {
    let tree = ridx_syntax::parse(doc);
    let (stmt, name, init) = plain_let(&tree, offset)?;
    let block = tree.parent(stmt)?;
    let f = tree.ancestors(stmt).find(|&a| tree.kind(a) == K::Fn)?;
    // Names compared as Rust compares them: `r#n` is `n`; a name beyond ASCII is not compared without its normal form.
    let idents: Vec<&str> =
        tree.tokens_of(init).iter().filter(|t| matches!(t.kind, T::Ident | T::RawIdent | T::SelfKw)).map(|t| ridx_syntax::sema_fn::norm_ident(t.text(doc))).collect();
    // Every name the function declares, a binding or an item (one declared later is in scope before it: review of
    // 03933e37); a name read that two of them share may mean another one at the use.
    let declared: Vec<&str> = tree
        .descendants(f)
        .filter(|&d| tree.kind(d) == K::Name && tree.parent(d).is_some_and(|p| tree.kind(p) == K::IdentPat || tree.kind(p).is_item()))
        .map(|d| ridx_syntax::sema_fn::norm_ident(tree.text(d)))
        .collect();
    if !tree.text(name).is_ascii() || idents.iter().any(|i| !i.is_ascii()) || declared.iter().any(|d| !d.is_ascii()) {
        return None;
    }
    // A `use` in the function brings names into its whole block, later ones too, whatever they are: none is compared
    // (review of b854363f).
    if idents.iter().any(|i| declared.iter().filter(|d| *d == i).count() > 1)
        || tree.descendants(f).any(|d| tree.kind(d) == K::Use)
        || super::assist_locals::macro_may_declare(&tree, f)
    {
        return None;
    }
    let uses = ridx_syntax::sema_fn::let_uses(&tree, stmt, ridx_syntax::sema_fn::norm_ident(tree.text(name)));
    let [use_range] = uses.uses.as_slice() else { return None };
    if !uses.macro_uses.is_empty() || is_shorthand(&tree, use_range.start) {
        return None;
    }
    let path = std::iter::once(tree.covering(use_range.start)).chain(tree.ancestors(tree.covering(use_range.start))).find(|&n| tree.kind(n) == K::PathExpr)?;
    let parent = tree.parent(path)?;
    // A value read, evaluated once where the `let` was: not borrowed or a receiver (which see the binding's storage),
    // not in a loop, a closure or an `async` block (run another number of times, or later) the `let` is outside of.
    // Parentheses change neither (`&(x)`, `(x).f()`: review of 03933e37).
    let mut outer = path;
    while let Some(p) = tree.parent(outer).filter(|&p| tree.kind(p) == K::ParenExpr) {
        outer = p;
    }
    let holder = tree.parent(outer)?;
    // Only where the value is copied into a place of its own: an argument, a `let` binding it by value, a returned,
    // tupled, cast or arithmetic operand, a block's value. Not a `match` or `if let` scrutinee (`ref r` borrows it), a
    // comparison (`==` takes `&self`, which a user's `PartialEq<P> for u8` may observe), a receiver, a borrow (review of
    // b854363f).
    let op = || tree.own_tokens(holder).filter(|(_, t)| !t.is_trivia()).map(|(_, t)| t.text(doc)).collect::<String>();
    let first = tree.children(holder).next() == Some(outer);
    let by_value = match tree.kind(holder) {
        K::ArgList | K::ReturnExpr | K::TupleExpr | K::ArrayExpr | K::CastExpr | K::RecordExprField | K::ExprStmt | K::PrefixExpr => true,
        K::BlockExpr => ridx_syntax::sema_fn::tail_expr(&tree, f) == Some(outer) || tree.children(holder).filter(|&c| tree.kind(c).is_expr()).last() == Some(outer),
        K::LetStmt => tree
            .children(holder)
            .find(|&c| tree.kind(c) == K::IdentPat)
            .is_some_and(|p| !tree.own_tokens(p).any(|(_, t)| matches!(t.kind, T::Ref | T::Mut)) && tree.children(p).count() == 1),
        K::BinExpr => {
            let o = op();
            matches!(o.as_str(), "+" | "-" | "*" | "/" | "%" | "&" | "|" | "^" | "<<" | ">>" | "&&" | "||")
                || (o.ends_with('=') && !matches!(o.as_str(), "==" | "!=" | "<=" | ">=") && !first)
        }
        _ => false,
    };
    if !by_value {
        return None;
    }
    for a in tree.ancestors(path).take_while(|&a| a != block) {
        let async_block = tree.kind(a) == K::BlockExpr && tree.own_tokens(a).any(|(_, t)| t.kind == T::Async);
        if async_block || matches!(tree.kind(a), K::ClosureExpr | K::LoopExpr | K::WhileExpr | K::ForExpr) {
            return None;
        }
    }
    let shadowed = primitive_shadowed(&tree);
    // Only a type written on the local: one inferred from the index may be stale (review of 03933e37).
    let local = |n: &str, at: usize| local_of(&tree, f, n, at, &|_| None).ok().flatten();
    pure(&tree, init, &local, shadowed)?;
    // Nothing between the `let` and the use names what the initializer reads: it reads the same values there.
    let between = tree.range(stmt).end..tree.range(path).start;
    if tree
        .tokens
        .iter()
        .filter(|t| between.contains(&(t.start as usize)))
        .any(|t| matches!(t.kind, T::Ident | T::RawIdent | T::SelfKw) && idents.contains(&ridx_syntax::sema_fn::norm_ident(t.text(doc))))
    {
        return None;
    }
    let text = tree.text(init);
    let atomic = matches!(tree.kind(init), K::Literal | K::PathExpr | K::ParenExpr | K::TupleExpr | K::ArrayExpr);
    let new_text = if !atomic && !OPEN.contains(&tree.kind(parent)) { format!("({text})") } else { text.to_string() };
    Some(vec![(statement_span(doc, tree.range(stmt)), String::new()), (tree.range(path), new_text)])
}

/// The bytes a statement at `range` takes: its whole lines when nothing else is on them, else the statement and the
/// spaces after it.
fn statement_span(doc: &str, range: std::ops::Range<usize>) -> std::ops::Range<usize> {
    let line_start = doc[..range.start].rfind('\n').map_or(0, |i| i + 1);
    let rest = &doc[range.end..];
    let line_end = rest.find('\n').map_or(doc.len(), |i| range.end + i + 1);
    if doc[line_start..range.start].trim().is_empty() && doc[range.end..line_end].trim().is_empty() {
        return line_start..line_end;
    }
    range.start..range.end + (rest.len() - rest.trim_start_matches([' ', '\t']).len())
}

/// The `Inline variable` action at the cursor of an empty `range`, with its edit.
pub fn inline_variable_action(doc: &str, uri: &str, range: &Value, doc_version: Option<i32>) -> Option<Value> {
    if range["start"] != range["end"] {
        return None;
    }
    let starts = line_starts_of(doc);
    let offset = utf16_pos_to_byte(doc, &starts, range["start"]["line"].as_u64()? as u32, range["start"]["character"].as_u64()? as u32)?;
    let edits: Vec<Value> = inline_edits(doc, offset)?
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
    Some(json!({ "title": "Inline variable", "kind": "refactor.inline", "edit": edit }))
}

#[cfg(test)]
#[path = "tests/inline_variable_tests.rs"]
mod tests;
