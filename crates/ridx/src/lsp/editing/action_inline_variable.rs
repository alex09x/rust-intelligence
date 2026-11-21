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
//! - the initializer reads only `Copy` locals whose type ridx knows and literals with a fixed type, combined by the
//!   built-in operators of primitives (no overloaded operator runs code), no assignment, call, macro, field, index or
//!   borrow: nothing to drop, move or run earlier or later;
//! - the use is a value read: not borrowed, not a method's receiver (both see the binding's own storage), not in a
//!   loop, a closure or an `async` block the `let` is outside of;
//! - with arithmetic, which may overflow, the use is in the statement right after the `let`, evaluated first there and
//!   on every path, so the same panic comes first.

use super::action_parse::{byte_to_utf16_pos, line_starts_of, utf16_pos_to_byte};
use super::assist_locals::{copy_type, local_of, primitive_shadowed, LetType, PRIMITIVES};
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

/// The operator of the binary expression `n` (`+`, `&&`, `<<`, ...).
fn op(tree: &SyntaxTree<'_>, n: NodeId) -> String {
    tree.own_tokens(n).filter(|(_, t)| !t.is_trivia()).map(|(_, t)| t.text(tree.src)).collect()
}

/// What the initializer `n` is made of, when it is evaluated anywhere to the same value with no effect: `Some((true,
/// _))` for a primitive's value, which built-in operators combine, `Some((_, true))` when it has arithmetic, which may
/// panic on overflow. `local` gives the type of the local read at a byte, if ridx knows it.
fn pure(tree: &SyntaxTree<'_>, n: NodeId, local: &dyn Fn(&str, usize) -> Option<String>, shadowed: bool) -> Option<(bool, bool)> {
    let kids: Vec<NodeId> = tree.children(n).filter(|&c| tree.kind(c).is_expr()).collect();
    match tree.kind(n) {
        K::Literal => {
            let t = tree.text(n);
            if t.starts_with(|c: char| c.is_ascii_digit()) {
                (typed_number(t) && !shadowed).then_some((true, false))
            } else {
                Some((matches!(t, "true" | "false") || t.starts_with('\'') && !shadowed, false))
            }
        }
        K::PathExpr => {
            let toks: Vec<_> = tree.tokens_of(n).iter().filter(|t| !t.is_trivia()).collect();
            let [tok] = toks.as_slice() else { return None };
            let name = ridx_syntax::sema_fn::norm_ident(tok.text(tree.src));
            let ty = local(name, tok.start as usize).filter(|ty| copy_type(ty, shadowed))?;
            Some((PRIMITIVES.contains(&ty.trim()), false))
        }
        K::ParenExpr => pure(tree, *kids.first()?, local, shadowed),
        K::CastExpr => {
            let (prim, arith) = pure(tree, *kids.first()?, local, shadowed)?;
            let target = tree.children(n).find(|&c| tree.kind(c).is_type()).map(|t| tree.text(t).trim().to_string())?;
            (prim && !shadowed && PRIMITIVES.contains(&target.as_str())).then_some((true, arith))
        }
        K::PrefixExpr => {
            let (prim, arith) = pure(tree, *kids.first()?, local, shadowed)?;
            let o = op(tree, n);
            (prim && (o == "-" || o == "!")).then_some((true, arith || o == "-"))
        }
        K::BinExpr => {
            let o = op(tree, n);
            if o.ends_with('=') && !matches!(o.as_str(), "==" | "!=" | "<=" | ">=") {
                return None;
            }
            let [l, r] = kids.as_slice() else { return None };
            let ((lp, la), (rp, ra)) = (pure(tree, *l, local, shadowed)?, pure(tree, *r, local, shadowed)?);
            (lp && rp).then_some((true, la || ra || matches!(o.as_str(), "+" | "-" | "*" | "/" | "%" | "<<" | ">>")))
        }
        K::TupleExpr | K::ArrayExpr => {
            let mut arith = false;
            for k in kids {
                arith |= pure(tree, k, local, shadowed)?.1;
            }
            Some((false, arith))
        }
        _ => None,
    }
}

/// Whether the use at `path` is evaluated whenever the statement `stmt` holding it is: not in a branch of an `if` or a
/// `match`, nor right of `&&` or `||`.
fn unconditional(tree: &SyntaxTree<'_>, path: NodeId, stmt: NodeId) -> bool {
    let mut child = path;
    for a in tree.ancestors(path) {
        let first_expr = tree.children(a).find(|&c| tree.kind(c).is_expr());
        let conditional = match tree.kind(a) {
            K::IfExpr | K::MatchExpr | K::MatchArm | K::MatchArmList => first_expr != Some(child),
            K::BinExpr => first_expr != Some(child) && matches!(op(tree, a).as_str(), "&&" | "||"),
            _ => false,
        };
        // The statement itself may be the `if` whose branch holds the use.
        if conditional {
            return false;
        }
        if a == stmt {
            return true;
        }
        child = a;
    }
    true
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
pub(crate) fn inline_edits(doc: &str, offset: usize, let_type: LetType) -> Option<Vec<(std::ops::Range<usize>, String)>> {
    let tree = ridx_syntax::parse(doc);
    let (stmt, name, init) = plain_let(&tree, offset)?;
    let block = tree.parent(stmt)?;
    let f = tree.ancestors(stmt).find(|&a| tree.kind(a) == K::Fn)?;
    // Names compared as Rust compares them: `r#n` is `n`; a name beyond ASCII is not compared without its normal form.
    let idents: Vec<&str> =
        tree.tokens_of(init).iter().filter(|t| matches!(t.kind, T::Ident | T::RawIdent | T::SelfKw)).map(|t| ridx_syntax::sema_fn::norm_ident(t.text(doc))).collect();
    if !tree.text(name).is_ascii() || idents.iter().any(|i| !i.is_ascii()) {
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
    let receiver = matches!(tree.kind(parent), K::MethodCallExpr | K::FieldExpr | K::IndexExpr | K::CallExpr) && tree.children(parent).next() == Some(path);
    if tree.kind(parent) == K::RefExpr || receiver {
        return None;
    }
    for a in tree.ancestors(path).take_while(|&a| a != block) {
        let async_block = tree.kind(a) == K::BlockExpr && tree.own_tokens(a).any(|(_, t)| t.kind == T::Async);
        if async_block || matches!(tree.kind(a), K::ClosureExpr | K::LoopExpr | K::WhileExpr | K::ForExpr) {
            return None;
        }
    }
    let shadowed = primitive_shadowed(&tree);
    let local = |n: &str, at: usize| local_of(&tree, f, n, at, let_type).ok().flatten();
    let (_, arithmetic) = pure(&tree, init, &local, shadowed)?;
    let use_stmt = std::iter::once(path).chain(tree.ancestors(path)).find(|&a| tree.parent(a) == Some(block))?;
    if arithmetic {
        // Nothing the statement evaluates before the use may panic, exit or run code: the overflow still comes first.
        let next = tree.children(block).skip_while(|&c| c != stmt).nth(1);
        let before = tree.range(use_stmt).start..tree.range(path).start;
        let first = !tree.descendants(use_stmt).any(|d| {
            let k = tree.kind(d);
            let effect = matches!(k, K::CallExpr | K::MethodCallExpr | K::MacroCall | K::IndexExpr | K::TryExpr | K::AwaitExpr | K::PrefixExpr | K::BinExpr | K::CastExpr);
            effect && tree.range(d).end <= before.end && before.contains(&tree.range(d).start)
        });
        if next != Some(use_stmt) || !first || !unconditional(&tree, path, use_stmt) {
            return None;
        }
    }
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
pub fn inline_variable_action(doc: &str, uri: &str, range: &Value, doc_version: Option<i32>, let_type: LetType) -> Option<Value> {
    if range["start"] != range["end"] {
        return None;
    }
    let starts = line_starts_of(doc);
    let offset = utf16_pos_to_byte(doc, &starts, range["start"]["line"].as_u64()? as u32, range["start"]["character"].as_u64()? as u32)?;
    let edits: Vec<Value> = inline_edits(doc, offset, let_type)?
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
