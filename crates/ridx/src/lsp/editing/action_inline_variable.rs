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
//! its initializer, and the `let` goes (prod-code, m=20226). Offered only where the result means what the code meant
//! and compiles, which the syntax tree tells (docs/integration.md §6): rust-analyzer also inlines a binding used twice
//! (a value moved twice), into a loop or a closure (evaluated another number of times), or past a statement that
//! changes what the initializer reads; ridx does not offer those.

use super::action_parse::{byte_to_utf16_pos, line_starts_of, utf16_pos_to_byte};
use ridx_syntax::{NodeId, SyntaxKind as K, SyntaxTree, TokenKind as T};
use serde_json::{json, Value};

/// Initializers that stay one expression wherever they go, with no parentheses around them.
const ATOMIC: &[K] = &[
    K::Literal,
    K::PathExpr,
    K::CallExpr,
    K::MethodCallExpr,
    K::FieldExpr,
    K::IndexExpr,
    K::ParenExpr,
    K::TupleExpr,
    K::ArrayExpr,
    K::MacroCall,
    K::RecordExpr,
    K::TryExpr,
    K::AwaitExpr,
];

/// Where a use takes any expression as it is.
const OPEN: &[K] = &[K::ArgList, K::LetStmt, K::BlockExpr, K::ExprStmt, K::ReturnExpr, K::ParenExpr, K::ArrayExpr, K::TupleExpr, K::RecordExprField];

/// The `let` statement whose binding's name covers byte `offset` of `tree`, with the name's node: a plain binding (no
/// `mut`, no `ref`, no sub-pattern), no type written (the initializer's type would be inferred again where it goes) and
/// no `else`.
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

/// The identifiers `node` reads (its path segments' and its `self`), to check that nothing between the `let` and the
/// use names them.
fn idents<'s>(tree: &SyntaxTree<'s>, node: NodeId) -> Vec<&'s str> {
    tree.tokens_of(node).iter().filter(|t| matches!(t.kind, T::Ident | T::RawIdent | T::SelfKw)).map(|t| t.text(tree.src)).collect()
}

/// The edit of `Inline variable` at byte `offset` of `doc`: the deletion of the `let` and the initializer at the use,
/// as byte ranges with their new text, in order.
pub(crate) fn inline_edits(doc: &str, offset: usize) -> Option<Vec<(std::ops::Range<usize>, String)>> {
    let tree = ridx_syntax::parse(doc);
    let (stmt, name, init) = plain_let(&tree, offset)?;
    let block = tree.parent(stmt)?;
    let uses = ridx_syntax::sema_fn::let_uses(&tree, stmt, ridx_syntax::sema_fn::norm_ident(tree.text(name)));
    let [use_range] = uses.uses.as_slice() else { return None };
    if !uses.macro_uses.is_empty() {
        return None;
    }
    let path =
        tree.ancestors(tree.covering(use_range.start)).chain(std::iter::once(tree.covering(use_range.start))).find(|&n| tree.kind(n) == K::PathExpr && tree.range(n) == *use_range);
    let path = path.or_else(|| tree.ancestors(tree.covering(use_range.start)).find(|&n| tree.kind(n) == K::PathExpr))?;
    // Evaluated once, where the `let` was: not in a loop or a closure the `let` is outside of.
    for a in tree.ancestors(path).take_while(|&a| a != block) {
        if matches!(tree.kind(a), K::ClosureExpr | K::LoopExpr | K::WhileExpr | K::ForExpr) {
            return None;
        }
    }
    let parent = tree.parent(path)?;
    // A field shorthand (`S { x }`) names the field too.
    if tree.kind(parent) == K::RecordExprField && !tree.own_tokens(parent).any(|(_, t)| t.kind == T::Colon) {
        return None;
    }
    // Nothing between the `let` and the use names what the initializer reads: it reads the same values there.
    let read = idents(&tree, init);
    let between = tree.range(stmt).end..tree.range(path).start;
    if tree.tokens.iter().filter(|t| between.contains(&(t.start as usize))).any(|t| matches!(t.kind, T::Ident | T::RawIdent | T::SelfKw) && read.contains(&t.text(doc))) {
        return None;
    }
    let text = tree.text(init);
    let wrap = !ATOMIC.contains(&tree.kind(init)) && !OPEN.contains(&tree.kind(parent));
    let new_text = if wrap { format!("({text})") } else { text.to_string() };
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
